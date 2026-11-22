//! HTTP boundary for the AO3 upstream.
//!
//! One thin layer with five responsibilities: build the URL, attach the cookie
//! jar, never follow redirects (the session layer reads `Location`), apply the
//! client's own request spacing, and reduce the response to what the state
//! machine needs. No retries here — retry policy is per-action
//! (`docs/ao3-write-api.md` §7), and a retry hidden in the transport is how
//! double-posts happen.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, LOCATION, RETRY_AFTER, SET_COOKIE};
use reqwest::{Method, RequestBuilder};

use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::consts;
use crate::crypto::SecretString;
use crate::diag::{HttpDirection, HttpErrorEvent, Transport, sanitize_path};
use crate::error::{Error, Result};

const ACCEPT_VALUE: &str = "text/html,application/json;q=0.9,*/*;q=0.8";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// A response reduced to the parts the session layer classifies on.
///
/// `Debug` is hand-written: the body can carry a CSRF meta token and
/// `set_cookies` carry the bearer session cookie, so neither is ever formatted.
pub struct Response {
    pub status: u16,
    /// `Location` header, verbatim (relative or absolute).
    pub location: Option<String>,
    /// `Retry-After` in seconds, when the server supplied one.
    pub retry_after: Option<u64>,
    pub body: String,
    pub(crate) set_cookies: Vec<String>,
    /// Whether the request rotated/changed the cookie jar (then the CSRF token
    /// must be dropped too).
    pub jar_changed: bool,
    /// Safe diagnostic metadata: never the body or the cookies.
    pub method: String,
    pub path: String,
    pub latency_ms: u64,
    pub server_header: Option<String>,
    pub cf_ray: Option<String>,
    pub content_type: Option<String>,
    /// Which transport carried the request (direct client or solver).
    pub transport: Transport,
}

impl Response {
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    /// Does the redirect target contain `needle`? Used for `/users/login`,
    /// `/lost_cookie` and `/auth_error` detection.
    pub fn location_contains(&self, needle: &str) -> bool {
        self.location
            .as_deref()
            .is_some_and(|location| location.contains(needle))
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("location", &self.location)
            .field("retry_after", &self.retry_after)
            .field("body_len", &self.body.len())
            .field("set_cookie_count", &self.set_cookies.len())
            .field("jar_changed", &self.jar_changed)
            .field("method", &self.method)
            .field("path", &self.path)
            .field("latency_ms", &self.latency_ms)
            .field("server_header", &self.server_header)
            .field("cf_ray", &self.cf_ray)
            .field("content_type", &self.content_type)
            .field("transport", &self.transport)
            .finish()
    }
}

/// A Cloudflare clearance captured by a successful solver solve.
///
/// Cloudflare binds the clearance cookie to the browser's User-Agent, so the
/// override and the cookie travel together and expire together. Only a scoped
/// window uses them; the honest UA is restored the moment the window closes.
pub struct Clearance {
    pub user_agent: String,
    pub cookie_header: SecretString,
    pub expires_at: i64,
}

/// Receives every upstream HTTP failure. The server drains these into SQLite
/// and stderr; `ao3::client` stays decoupled from `Store`.
pub type ErrorSink = Arc<dyn Fn(&HttpErrorEvent) + Send + Sync>;

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    jar: Jar,
    spacing: Duration,
    last_request: Option<Instant>,
    sink: Option<ErrorSink>,
    /// Honest identity for direct traffic (ADR-0002 §5).
    user_agent: String,
    clearance: Option<Clearance>,
}

