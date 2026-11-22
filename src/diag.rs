//! Structured, redacted HTTP failure diagnostics.
//!
//! Every event is assembled from safe fields only: a sanitized path, a status,
//! a classifier and a redacted message. Cookies, CSRF tokens, passwords and
//! response bodies never enter this module, and `sanitize_path` /
//! `redact_message` are the last line of defence when a path or header arrives
//! from the network (`docs/security-standards.md` §5).
//!
//! The structured line is single-line and greppable so systemd/journald can
//! index it without a JSON stack.

use std::fmt;

use crate::time::{format_rfc3339, now_secs};

/// Longest path kept in a log row; enough for AO3's deepest work URLs.
const MAX_PATH_CHARS: usize = 200;
/// Longest redacted message kept in a log row.
const MAX_MESSAGE_CHARS: usize = 300;
/// Longest value kept for an allowlisted query parameter.
const MAX_QUERY_VALUE_CHARS: usize = 48;
/// Query keys that are safe and useful for support (`view_full_work=true`).
const SAFE_QUERY_KEYS: [&str; 3] = ["view_full_work", "chapter", "page"];
/// Names that always carry secrets; their values are replaced wholesale.
const SECRET_KEYS: [&str; 8] = [
    "_otwarchive_session",
    "user_credentials",
    "authenticity_token",
    "password",
    "cookie",
    "set-cookie",
    "cf_clearance",
    "__cf_bm",
];

/// Which side of the proxy produced the failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpDirection {
    Upstream,
    Inbound,
}

impl HttpDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upstream => "upstream",
            Self::Inbound => "inbound",
        }
    }
}

impl fmt::Display for HttpDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which transport carried the request. `Solver` is the host-provided
/// FlareSolverr fallback (Phase 2.5); `Direct` is our own client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Direct,
    Solver,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Solver => "solver",
        }
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One HTTP failure, safe to log, render and store.
///
/// The constructor already sanitizes `path` and redacts `message`, so a caller
/// cannot accidentally persist a token by passing a raw upstream string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpErrorEvent {
    pub direction: HttpDirection,
    pub transport: Transport,
    pub method: String,
    pub path: String,
    pub status: Option<u16>,
    pub error_kind: &'static str,
    pub message: String,
    pub latency_ms: Option<u64>,
    pub retry_after: Option<u64>,
    pub location_path: Option<String>,
    pub server_header: Option<String>,
    pub cf_ray: Option<String>,
    pub content_type: Option<String>,
    pub created_at: i64,
}

impl HttpErrorEvent {
    pub fn new(
        direction: HttpDirection,
        transport: Transport,
        method: impl Into<String>,
        path: impl Into<String>,
        error_kind: &'static str,
        message: impl Into<String>,
    ) -> Self {
        let path: String = path.into();
        let message: String = message.into();
        Self {
            direction,
            transport,
            method: method.into(),
            path: sanitize_path(&path),
            status: None,
            error_kind,
            message: redact_message(&message),
            latency_ms: None,
            retry_after: None,
            location_path: None,
            server_header: None,
            cf_ray: None,
            content_type: None,
            created_at: now_secs(),
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_ms = Some(latency_ms);
        self
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    /// Keeps only the path of a redirect target; query and fragment are dropped.
    pub fn with_location(mut self, location: Option<&str>) -> Self {
        self.location_path = sanitize_location(location);
        self
    }

    /// Diagnostic response headers. Values are scrubbed and truncated, never
    /// trusted verbatim.
    pub fn with_headers(
        mut self,
        server: Option<String>,
        cf_ray: Option<String>,
        content_type: Option<String>,
    ) -> Self {
        self.server_header = server.map(|value| safe_header(&value));
        self.cf_ray = cf_ray.map(|value| safe_header(&value));
        self.content_type = content_type.map(|value| safe_header(&value));
        self
    }
}

/// Formats one event as a single greppable line.
///
/// Path, location, headers and message are `Debug`-quoted, so a newline from a
/// hostile response cannot forge a second log line.
pub fn format_line(event: &HttpErrorEvent) -> String {
    let status = match event.status {
        Some(status) => status.to_string(),
        None => "none".to_string(),
    };
    let mut line = format!(
        "{} level=ERROR component=http direction={} transport={} method={} path={:?} status={status} kind={}",
        format_rfc3339(event.created_at),
        event.direction.as_str(),
        event.transport.as_str(),
        event.method,
        event.path,
        event.error_kind,
    );
    if let Some(latency_ms) = event.latency_ms {
        line.push_str(&format!(" latency_ms={latency_ms}"));
    }
    if let Some(retry_after) = event.retry_after {
        line.push_str(&format!(" retry_after={retry_after}"));
    }
    if let Some(location_path) = &event.location_path {
        line.push_str(&format!(" location_path={location_path:?}"));
    }
    if let Some(server) = &event.server_header {
        line.push_str(&format!(" server={server:?}"));
    }
    if let Some(cf_ray) = &event.cf_ray {
        line.push_str(&format!(" cf_ray={cf_ray:?}"));
    }
    if let Some(content_type) = &event.content_type {
        line.push_str(&format!(" content_type={content_type:?}"));
    }
    line.push_str(&format!(" msg={:?}", event.message));
    line
}

/// Emits the structured line on stderr (journald-friendly).
pub fn emit_stderr(event: &HttpErrorEvent) {
    eprintln!("{}", format_line(event));
}

/// Sanitizes a request path for storage or display.
///
/// Drops the origin, keeps only allowlisted query keys (`view_full_work`,
/// `chapter`, `page`), removes control characters, and truncates on a UTF-8
/// boundary. Empty paths become `/`.
pub fn sanitize_path(raw: &str) -> String {
    let without_origin = strip_origin(raw);
    let (path, query) = match without_origin.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (without_origin, None),
    };
    let path = clean_path(path, MAX_PATH_CHARS);
    let mut out = if path.is_empty() {
        "/".to_string()
    } else {
        path
    };
    let kept = keep_safe_query(query);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept);
    }
    out
}

