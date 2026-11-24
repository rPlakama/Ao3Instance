//! 30-minute persistent SQLite cache for live AO3 pages (work search, tag
//! pages, tag search, subscriptions, bookmarks, reading history, fandoms, home,
//! and public skins).
//!
//! Every cache miss goes through FlareSolverr when available (and falls back
//! to direct HTTP only when the solver is absent or unreachable). When AO3 is
//! unreachable or rate-limiting, a stale cached copy is served rather than
//! failing the page.

use std::time::Duration;

use super::AppState;
use crate::error::Error;
use crate::time::now_secs;

/// A live AO3 HTML page returned either from the 30-minute SQLite cache or
/// freshly fetched upstream.
#[derive(Debug, Clone)]
pub(super) struct CachedHtml {
    pub body: String,
    pub from_cache: bool,
    pub fetched_at: i64,
}

/// Typed miss failure when neither fresh upstream nor a stale cache entry is
/// available.
pub(super) enum PageFetchError {
    Status {
        status: u16,
        location: Option<String>,
    },
    Error(Error),
}

/// Fetches an AO3 page with a 30-minute SQLite cache and stale-on-error
/// fallback. Pass `refresh = true` to bypass a fresh cached entry.
pub(super) async fn fetch_cached_page(
    state: &AppState,
    account: &str,
    path: &str,
    send_cookies: bool,
    refresh: bool,
) -> std::result::Result<CachedHtml, PageFetchError> {
    let cached = state
        .store
        .get_cached_page(account, path)
        .await
        .ok()
        .flatten();
    let now = now_secs();
    if !refresh
        && let Some(page) = cached.as_ref()
        && page.is_fresh(now)
    {
        return Ok(CachedHtml {
            body: page.body.clone(),
            from_cache: true,
            fetched_at: page.fetched_at,
        });
    }

    let fetched = fetch_live(state, path, send_cookies).await;
    match fetched {
        Ok(body) => {
            if let Err(err) = state.store.put_cached_page(account, path, &body).await {
                eprintln!("put page_cache {path}: {err}");
            }
            Ok(CachedHtml {
                body,
                from_cache: false,
                fetched_at: now_secs(),
            })
        }
        Err(error) => {
            if let Some(stale) = cached {
                return Ok(CachedHtml {
                    body: stale.body,
                    from_cache: true,
                    fetched_at: stale.fetched_at,
                });
            }
            Err(error)
        }
    }
}

/// Same as [`fetch_cached_page`], collapsing unrecoverable upstream failures
/// into `None` so list/directory pages can fall back to local DB rows.
pub(super) async fn fetch_cached_optional(
    state: &AppState,
    account: &str,
    path: &str,
    send_cookies: bool,
    refresh: bool,
) -> Option<CachedHtml> {
    fetch_cached_page(state, account, path, send_cookies, refresh)
        .await
        .ok()
}

/// Returns the signed-in AO3 account name when a complete cookie jar is held,
/// or an empty string when signed out.
pub(super) async fn cache_account(state: &AppState) -> String {
    let snap = state.session_snapshot.read().await;
    if !snap.account.is_empty() && snap.jar_complete {
        snap.account.clone()
    } else {
        String::new()
    }
}

async fn fetch_live(
    state: &AppState,
    path: &str,
    send_cookies: bool,
) -> std::result::Result<String, PageFetchError> {
    if super::solver::solver_available(state).await {
        match super::solver::fetch_via_solver(state, path, send_cookies).await {
            Ok(response) if response.status == 200 => return Ok(response.body),
            Ok(response) => {
                return Err(PageFetchError::Status {
                    status: response.status,
                    location: response.location,
                });
            }
            Err(Error::SolverUnavailable(_)) => {}
            Err(error) => return Err(PageFetchError::Error(error)),
        }
    }

    let response = {
        let mut session = state.session.lock().await;
        let response = session
            .client_mut()
            .get_with_timeout(path, Duration::from_secs(10))
            .await
            .map_err(PageFetchError::Error)?;
        session.sync_csrf_from_response(&response);
        super::sync_session_snapshot(state, &session).await;
        response
    };
    if response.jar_changed {
        super::session::persist_if_complete(state).await;
    }

    if response.status == 200 {
        Ok(response.body)
    } else {
        Err(PageFetchError::Status {
            status: response.status,
            location: response.location,
        })
    }
}