impl Client {
    /// Constructor with the client's own request spacing (`ZERO` in tests).
    pub fn with_spacing(base_url: &str, user_agent: &str, spacing: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(consts::CONNECT_TIMEOUT)
            .timeout(consts::REQUEST_TIMEOUT)
            // Redirects are data: a 302 to /users/login means "logged out", and
            // following it silently would hide that.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Network(format!("build http client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            jar: Jar::new(),
            spacing,
            last_request: None,
            sink: None,
            user_agent: user_agent.to_string(),
            clearance: None,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn jar(&self) -> &Jar {
        &self.jar
    }

    pub fn set_jar(&mut self, jar: Jar) {
        self.jar = jar;
    }

    pub fn jar_mut(&mut self) -> &mut Jar {
        &mut self.jar
    }

    /// Installs the diagnostic sink. Called once by the server; tests use it to
    /// assert exactly which events a request produced.
    pub fn set_error_sink(&mut self, sink: ErrorSink) {
        self.sink = Some(sink);
    }

    /// The identity sent upstream: the honest UA, unless a fresh clearance
    /// window is active (then the browser UA the cookie is bound to).
    fn effective_user_agent(&self) -> &str {
        match &self.clearance {
            Some(clearance) => &clearance.user_agent,
            None => &self.user_agent,
        }
    }

    pub fn clearance(&self) -> Option<&Clearance> {
        self.clearance.as_ref()
    }

    pub fn set_clearance(&mut self, user_agent: &str, cookie_header: &str, expires_at: i64) {
        self.clearance = Some(Clearance {
            user_agent: user_agent.to_string(),
            cookie_header: SecretString::new(cookie_header),
            expires_at,
        });
    }

    pub fn clear_clearance(&mut self) {
        self.clearance = None;
    }

    /// Drops the override once its window closed; called before every request.
    fn prune_expired_clearance(&mut self, now: i64) {
        if self
            .clearance
            .as_ref()
            .is_some_and(|clearance| clearance.expires_at <= now)
        {
            self.clearance = None;
        }
    }

    fn report(&self, event: HttpErrorEvent) {
        if let Some(sink) = &self.sink {
            sink(&event);
        }
    }

    pub async fn get(&mut self, path: &str) -> Result<Response> {
        let request = self.request(Method::GET, path);
        self.send(request, Method::GET, path).await
    }

    /// `GET` with a tighter per-request timeout (CSRF token fetch).
    pub async fn get_with_timeout(&mut self, path: &str, timeout: Duration) -> Result<Response> {
        let request = self.request(Method::GET, path).timeout(timeout);
        self.send(request, Method::GET, path).await
    }

    pub async fn post_form(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let body = form_urlencode(fields);
        let request = self
            .request(Method::POST, path)
            .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
            .body(body);
        self.send(request, Method::POST, path).await
    }

    pub async fn patch_form(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let body = form_urlencode(fields);
        let request = self
            .request(Method::PATCH, path)
            .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
            .body(body);
        self.send(request, Method::PATCH, path).await
    }

    pub async fn delete(&mut self, path: &str) -> Result<Response> {
        let request = self.request(Method::DELETE, path);
        self.send(request, Method::DELETE, path).await
    }

    /// `DELETE` carrying the CSRF token both as a form field and as
    /// `X-CSRF-Token` — Rails accepts either, and logout otherwise lands on
    /// `/auth_error` while looking like a success.
    pub async fn delete_with_csrf(&mut self, path: &str, token: &str) -> Result<Response> {
        let body = form_urlencode(&[("authenticity_token", token)]);
        let request = self
            .request(Method::DELETE, path)
            .header("x-csrf-token", token)
            .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
            .body(body);
        self.send(request, Method::DELETE, path).await
    }

    fn request(&mut self, method: Method, path: &str) -> RequestBuilder {
        self.prune_expired_clearance(crate::time::now_secs());
        let url = format!("{}{}", self.base_url, path);
        let mut request = self
            .http
            .request(method, url)
            .header(ACCEPT, ACCEPT_VALUE)
            .header(reqwest::header::USER_AGENT, self.effective_user_agent());
        let cookie = match (&self.clearance, self.jar.cookie_header()) {
            (Some(clearance), Some(jar)) => {
                Some(format!("{jar}; {}", clearance.cookie_header.expose()))
            }
            (Some(clearance), None) => Some(clearance.cookie_header.expose().to_string()),
            (None, jar) => jar,
        };
        if let Some(cookie) = cookie {
            request = request.header(COOKIE, cookie);
        }
        request
    }

    async fn send(
        &mut self,
        request: RequestBuilder,
        method: Method,
        path: &str,
    ) -> Result<Response> {
        let started = Instant::now();
        self.throttle().await;
        self.last_request = Some(Instant::now());

        let mut response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                self.report(
                    HttpErrorEvent::new(
                        HttpDirection::Upstream,
                        Transport::Direct,
                        method.as_str(),
                        path,
                        "network",
                        format!("request failed ({}): {error}", transport_detail(&error)),
                    )
                    .with_latency(elapsed_ms(started)),
                );
                return Err(Error::Network(format!("request failed: {error}")));
            }
        };