/// Extracts only the path of a redirect target. Query and fragment never
/// survive, and an empty target becomes `None`.
pub fn sanitize_location(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let without_origin = strip_origin(raw);
    let path = without_origin.split(['?', '#']).next().unwrap_or_default();
    let cleaned = clean_path(path, MAX_PATH_CHARS);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Defensive scrubber for free-form diagnostic text.
///
/// Credential-looking `key=value` pairs are replaced with `[redacted]`,
/// control characters become spaces, whitespace is collapsed and the result is
/// truncated. Callers should still prefer writing a fixed message.
pub fn redact_message(message: &str) -> String {
    let mut text = String::with_capacity(message.len());
    for ch in message.chars() {
        if ch.is_control() {
            text.push(' ');
        } else {
            text.push(ch);
        }
    }
    for key in SECRET_KEYS {
        text = redact_key(&text, key);
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, MAX_MESSAGE_CHARS)
}

/// `scheme://authority/path` → `/path`; anything else is returned untouched.
fn strip_origin(raw: &str) -> &str {
    let Some(idx) = raw.find("://") else {
        return raw;
    };
    let scheme = &raw[..idx];
    let looks_like_scheme = !scheme.is_empty()
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'));
    if !looks_like_scheme {
        return raw;
    }
    let rest = &raw[idx + 3..];
    rest.find('/').map_or("/", |path| &rest[path..])
}

/// Removes control characters and truncates on a character boundary.
fn clean_path(raw: &str, max_chars: usize) -> String {
    let filtered: String = raw.chars().filter(|ch| !ch.is_control()).collect();
    truncate_chars(&filtered, max_chars)
}

fn keep_safe_query(query: Option<&str>) -> String {
    let Some(query) = query else {
        return String::new();
    };
    let mut kept = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if !SAFE_QUERY_KEYS.contains(&key) {
            continue;
        }
        kept.push(format!(
            "{}={}",
            clean_path(key, 32),
            clean_path(value, MAX_QUERY_VALUE_CHARS)
        ));
    }
    kept.join("&")
}

