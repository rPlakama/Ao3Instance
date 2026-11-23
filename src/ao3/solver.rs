//! HTTP client for the host-provided FlareSolverr service (Phase 2.5).
//!
//! We never spawn, vendor or depend on the service at build time: the contract
//! is one loopback `/v1` endpoint that the operator's system already runs. This
//! module only turns that API into typed values; the cookie jar, parser and
//! sanitizer decide what happens to the HTML afterwards.
//!
//! Secrets: cookies returned by the browser (`cf_clearance`, `__cf_bm`, the AO3
//! session) are wrapped in [`SecretString`], so `Debug` and `Display` print
//! `[redacted]` and a careless log line cannot leak them.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::config::{SolverConfig, SolverMode};
use crate::consts;
use crate::crypto::SecretString;
use crate::error::{Error, Result};
use crate::time::now_secs;

/// FlareSolverr escapes the HTML inside JSON, so the wire body can be several
/// times `MAX_BODY_BYTES`; this bound still stops a hostile local service from
/// exhausting memory.
const MAX_SOLVER_WIRE_BYTES: usize = consts::MAX_BODY_BYTES * 4;

/// Named browser session reused across requests when the FlareSolverr server
/// advertises session support, keeping cookies and CSRF tokens coherent between
/// GET and POST requests.
const FLARESOLVERR_SESSION_ID: &str = "ao3instance";

/// Result of a liveness probe. Safe to render and log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverStatus {
    pub mode: SolverMode,
    pub reachable: bool,
    pub version: Option<String>,
    pub checked_at: i64,
}

/// One cookie returned by the browser. The value is a secret; only the name and
/// domain may be printed.
pub struct SolverCookie {
    pub name: String,
    pub value: SecretString,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub expires: Option<f64>,
    pub http_only: bool,
    pub secure: bool,
}

impl fmt::Debug for SolverCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SolverCookie")
            .field("name", &self.name)
            .field("value", &"[redacted]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("expires", &self.expires)
            .field("http_only", &self.http_only)
            .field("secure", &self.secure)
            .finish()
    }
}

/// What a successful solve produced.
pub struct SolveOutcome {
    pub status: u16,
    pub html: String,
    pub cookies: Vec<SolverCookie>,
    pub user_agent: String,
    /// Final URL after the browser followed redirects (`solution.url`).
    pub solution_url: String,
    pub latency_ms: u64,
}

impl fmt::Debug for SolveOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SolveOutcome")
            .field("status", &self.status)
            .field("html_len", &self.html.len())
            .field("cookie_count", &self.cookies.len())
            .field("user_agent", &self.user_agent)
            .field("solution_url", &self.solution_url)
            .field("latency_ms", &self.latency_ms)
            .finish()
    }
}

/// The solver endpoint plus the local policy knobs that go with it.
pub struct Solver {
    http: reqwest::Client,
    endpoint: String,
    probe_timeout: Duration,
    max_timeout_ms: u64,
    mode: SolverMode,
    /// True once `sessions.list` confirms the server supports named sessions.
    sessions_supported: AtomicBool,
    /// True while [`FLARESOLVERR_SESSION_ID`] is believed live on the solver.
    session_ready: AtomicBool,
}

impl Solver {
    pub fn new(config: &SolverConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(consts::FLARESOLVERR_SOLVE_TIMEOUT)
            // The service is local; a proxy inherited from the environment would
            // only add a way to leak the request somewhere unexpected.
            .no_proxy()
            .build()
            .map_err(|e| Error::Network(format!("build solver client: {e}")))?;
        Ok(Self {
            http,
            endpoint: format!("{}/v1", config.origin.trim_end_matches('/')),
            probe_timeout: consts::FLARESOLVERR_PROBE_TIMEOUT,
            max_timeout_ms: config.max_timeout_ms,
            mode: config.mode,
            sessions_supported: AtomicBool::new(false),
            session_ready: AtomicBool::new(false),
        })
    }

    pub fn mode(&self) -> SolverMode {
        self.mode
    }

    /// Liveness + version probe. Never fatal: an absent service is a status,
    /// not an error, and this never touches AO3.
    pub async fn probe(&self) -> SolverStatus {
        let mut status = SolverStatus {
            mode: self.mode,
            reachable: false,
            version: None,
            checked_at: now_secs(),
        };
        if self.mode == SolverMode::Off {
            return status;
        }

        let result = self
            .http
            .post(&self.endpoint)
            .timeout(self.probe_timeout)
            .json(&json!({"cmd": "sessions.list"}))
            .send()
            .await;
        let Ok(response) = result else {
            return status;
        };
        if !response.status().is_success() {
            return status;
        }
        let Ok(body) = response.text().await else {
            return status;
        };
        let Ok(api) = serde_json::from_str::<ApiResponse>(&body) else {
            return status;
        };
        // FlareSolverr does not guarantee a `status` field on every command, so
        // only an explicit error counts as unreachable.
        if api.status.as_deref() == Some("error") {
            return status;
        }
        if let Some(sessions) = api.sessions {
            self.sessions_supported.store(true, Ordering::Relaxed);
            let exists = sessions.iter().any(|s| s == FLARESOLVERR_SESSION_ID);
            self.session_ready.store(exists, Ordering::Relaxed);
        }
        status.reachable = true;
        status.version = api.version;
        status.checked_at = now_secs();
        status
    }