        let status = response.status().as_u16();
        let location = header_string(&response, LOCATION.as_str());
        let retry_after = header_string(&response, RETRY_AFTER.as_str())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let server_header = header_string(&response, "server");
        let cf_ray = header_string(&response, "cf-ray");
        let content_type = header_string(&response, "content-type");
        let set_cookies: Vec<String> = response
            .headers()
            .get_all(SET_COOKIE.as_str())
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_string)
            .collect();

        // Reject early when the server announces an oversized body; the streaming
        // cap below covers chunked responses that do not announce one.
        if let Some(length) = header_string(&response, "content-length")
            .and_then(|value| value.trim().parse::<usize>().ok())
            && length > consts::MAX_BODY_BYTES
        {
            self.report(
                HttpErrorEvent::new(
                    HttpDirection::Upstream,
                    Transport::Direct,
                    method.as_str(),
                    path,
                    "body_cap_exceeded",
                    format!(
                        "announced body {length} exceeds {} cap",
                        consts::MAX_BODY_BYTES
                    ),
                )
                .with_status(status)
                .with_latency(elapsed_ms(started))
                .with_headers(server_header.clone(), cf_ray.clone(), content_type.clone())
                .with_location(location.as_deref()),
            );
            return Err(Error::UpstreamChanged(format!(
                "announced body {length} exceeds {} cap",
                consts::MAX_BODY_BYTES
            )));
        }

        let before = self.jar.cookie_header();
        self.jar.apply_all(set_cookies.iter().map(String::as_str));
        let jar_changed = before != self.jar.cookie_header();

        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if bytes.len() + chunk.len() > consts::MAX_BODY_BYTES {
                        self.report(
                            HttpErrorEvent::new(
                                HttpDirection::Upstream,
                                Transport::Direct,
                                method.as_str(),
                                path,
                                "body_cap_exceeded",
                                format!(
                                    "body exceeds {} cap while streaming",
                                    consts::MAX_BODY_BYTES
                                ),
                            )
                            .with_status(status)
                            .with_latency(elapsed_ms(started))
                            .with_headers(
                                server_header.clone(),
                                cf_ray.clone(),
                                content_type.clone(),
                            ),
                        );
                        return Err(Error::UpstreamChanged(format!(
                            "body exceeds {} cap while streaming",
                            consts::MAX_BODY_BYTES
                        )));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => {
                    self.report(
                        HttpErrorEvent::new(
                            HttpDirection::Upstream,
                            Transport::Direct,
                            method.as_str(),
                            path,
                            "network",
                            format!("read body ({}): {error}", transport_detail(&error)),
                        )
                        .with_status(status)
                        .with_latency(elapsed_ms(started))
                        .with_headers(
                            server_header.clone(),
                            cf_ray.clone(),
                            content_type.clone(),
                        ),
                    );
                    return Err(Error::Network(format!("read body: {error}")));
                }
            }
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        let latency_ms = elapsed_ms(started);

        // A block while using a clearance means Cloudflare no longer trusts it;
        // fall back to the honest identity rather than retrying with a stale
        // browser UA on every later request.
        if status == 403 {
            self.clear_clearance();
        }

        if status >= 400 {
            let mut event = HttpErrorEvent::new(
                HttpDirection::Upstream,
                Transport::Direct,
                method.as_str(),
                path,
                classify_status(status),
                format!("upstream answered HTTP {status}"),
            )
            .with_status(status)
            .with_latency(latency_ms)
            .with_headers(server_header.clone(), cf_ray.clone(), content_type.clone())
            .with_location(location.as_deref());
            event.retry_after = retry_after;
            self.report(event);
        }

        if let Some(kind) = redirect_kind(location.as_deref()) {
            self.report(
                HttpErrorEvent::new(
                    HttpDirection::Upstream,
                    Transport::Direct,
                    method.as_str(),
                    path,
                    kind,
                    format!("upstream redirected HTTP {status}"),
                )
                .with_status(status)
                .with_latency(latency_ms)
                .with_headers(server_header.clone(), cf_ray.clone(), content_type.clone())
                .with_location(location.as_deref()),
            );
        }

        Ok(Response {
            status,
            location,
            retry_after,
            body,
            set_cookies,
            jar_changed,
            method: method.as_str().to_string(),
            path: sanitize_path(path),
            latency_ms,
            server_header,
            cf_ray,
            content_type,
            transport: Transport::Direct,
        })
    }

    /// Client-side floor between upstream requests (ADR-0002 §4). The server's
    /// own limit is 300/300 s per IP; we stay far below it by design.
    async fn throttle(&self) {
        if self.spacing.is_zero() {
            return;
        }
        if let Some(last) = self.last_request {
            let elapsed = last.elapsed();
            if elapsed < self.spacing {
                tokio::time::sleep(self.spacing - elapsed).await;
            }
        }
    }
}

