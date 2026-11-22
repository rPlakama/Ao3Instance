//! Typed errors for the client core.
//!
//! The taxonomy is `docs/ao3-write-api.md` §6 minus the outcomes that only exist
//! once write actions land (W3+). Callers branch on the variant, never on
//! strings, and no variant may carry a cookie, password or CSRF token.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Configuration is missing or malformed (paths, env values).
    Config(String),
    /// Filesystem failure reading/writing the key or secret files.
    Io(String),
    /// Secret storage failure: bad hex, wrong length, authentication failed.
    Crypto(String),
    /// Transport failure: DNS, connect, timeout, TLS, body read.
    Network(String),
    /// Login form was accepted by the transport but rejected by the app.
    LoginRejected(String),
    /// AO3 signed the user in but wants a password change before use.
    PasswordChangeRequired,
    /// Session is dead: a read redirected to the login page.
    LoggedOut,
    /// Cookie jar incomplete: the app redirected to `/lost_cookie`.
    SessionIncomplete,
    /// CSRF token missing or rejected upstream.
    CsrfMissing,
    /// CSRF token was rejected (`/auth_error`) and must be refetched.
    CsrfExpired,
    /// Logout did not reach a signed-out state upstream.
    LogoutFailed(String),
    /// An operator switched all upstream traffic off (ADR-0002 §6).
    Killed,
    /// Upstream asked us to slow down (429, or the comment limiter's 302).
    RateLimited { retry_after: Option<u64> },
    /// Upstream answered with a bot challenge or refusal (Cloudflare / 403).
    BlockedByEdge { status: u16 },
    /// A page did not have the shape the parser expects.
    UpstreamChanged(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "configuration error: {msg}"),
            Self::Io(msg) => write!(f, "io error: {msg}"),
            Self::Crypto(msg) => write!(f, "secret store error: {msg}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::LoginRejected(msg) => write!(f, "login rejected: {msg}"),
            Self::PasswordChangeRequired => {
                write!(
                    f,
                    "AO3 requires a password change before this session can be used"
                )
            }
            Self::LoggedOut => write!(f, "session is logged out"),
            Self::SessionIncomplete => {
                write!(
                    f,
                    "session cookie jar is incomplete (missing user_credentials)"
                )
            }
            Self::CsrfMissing => write!(f, "csrf token missing from upstream page"),
            Self::CsrfExpired => write!(f, "csrf token was rejected; refetch required"),
            Self::LogoutFailed(msg) => write!(f, "logout failed upstream: {msg}"),
            Self::Killed => write!(f, "session is killed: upstream traffic is disabled"),
            Self::RateLimited { retry_after } => match retry_after {
                Some(secs) => write!(f, "rate limited; retry after {secs}s"),
                None => write!(f, "rate limited"),
            },
            Self::BlockedByEdge { status } => {
                write!(f, "upstream blocked the request (HTTP {status})")
            }
            Self::UpstreamChanged(msg) => write!(f, "upstream page changed: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