    /// Destroys the named FlareSolverr browser session (if any) so the next
    /// request starts with a fresh cookie jar (used on logout / cookie import).
    pub async fn reset_session(&self) {
        if !self.sessions_supported.load(Ordering::Relaxed) {
            return;
        }
        if !self.session_ready.swap(false, Ordering::Relaxed) {
            return;
        }
        drop(
            self.http
                .post(&self.endpoint)
                .timeout(self.probe_timeout)
                .json(&json!({
                    "cmd": "sessions.destroy",
                    "session": FLARESOLVERR_SESSION_ID,
                }))
                .send()
                .await,
        );
    }

    async fn ensure_session(&self) -> Option<&'static str> {
        if !self.sessions_supported.load(Ordering::Relaxed) {
            return None;
        }
        if self.session_ready.load(Ordering::Relaxed) {
            return Some(FLARESOLVERR_SESSION_ID);
        }
        let response = self
            .http
            .post(&self.endpoint)
            .timeout(consts::FLARESOLVERR_SOLVE_TIMEOUT)
            .json(&json!({
                "cmd": "sessions.create",
                "session": FLARESOLVERR_SESSION_ID,
            }))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body = response.text().await.ok()?;
        let api: ApiResponse = serde_json::from_str(&body).ok()?;
        if api.status.as_deref() == Some("error") {
            return None;
        }
        self.session_ready.store(true, Ordering::Relaxed);
        Some(FLARESOLVERR_SESSION_ID)
    }

    /// Solves one `GET` in the local browser and returns the rendered page.
    pub async fn solve_get(&self, url: &str, cookies: &[(String, String)]) -> Result<SolveOutcome> {
        if self.mode == SolverMode::Off {
            return Err(Error::SolverDisabled);
        }
        let mut payload = json!({
            "cmd": "request.get",
            "url": url,
            "maxTimeout": self.max_timeout_ms,
            "disableMedia": true,
            "returnScreenshot": false,
        });
        attach_cookies(&mut payload, cookies);
        self.execute_with_session(payload).await
    }

    /// Sends one form `POST` through the local browser and returns the landed page.
    pub async fn solve_post(
        &self,
        url: &str,
        post_data: &str,
        cookies: &[(String, String)],
    ) -> Result<SolveOutcome> {
        if self.mode == SolverMode::Off {
            return Err(Error::SolverDisabled);
        }
        let mut payload = json!({
            "cmd": "request.post",
            "url": url,
            "postData": post_data,
            "maxTimeout": self.max_timeout_ms,
            "disableMedia": true,
            "returnScreenshot": false,
        });
        attach_cookies(&mut payload, cookies);
        self.execute_with_session(payload).await
    }

    async fn execute_with_session(&self, mut payload: serde_json::Value) -> Result<SolveOutcome> {
        let had_session = if let Some(session_id) = self.ensure_session().await {
            payload["session"] = serde_json::Value::String(session_id.to_string());
            true
        } else {
            false
        };
        match self.execute(&payload).await {
            Ok(outcome) => Ok(outcome),
            Err(Error::SolverFailed(msg))
                if had_session && msg.to_ascii_lowercase().contains("session") =>
            {
                // The FlareSolverr daemon restarted or evicted the session;
                // recreate it once and retry.
                self.session_ready.store(false, Ordering::Relaxed);
                if let Some(session_id) = self.ensure_session().await {
                    payload["session"] = serde_json::Value::String(session_id.to_string());
                } else if let Some(map) = payload.as_object_mut() {
                    map.remove("session");
                }
                self.execute(&payload).await
            }
            Err(error) => Err(error),
        }
    }

    async fn execute(&self, payload: &serde_json::Value) -> Result<SolveOutcome> {
        let started = Instant::now();
        let mut response = self
            .http
            .post(&self.endpoint)
            .json(payload)
            .send()
            .await
            .map_err(|error| {
                Error::SolverUnavailable(format!("request failed: {}", transport_detail(&error)))
            })?;
        let http_status = response.status();
        if !http_status.is_success() {
            return Err(Error::SolverFailed(format!(
                "solver answered HTTP {}",
                http_status.as_u16()
            )));
        }

        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if bytes.len() + chunk.len() > MAX_SOLVER_WIRE_BYTES {
                        return Err(Error::SolverBodyCap);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => {
                    return Err(Error::SolverUnavailable(format!(
                        "read response: {}",
                        transport_detail(&error)
                    )));
                }
            }
        }

        let api: ApiResponse = serde_json::from_slice(&bytes)
            .map_err(|_| Error::SolverFailed("solver returned malformed JSON".into()))?;
        if api.status.as_deref() == Some("error") {
            let message = api.message.unwrap_or_default();
            if message.to_ascii_lowercase().contains("captcha") {
                return Err(Error::SolverCaptcha);
            }
            return Err(Error::SolverFailed(crate::diag::redact_message(&message)));
        }
        let solution = api
            .solution
            .ok_or_else(|| Error::SolverFailed("solver returned no solution".into()))?;
        let html = solution.response.unwrap_or_default();
        if html.len() > consts::MAX_BODY_BYTES {
            return Err(Error::SolverBodyCap);
        }

        let solution_url = solution.url.unwrap_or_default();
        if solution_url.starts_with("chrome-error://") {
            return Err(Error::SolverFailed(
                "browser could not reach upstream".into(),
            ));
        }

        Ok(SolveOutcome {
            status: solution.status.unwrap_or(200),
            html,
            cookies: solution
                .cookies
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
            user_agent: solution.user_agent.unwrap_or_default(),
            solution_url,
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }
}

