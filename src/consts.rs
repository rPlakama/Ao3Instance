//! Process-wide constants.
//!
//! One place for timeouts, paths, cookie names and the request identity, so a
//! policy change is a single edit. Inherited habit from MUSIKA's `consts.rs`.

use std::time::Duration;

/// Production upstream. Always overridable through `AO3I_BASE_URL` (sandbox).
pub const BASE_URL: &str = "https://archiveofourown.org";

/// Default request identity. No spoofed browsers (ADR-0002 §5).
///
/// ADR-0002 asks for a project URL and a contact address so an operator can be
/// asked to stop. The owner must supply those; set `AO3I_USER_AGENT` before any
/// public deployment. The default is honest about what it is, which is the
/// minimum this constant can guarantee on its own.
pub const USER_AGENT: &str = concat!(
    "AO3Instance/",
    env!("CARGO_PKG_VERSION"),
    " (offline AO3 reader; contact the instance operator)"
);

/// Session cookie. Signed by Rails; opaque to us (`docs/ao3-internals.md` §4.2).
pub const SESSION_COOKIE: &str = "_otwarchive_session";
/// Load-bearing companion cookie: missing it triggers `/lost_cookie`.
pub const CREDENTIALS_COOKIE: &str = "user_credentials";

/// Where the web app listens by default (loopback only; it holds a session).
pub const DEFAULT_BIND: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8099;
pub const DB_FILE_NAME: &str = "ao3i.db";

pub const LOGIN_PATH: &str = "/users/login";
pub const LOGOUT_PATH: &str = "/users/logout";
pub const TOKEN_PATH: &str = "/token_dispenser.json";

/// TCP connect budget.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-request budget for ordinary reads.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// CSRF token fetch is small and on the critical path.
pub const TOKEN_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard cap on a response body. A work page is far below this; anything past it
/// is an interstitial or an attack, not content.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// The client's own floor between upstream requests (the server allows
/// 300/300 s per IP; we stay far below it — ADR-0002).
pub const MIN_REQUEST_SPACING: Duration = Duration::from_secs(2);