fn header_string(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The part of a `reqwest` failure worth putting in a redacted message.
fn transport_detail(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else if error.is_request() {
        "request"
    } else {
        "transport"
    }
}

/// Maps an upstream status onto the diagnostic classifier (`docs/failure-matrix.md`).
fn classify_status(status: u16) -> &'static str {
    match status {
        429 => "rate_limited",
        403 | 503 => "blocked_by_edge",
        404 => "not_found",
        422 => "validation",
        500..=599 => "upstream_server_error",
        _ => "upstream_client_error",
    }
}

/// Auth-failure redirects the session state machine cares about.
fn redirect_kind(location: Option<&str>) -> Option<&'static str> {
    let location = location?;
    if location.contains("/users/login") {
        Some("redirect_login")
    } else if location.contains("/lost_cookie") {
        Some("redirect_lost_cookie")
    } else if location.contains("/auth_error") {
        Some("redirect_auth_error")
    } else {
        None
    }
}

/// `application/x-www-form-urlencoded` body from ordered pairs.
pub fn form_urlencode(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encode(key, SpaceMode::Plus),
                percent_encode(value, SpaceMode::Plus)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_matches_the_spec() {
        assert_eq!(
            form_urlencode(&[("user[login]", "a b"), ("x", "1+2")]),
            "user%5Blogin%5D=a+b&x=1%2B2"
        );
    }

    #[test]
    fn response_debug_hides_body_and_cookies() {
        let response = Response {
            status: 200,
            location: None,
            retry_after: None,
            body: "<meta name=\"csrf-token\" content=\"secret-token\">".into(),
            set_cookies: vec!["_otwarchive_session=secret-cookie".into()],
            jar_changed: true,
            method: "GET".into(),
            path: "/works/1".into(),
            latency_ms: 5,
            server_header: Some("cloudflare".into()),
            cf_ray: Some("abc".into()),
            content_type: Some("text/html".into()),
            transport: Transport::Direct,
        };
        let text = format!("{response:?}");
        assert!(!text.contains("secret-token"), "{text}");
        assert!(!text.contains("secret-cookie"), "{text}");
        assert!(text.contains("body_len"));
    }

    #[test]
    fn status_classification_matches_the_failure_matrix() {
        assert_eq!(classify_status(429), "rate_limited");
        assert_eq!(classify_status(403), "blocked_by_edge");
        assert_eq!(classify_status(503), "blocked_by_edge");
        assert_eq!(classify_status(404), "not_found");
        assert_eq!(classify_status(422), "validation");
        assert_eq!(classify_status(500), "upstream_server_error");
        assert_eq!(classify_status(418), "upstream_client_error");
    }

    #[test]
    fn only_auth_failure_redirects_are_classified() {
        assert_eq!(
            redirect_kind(Some("/users/login?return_to=/works/1")),
            Some("redirect_login")
        );
        assert_eq!(
            redirect_kind(Some("/lost_cookie")),
            Some("redirect_lost_cookie")
        );
        assert_eq!(
            redirect_kind(Some("/auth_error")),
            Some("redirect_auth_error")
        );
        assert_eq!(redirect_kind(Some("/works/123")), None);
        assert_eq!(redirect_kind(None), None);
    }

    #[test]
    fn response_classification() {
        let redirect = Response {
            status: 302,
            location: Some("/users/login?return_to=/x".into()),
            retry_after: None,
            body: String::new(),
            set_cookies: Vec::new(),
            jar_changed: false,
            method: "GET".into(),
            path: "/works/2".into(),
            latency_ms: 1,
            server_header: None,
            cf_ray: None,
            content_type: None,
            transport: Transport::Direct,
        };
        assert!(redirect.is_redirect());
        assert!(redirect.location_contains("/users/login"));
        assert!(!redirect.location_contains("/lost_cookie"));
    }
}
