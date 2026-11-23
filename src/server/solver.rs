//! Host-provided FlareSolverr transport and diagnostics page.
//!
//! When the local FlareSolverr is reachable (and `AO3I_FLARESOLVERR` is not
//! `off`), read routes always send upstream `GET`s through it and fall back to
//! the direct client only when the solver is absent or unreachable. Writes keep
//! the direct client (riding on any captured Cloudflare clearance). The
//! single-flight lock serializes solver requests so one Chromium runs at a time.

use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{Form, Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use super::reads::{cache_work_html, normalize_work_id};
use crate::ao3::client::Response as UpstreamResponse;
use crate::config::SolverMode;
use crate::consts;
use crate::crypto::AAD_SOLVER;
use crate::error::Error as CoreError;
use crate::render::{DiagnosticsView, NavSection, diagnostics_page};
use crate::time::now_secs;

#[derive(Deserialize)]
pub(super) struct SolveForm {
    /// Work id / `/works/<id>` URL, or an exact `/works/search?...` path.
    target: String,
    /// Local search URL to return to after a successful search solve.
    next: Option<String>,
    send_cookies: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum SolveTarget {
    Work(String),
    Search(String),
}

fn solve_target(input: &str) -> Option<SolveTarget> {
    let trimmed = input.trim();
    if let Some(search) = trimmed.strip_prefix("/works/search") {
        let path = if search.is_empty() {
            "/works/search".to_string()
        } else {
            let query = search.strip_prefix('?')?;
            format!("/works/search?{query}")
        };
        if path.contains("://") || path.contains(['\n', '\r']) {
            return None;
        }
        return Some(SolveTarget::Search(path));
    }
    normalize_work_id(trimmed).map(SolveTarget::Work)
}

fn local_search_return(input: Option<&str>) -> Handled<String> {
    let next = input
        .map(str::trim)
        .filter(|value| value.starts_with("/works/search"))
        .filter(|value| !value.contains("://") && !value.contains(['\n', '\r']))
        .ok_or_else(|| {
            AppError::BadRequest("A search solve needs a local /works/search return path.".into())
        })?;
    Ok(next.to_string())
}

pub(super) async fn solve(
    State(state): State<Arc<AppState>>,
    Form(form): Form<SolveForm>,
) -> Handled<Redirect> {
    let mode = {
        let session = state.session.lock().await;
        session.solver_mode()
    };
    if mode == SolverMode::Off {
        return Err(AppError::Forbidden(
            "The local FlareSolverr fallback is disabled (AO3I_FLARESOLVERR=off).".into(),
        ));
    }
    let target = solve_target(&form.target).ok_or_else(|| {
        AppError::BadRequest(
            "Expected an AO3 work id, /works/<id> URL, or /works/search?... path.".into(),
        )
    })?;

    if let SolveTarget::Search(path) = target {
        let next = local_search_return(form.next.as_deref())?;
        let send_cookies = form.send_cookies.is_some();
        let response = fetch_via_solver(&state, &path, send_cookies)
            .await
            .map_err(AppError::from)?;
        if response.status == 200 {
            let separator = if next.contains('?') { '&' } else { '?' };
            let redirect = format!("{next}{separator}solved=1");
            *state.solved_search.lock().await = Some(super::SolvedSearch {
                html: response.body,
                at: now_secs(),
            });
            return Ok(Redirect::to(&redirect));
        }
        return Err(status_error(response.status));
    }

    let SolveTarget::Work(id) = target else {
        unreachable!("search target returned above")
    };
    let path = format!("/works/{id}?view_full_work=true");
    let send_cookies = form.send_cookies.is_some();
    let response = fetch_via_solver(&state, &path, send_cookies)
        .await
        .map_err(AppError::from)?;
    if response.status == 200 {
        let existing = state.store.get_work(&id).await?;
        cache_work_html(&state, &id, response.body, existing).await
    } else {
        Err(status_error(response.status))
    }
}

/// True when the local FlareSolverr is enabled and reachable. Re-probes if the
/// solver was previously unreachable and the reprobe interval has elapsed.
pub(super) async fn solver_available(state: &AppState) -> bool {
    let (available, status) = {
        let mut session = state.session.lock().await;
        let available = session.solver_available().await;
        (available, session.solver_status().clone())
    };
    *state.solver_status.write().await = status;
    available
}

/// Syncs `AppState::solver_status` from the session, persists any newly
/// captured Cloudflare clearance, and persists rotated AO3 session cookies.
pub(super) async fn sync_solver_state(state: &AppState) {
    let status = {
        let session = state.session.lock().await;
        session.solver_status().clone()
    };
    *state.solver_status.write().await = status;
    if let Err(err) = persist_clearance(state).await {
        eprintln!("persist solver clearance: {err}");
    }
    super::session::persist_if_complete(state).await;
}

/// Runs one upstream `GET` through the local FlareSolverr, serializing solver
/// calls so at most one browser solve runs at a time.
pub(super) async fn fetch_via_solver(
    state: &AppState,
    path: &str,
    send_cookies: bool,
) -> std::result::Result<UpstreamResponse, CoreError> {
    let _guard = state.solver_lock.lock().await;
    let started = Instant::now();
    let (result, status) = {
        let mut session = state.session.lock().await;
        let result = session.solve_get(path, send_cookies).await;
        (result, session.solver_status().clone())
    };
    *state.solver_status.write().await = status;
    let latency_ms = elapsed_ms(started);
    match result {
        Ok(response) => {
            let outcome = if response.status == 200 {
                "solved"
            } else {
                "failed"
            };
            if let Err(err) = state
                .store
                .record_solver_event(
                    outcome,
                    path,
                    Some(response.status),
                    Some(latency_ms),
                    consts::SOLVER_EVENT_RETENTION,
                )
                .await
            {
                eprintln!("record solver event: {err}");
            }
            if response.status == 200
                && let Err(err) = persist_clearance(state).await
            {
                eprintln!("persist solver clearance: {err}");
            }
            if response.jar_changed {
                super::session::persist_if_complete(state).await;
            }
            Ok(response)
        }
        Err(error) => {
            if let Err(err) = record_failure(state, path, &error, latency_ms).await {
                eprintln!("record solver failure: {err}");
            }
            Err(error)
        }
    }
}

/// Writes the failed outcome to the solver event ledger (HTTP diagnostics are
/// already emitted by `AccountSession::solve_request`).
async fn record_failure(
    state: &AppState,
    path: &str,
    error: &CoreError,
    latency_ms: u64,
) -> crate::error::Result<()> {
    let status = match error {
        CoreError::BlockedByEdge { status } => Some(*status),
        CoreError::RateLimited { .. } => Some(429),
        CoreError::Config(_)
        | CoreError::Io(_)
        | CoreError::Store(_)
        | CoreError::Crypto(_)
        | CoreError::Network(_)
        | CoreError::LoginRejected(_)
        | CoreError::PasswordChangeRequired
        | CoreError::LoggedOut
        | CoreError::Validation(_)
        | CoreError::SpamRejected
        | CoreError::SessionIncomplete
        | CoreError::CsrfMissing
        | CoreError::CsrfExpired
        | CoreError::LogoutFailed(_)
        | CoreError::Killed
        | CoreError::UpstreamChanged(_)
        | CoreError::SolverDisabled
        | CoreError::SolverUnavailable(_)
        | CoreError::SolverCaptcha
        | CoreError::SolverFailed(_)
        | CoreError::SolverBodyCap => None,
    };
    state
        .store
        .record_solver_event(
            outcome_for(error),
            path,
            status,
            Some(latency_ms),
            consts::SOLVER_EVENT_RETENTION,
        )
        .await?;
    Ok(())
}

/// Encrypts and stores the clearance the browser handed back.
pub(super) async fn persist_clearance(state: &AppState) -> crate::error::Result<()> {
    let snapshot = {
        let session = state.session.lock().await;
        session.clearance_snapshot()
    };
    if let Some((user_agent, cookie_header, expires_at)) = snapshot {
        let ciphertext = state.key.encrypt(AAD_SOLVER, &cookie_header)?;
        state
            .store
            .save_solver_clearance(&ciphertext, &user_agent, expires_at)
            .await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct DiagnosticsQuery {
    direction: Option<String>,
}

pub(super) async fn diagnostics(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DiagnosticsQuery>,
) -> Handled<Html<String>> {
    let direction = match query.direction.as_deref() {
        Some("upstream") => Some("upstream"),
        Some("inbound") => Some("inbound"),
        _ => None,
    };
    let errors = state.store.recent_http_errors(100, direction).await?;
    let events = state.store.recent_solver_events(20).await?;
    let cached_works = state.store.cached_work_ids().await?;
    let status = state.solver_status.read().await.clone();
    let (send_cookies_allowed, clearance_expires_at) = {
        let session = state.session.lock().await;
        (
            session.solver_may_receive_cookies(),
            session
                .clearance_snapshot()
                .map(|(_, _, expires_at)| expires_at),
        )
    };
    let solves_last_hour = state
        .store
        .solves_since(now_secs().saturating_sub(3_600))
        .await?;
    let page =
        super::page_context(&state, "Diagnostics — AO3Instance", NavSection::Diagnostics).await;
    let view = DiagnosticsView {
        errors: &errors,
        events: &events,
        solver_mode: status.mode,
        solver_reachable: status.reachable,
        solver_version: status.version.as_deref(),
        send_cookies_allowed,
        clearance_expires_at,
        solves_last_hour,
        direction,
        cached_works: &cached_works,
    };
    Ok(Html(diagnostics_page(&page, &view)))
}

pub(super) async fn probe(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    let status = {
        let mut session = state.session.lock().await;
        session.probe_solver().await
    };
    *state.solver_status.write().await = status;
    Ok(Redirect::to("/diagnostics"))
}

pub(super) async fn clear_clearance(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    {
        let mut session = state.session.lock().await;
        session.clear_clearance();
    }
    state.store.clear_solver_clearance().await?;
    Ok(Redirect::to("/diagnostics"))
}

pub(super) async fn clear_logs(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    state.store.clear_http_errors().await?;
    Ok(Redirect::to("/diagnostics"))
}

fn status_error(status: u16) -> AppError {
    match status {
        404 => AppError::NotFound,
        429 => AppError::TooManyRequests("AO3 rate limited the solve; wait and retry.".into()),
        403 | 503 => AppError::Blocked {
            status,
            work_id: String::new(),
            solver_reachable: false,
            send_cookies_allowed: false,
        },
        status => AppError::Upstream(format!(
            "Unexpected HTTP {status} from AO3 through the local solver."
        )),
    }
}

fn outcome_for(error: &CoreError) -> &'static str {
    match error {
        CoreError::SolverCaptcha => "captcha",
        CoreError::SolverUnavailable(_) => "unavailable",
        CoreError::SolverDisabled
        | CoreError::Config(_)
        | CoreError::Io(_)
        | CoreError::Store(_)
        | CoreError::Crypto(_)
        | CoreError::Network(_)
        | CoreError::LoginRejected(_)
        | CoreError::PasswordChangeRequired
        | CoreError::LoggedOut
        | CoreError::Validation(_)
        | CoreError::SpamRejected
        | CoreError::SessionIncomplete
        | CoreError::CsrfMissing
        | CoreError::CsrfExpired
        | CoreError::LogoutFailed(_)
        | CoreError::Killed
        | CoreError::RateLimited { .. }
        | CoreError::BlockedByEdge { .. }
        | CoreError::UpstreamChanged(_)
        | CoreError::SolverFailed(_)
        | CoreError::SolverBodyCap => "failed",
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_targets_stay_on_ao3_paths() {
        assert_eq!(
            solve_target("12345"),
            Some(SolveTarget::Work("12345".into()))
        );
        assert_eq!(
            solve_target("https://archiveofourown.org/works/12345"),
            Some(SolveTarget::Work("12345".into()))
        );
        assert_eq!(
            solve_target("/works/search?work_search%5Bquery%5D=re%20zero"),
            Some(SolveTarget::Search(
                "/works/search?work_search%5Bquery%5D=re%20zero".into()
            ))
        );
        assert_eq!(
            solve_target("https://archiveofourown.org/works/search?q=x"),
            None
        );
        assert_eq!(solve_target("/works/search/evil"), None);
    }

    #[test]
    fn search_return_paths_stay_local() {
        assert!(local_search_return(Some("/works/search?q=re%20zero")).is_ok());
        assert!(local_search_return(Some("https://example.org/works/search")).is_err());
        assert!(local_search_return(None).is_err());
    }
}