/// Replaces the value of `key=` with `[redacted]` until a delimiter.
fn redact_key(text: &str, key: &str) -> String {
    let delimiter = |ch: char| {
        matches!(
            ch,
            '&' | ';' | ',' | ' ' | '\t' | '\n' | '\r' | '"' | '\'' | '<' | '>'
        )
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(key) {
        out.push_str(&rest[..pos]);
        out.push_str(key);
        let after_key = rest[pos + key.len()..].trim_start_matches(['"', '\'', ' ']);
        if let Some(after_eq) = after_key.strip_prefix('=') {
            out.push('=');
            out.push_str("[redacted]");
            let value = after_eq.trim_start_matches(['"', '\'', ' ']);
            rest = &value[value.find(delimiter).unwrap_or(value.len())..];
        } else {
            rest = after_key;
        }
    }
    out.push_str(rest);
    out
}

fn safe_header(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(120)
        .collect();
    cleaned.trim().to_string()
}

/// Truncates to `max_chars` on a UTF-8 boundary, marking the cut with `…`.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_and_transports_have_stable_names() {
        assert_eq!(HttpDirection::Upstream.to_string(), "upstream");
        assert_eq!(HttpDirection::Inbound.to_string(), "inbound");
        assert_eq!(Transport::Direct.to_string(), "direct");
        assert_eq!(Transport::Solver.to_string(), "solver");
    }

    #[test]
    fn paths_drop_origin_queries_and_control_characters() {
        assert_eq!(
            sanitize_path(
                "https://archiveofourown.org/works/123?view_full_work=true&authenticity_token=abc#top"
            ),
            "/works/123?view_full_work=true"
        );
        assert_eq!(
            sanitize_path("/works/123?user_credentials=secret&chapter=2"),
            "/works/123?chapter=2"
        );
        assert_eq!(sanitize_path("?token=abc"), "/");
        assert_eq!(sanitize_path(""), "/");
        assert_eq!(sanitize_path("/x\r\nlevel=ERROR"), "/xlevel=ERROR");
        assert_eq!(
            sanitize_path("http://example.org"),
            "/",
            "origin-only URLs have no path"
        );
    }

    #[test]
    fn long_paths_truncate_on_a_utf8_boundary() {
        let raw = format!("/works/{}", "⌘".repeat(400));
        let path = sanitize_path(&raw);
        assert!(path.chars().count() <= MAX_PATH_CHARS + "?chapter=2".len());
        assert!(path.ends_with('…'));
    }

    #[test]
    fn locations_keep_only_the_path() {
        assert_eq!(
            sanitize_location(Some(
                "https://archiveofourown.org/users/login?return_to=/works/1#x"
            ))
            .as_deref(),
            Some("/users/login")
        );
        assert_eq!(
            sanitize_location(Some("/lost_cookie?x=1")).as_deref(),
            Some("/lost_cookie")
        );
        assert_eq!(sanitize_location(Some("?token=abc")), None);
        assert_eq!(sanitize_location(Some("   ")), None);
        assert_eq!(sanitize_location(None), None);
    }

    #[test]
    fn messages_redact_credentials_and_collapse_newlines() {
        let dirty = "GET /works/1 failed cookie: _otwarchive_session=abc123; \
                     user_credentials=def456 authenticity_token=ghi789 password=hunter2";
        let clean = redact_message(dirty);
        for secret in ["abc123", "def456", "ghi789", "hunter2"] {
            assert!(!clean.contains(secret), "leaked {secret}: {clean}");
        }
        assert!(clean.contains("[redacted]"), "{clean}");
        assert!(!clean.contains('\n'), "{clean}");

        let injected = redact_message("bad\r\nlevel=ERROR forged line");
        assert!(
            !injected.contains('\r') && !injected.contains('\n'),
            "{injected}"
        );
    }

    #[test]
    fn long_messages_are_truncated() {
        let message = redact_message(&"x".repeat(1_000));
        assert!(message.chars().count() <= MAX_MESSAGE_CHARS);
        assert!(message.ends_with('…'));
    }

    #[test]
    fn constructors_sanitize_their_inputs() {
        let event = HttpErrorEvent::new(
            HttpDirection::Upstream,
            Transport::Direct,
            "GET",
            "/works/1?authenticity_token=secret",
            "blocked_by_edge",
            "refused; cookie _otwarchive_session=secretcookie",
        )
        .with_status(403)
        .with_latency(12)
        .with_retry_after(30)
        .with_location(Some("/users/login?return_to=/works/1"))
        .with_headers(
            Some("cloudflare".into()),
            Some("8f2a".into()),
            Some("text/html".into()),
        );
        assert_eq!(event.path, "/works/1");
        assert!(
            !event.message.contains("secretcookie"),
            "{:?}",
            event.message
        );
        assert_eq!(event.location_path.as_deref(), Some("/users/login"));
    }

    #[test]
    fn log_lines_are_single_line_and_greppable() {
        let event = HttpErrorEvent::new(
            HttpDirection::Upstream,
            Transport::Direct,
            "GET",
            "/works/123",
            "blocked_by_edge",
            "AO3 refused the request\nforged line",
        )
        .with_status(403)
        .with_latency(120)
        .with_retry_after(30)
        .with_headers(
            Some("cloudflare".into()),
            Some("8f2a".into()),
            Some("text/html".into()),
        );
        let line = format_line(&event);
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains("level=ERROR"), "{line}");
        assert!(
            line.contains("direction=upstream transport=direct method=GET"),
            "{line}"
        );
        assert!(line.contains("path=\"/works/123\""), "{line}");
        assert!(line.contains("status=403"), "{line}");
        assert!(line.contains("kind=blocked_by_edge"), "{line}");
        assert!(line.contains("latency_ms=120"), "{line}");
        assert!(line.contains("retry_after=30"), "{line}");
        assert!(line.contains("server=\"cloudflare\""), "{line}");
        assert!(line.contains("cf_ray=\"8f2a\""), "{line}");
        assert!(
            line.contains("msg=\"AO3 refused the request forged line\""),
            "{line}"
        );
        assert!(!line.contains("forged line\n"), "{line}");
    }
}