/// The subset of FlareSolverr's JSON we rely on. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct ApiResponse {
    status: Option<String>,
    message: Option<String>,
    version: Option<String>,
    sessions: Option<Vec<String>>,
    solution: Option<Solution>,
}

#[derive(Debug, Deserialize)]
struct Solution {
    url: Option<String>,
    status: Option<u16>,
    response: Option<String>,
    #[serde(rename = "userAgent")]
    user_agent: Option<String>,
    cookies: Option<Vec<RawCookie>>,
}

#[derive(Debug, Deserialize)]
struct RawCookie {
    name: String,
    value: String,
    domain: Option<String>,
    path: Option<String>,
    expires: Option<f64>,
    #[serde(default, rename = "httpOnly")]
    http_only: bool,
    #[serde(default)]
    secure: bool,
}

impl From<RawCookie> for SolverCookie {
    fn from(raw: RawCookie) -> Self {
        Self {
            name: raw.name,
            value: SecretString::new(raw.value),
            domain: raw.domain,
            path: raw.path,
            expires: raw.expires,
            http_only: raw.http_only,
            secure: raw.secure,
        }
    }
}

fn attach_cookies(payload: &mut serde_json::Value, cookies: &[(String, String)]) {
    if !cookies.is_empty() {
        let wire: Vec<serde_json::Value> = cookies
            .iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        payload["cookies"] = serde_json::Value::Array(wire);
    }
}

/// Extracts raw JSON from either a direct JSON response or Chromium's
/// HTML-wrapped `<pre>{...}</pre>` document returned by FlareSolverr.
pub(crate) fn extract_json_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return trimmed.to_string();
    }
    let doc = scraper::Html::parse_document(body);
    let Ok(selector) = scraper::Selector::parse("pre") else {
        return trimmed.to_string();
    };
    doc.select(&selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

fn transport_detail(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() {
        "body"
    } else {
        "transport"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: SolverMode) -> SolverConfig {
        SolverConfig {
            mode,
            origin: "http://127.0.0.1:8191".into(),
            ..SolverConfig::default()
        }
    }

    #[test]
    fn cookies_and_outcomes_hide_secrets_in_debug() {
        let cookie = SolverCookie {
            name: "cf_clearance".into(),
            value: SecretString::new("secret-clearance"),
            domain: Some(".archiveofourown.org".into()),
            path: Some("/".into()),
            expires: Some(1.0),
            http_only: true,
            secure: true,
        };
        let text = format!("{cookie:?}");
        assert!(text.contains("cf_clearance"));
        assert!(!text.contains("secret-clearance"), "{text}");
        assert!(text.contains("[redacted]"), "{text}");

        let outcome = SolveOutcome {
            status: 200,
            html: "<html>secret-cookie</html>".into(),
            cookies: vec![cookie],
            user_agent: "Mozilla/5.0".into(),
            solution_url: "https://archiveofourown.org/works/1".into(),
            latency_ms: 42,
        };
        let text = format!("{outcome:?}");
        assert!(!text.contains("secret-cookie"), "{text}");
        assert!(text.contains("html_len"), "{text}");
    }

    #[test]
    fn off_mode_never_contacts_the_service() {
        let solver = Solver::new(&config(SolverMode::Off)).expect("solver");
        assert_eq!(solver.mode(), SolverMode::Off);
        let status = futures_lite_block(solver.probe());
        assert!(!status.reachable);
    }

    /// Minimal blocking shim so this unit test needs no tokio test runtime for
    /// an off-mode probe (which returns before any await).
    fn futures_lite_block<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let mut future = std::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("off-mode probe must not await"),
        }
    }
}
