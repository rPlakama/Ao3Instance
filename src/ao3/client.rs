//! HTTP boundary for the AO3 upstream.
//!
//! One thin layer with five responsibilities: build the URL, attach the cookie
//! jar, never follow redirects (the session layer reads `Location`), apply the
//! client's own request spacing, and reduce the response to what the state
//! machine needs. No retries here — retry policy is per-action
//! (`docs/ao3-write-api.md` §7), and a retry hidden in the transport is how
//! double-posts happen.

use std::fmt;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, LOCATION, RETRY_AFTER, SET_COOKIE};
use reqwest::{Method, RequestBuilder};

use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::consts;
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
            .finish()
    }
}

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    jar: Jar,
    spacing: Duration,
    last_request: Option<Instant>,
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
            .user_agent(user_agent)
            .build()
            .map_err(|e| Error::Network(format!("build http client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            jar: Jar::new(),
            spacing,
            last_request: None,
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

    pub async fn get(&mut self, path: &str) -> Result<Response> {
        let request = self.request(Method::GET, path);
        self.send(request).await
    }

    /// `GET` with a tighter per-request timeout (CSRF token fetch).
    pub async fn get_with_timeout(&mut self, path: &str, timeout: Duration) -> Result<Response> {
        let request = self.request(Method::GET, path).timeout(timeout);
        self.send(request).await
    }

    pub async fn post_form(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let body = form_urlencode(fields);
        let request = self
            .request(Method::POST, path)
            .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
            .body(body);
        self.send(request).await
    }

    pub async fn patch_form(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let body = form_urlencode(fields);
        let request = self
            .request(Method::PATCH, path)
            .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
            .body(body);
        self.send(request).await
    }

    pub async fn delete(&mut self, path: &str) -> Result<Response> {
        let request = self.request(Method::DELETE, path);
        self.send(request).await
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
        self.send(request).await
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.http.request(method, url).header(ACCEPT, ACCEPT_VALUE);
        if let Some(cookie) = self.jar.cookie_header() {
            request = request.header(COOKIE, cookie);
        }
        request
    }

    async fn send(&mut self, request: RequestBuilder) -> Result<Response> {
        self.throttle().await;
        self.last_request = Some(Instant::now());

        let mut response = request
            .send()
            .await
            .map_err(|e| Error::Network(format!("request failed: {e}")))?;

        let status = response.status().as_u16();
        let location = header_string(&response, LOCATION.as_str());
        let retry_after = header_string(&response, RETRY_AFTER.as_str())
            .and_then(|value| value.trim().parse::<u64>().ok());
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
            return Err(Error::UpstreamChanged(format!(
                "announced body {length} exceeds {} cap",
                consts::MAX_BODY_BYTES
            )));
        }

        let before = self.jar.cookie_header();
        self.jar.apply_all(set_cookies.iter().map(String::as_str));
        let jar_changed = before != self.jar.cookie_header();

        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Error::Network(format!("read body: {e}")))?
        {
            if bytes.len() + chunk.len() > consts::MAX_BODY_BYTES {
                return Err(Error::UpstreamChanged(format!(
                    "body exceeds {} cap while streaming",
                    consts::MAX_BODY_BYTES
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();

        Ok(Response {
            status,
            location,
            retry_after,
            body,
            set_cookies,
            jar_changed,
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
        };
        let text = format!("{response:?}");
        assert!(!text.contains("secret-token"), "{text}");
        assert!(!text.contains("secret-cookie"), "{text}");
        assert!(text.contains("body_len"));
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
        };
        assert!(redirect.is_redirect());
        assert!(redirect.location_contains("/users/login"));
        assert!(!redirect.location_contains("/lost_cookie"));
    }
}
