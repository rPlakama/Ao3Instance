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

/// Rows kept in `http_error_log` before the oldest are pruned.
pub const HTTP_ERROR_LOG_RETENTION: i64 = 500;

/// Host-provided FlareSolverr endpoint (Phase 2.5). Never a build input.
pub const FLARESOLVERR_DEFAULT_URL: &str = "http://127.0.0.1:8191";
/// Liveness probe budget: the service is local, so this is generous.
pub const FLARESOLVERR_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Whole solve budget, including Chromium startup.
pub const FLARESOLVERR_SOLVE_TIMEOUT: Duration = Duration::from_secs(120);
/// Default FlareSolverr `maxTimeout` (challenge-solving budget).
pub const FLARESOLVERR_MAX_TIMEOUT_MS: u64 = 60_000;
/// Minimum gap between two solves, even when the operator clicks again.
pub const FLARESOLVERR_COOLDOWN: Duration = Duration::from_secs(300);
/// Hard cap on successful solves per rolling hour.
pub const FLARESOLVERR_MAX_SOLVES_PER_HOUR: i64 = 6;
/// How long a Cloudflare clearance cookie is considered usable.
pub const FLARESOLVERR_CLEARANCE_TTL: Duration = Duration::from_secs(1_800);
/// Solver events kept in SQLite before pruning.
pub const SOLVER_EVENT_RETENTION: i64 = 200;

/// Freshness window for cached live AO3 pages (search, tags, account lists,
/// fandoms, home, public skins): 30 minutes.
pub const PAGE_CACHE_TTL_SECS: i64 = 30 * 60;
/// Maximum cached AO3 pages kept in SQLite before oldest entries are pruned.
pub const PAGE_CACHE_MAX_ROWS: i64 = 500;

/// Built-in display skins (`(value, preferences_label)`). Single source of truth
/// shared by settings loading, `/skin` validation, chapter rendering, and the
/// session preferences UI.
pub const BUILTIN_SKINS: &[(&str, &str)] = &[
    ("auto", "Auto (follow the system light/dark preference)"),
    ("default", "Classic (light)"),
    ("reversi", "Reversi (dark)"),
    ("snow", "Snow (blue on white)"),
    ("frost", "Frost (cool blue)"),
    ("cloud", "Cloud (soft grey)"),
    ("sepia", "Sepia (warm paper)"),
    ("midnight", "Midnight (deep blue)"),
];

/// Returns `true` when `skin` is one of the built-in palettes.
pub fn is_builtin_skin(skin: &str) -> bool {
    BUILTIN_SKINS.iter().any(|(name, _)| *name == skin)
}

/// Returns `true` when `skin` may be stored in `display.skin` (built-in or imported `"public"`).
pub fn is_stored_skin(skin: &str) -> bool {
    skin == "public" || is_builtin_skin(skin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_validators_accept_known_palettes_and_reject_unknown() {
        for (name, _) in BUILTIN_SKINS {
            assert!(is_builtin_skin(name), "{name}");
            assert!(is_stored_skin(name), "{name}");
        }
        assert!(!is_builtin_skin("public"));
        assert!(is_stored_skin("public"));
        assert!(!is_builtin_skin("neon"));
        assert!(!is_stored_skin("neon"));
    }
}
