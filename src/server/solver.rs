//! The explicit flare-solver fallback and the diagnostics page.
//!
//! `POST /solve` is the only place a solve can start: the user presses a button
//! on an edge-block error page. Nothing here runs on a `GET`, in the background,
//! or for a write. The local budget (cooldown + rolling hour) and a single-flight
//! lock keep one Chromium per solve and stop a clicked-again loop.

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
use crate::config::SolverMode;
use crate::consts;
use crate::crypto::AAD_SOLVER;
use crate::diag::{HttpDirection, HttpErrorEvent, Transport};
use crate::error::Error as CoreError;
use crate::render::{DiagnosticsView, NavSection, diagnostics_page};
use crate::time::now_secs;

#[derive(Deserialize)]
pub(super) struct SolveForm {
    /// Work id or AO3 `/works/<id>` URL; never an arbitrary URL.
    target: String,
    send_cookies: Option<String>,
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
    let Some(id) = normalize_work_id(&form.target) else {
        return Err(AppError::BadRequest(
            "Expected an AO3 work id (e.g. 12345678) or a /works/<id> URL.".into(),
        ));
    };

    let _guard = state.solver_lock.try_lock().map_err(|_| {
        AppError::Conflict("A solve is already running; wait for it to finish.".into())
    })?;
    enforce_budget(&state).await?;

    let path = format!("/works/{id}?view_full_work=true");
    let send_cookies = form.send_cookies.is_some();
    let started = Instant::now();
    let result = {
        let mut session = state.session.lock().await;
        session.solve_read(&path, send_cookies).await
    };
    let latency_ms = elapsed_ms(started);

    match result {
        Ok(response) if response.status == 200 => {
            state
                .store
                .record_solver_event(
                    "solved",
                    &path,
                    Some(200),
                    Some(latency_ms),
                    consts::SOLVER_EVENT_RETENTION,
                )
                .await?;
            persist_clearance(&state).await?;
            let existing = state.store.get_work(&id).await?;
            cache_work_html(&state, &id, response.body, existing).await
        }
        Ok(response) => {
            state
                .store
                .record_solver_event(
                    "failed",
                    &path,
                    Some(response.status),
                    Some(latency_ms),
                    consts::SOLVER_EVENT_RETENTION,
                )
                .await?;
            Err(status_error(response.status))
        }
        Err(error) => {
            record_failure(&state, &path, &error, latency_ms).await?;
            Err(AppError::from(error))
        }
    }
}

/// Cooldown and rolling-hour budget, checked before any solver traffic.
async fn enforce_budget(state: &AppState) -> Handled<()> {
    let now = now_secs();
    let cooldown = i64::try_from(consts::FLARESOLVERR_COOLDOWN.as_secs()).unwrap_or(0);
    if let Some(last) = state.store.last_solved_at().await? {
        let wait = cooldown.saturating_sub(now.saturating_sub(last));
        if wait > 0 {
            return Err(AppError::TooManyRequests(format!(
                "A solve already ran recently; wait {wait}s before another."
            )));
        }
    }
    let solves = state.store.solves_since(now.saturating_sub(3_600)).await?;
    if solves >= consts::FLARESOLVERR_MAX_SOLVES_PER_HOUR {
        return Err(AppError::TooManyRequests(format!(
            "Local solve budget reached ({solves}/{} this hour); try again later.",
            consts::FLARESOLVERR_MAX_SOLVES_PER_HOUR
        )));
    }
    Ok(())
}

/// Writes the failed outcome to both ledgers and one redacted diagnostic event.
async fn record_failure(
    state: &AppState,
    path: &str,
    error: &CoreError,
    latency_ms: u64,
) -> Handled<()> {
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

    let mut event = HttpErrorEvent::new(
        HttpDirection::Upstream,
        Transport::Solver,
        "GET",
        path,
        diag_kind(error),
        error.to_string(),
    )
    .with_latency(latency_ms);
    event.status = status;
    drop(state.diag_tx.try_send(event));
    Ok(())
}

/// Encrypts and stores the clearance the browser handed back.
async fn persist_clearance(state: &AppState) -> Handled<()> {
    let snapshot = {
        let session = state.session.lock().await;
        session.clearance_snapshot()
    };
    if let Some((user_agent, cookie_header, expires_at)) = snapshot {
        let ciphertext = state
            .key
            .encrypt(AAD_SOLVER, &cookie_header)
            .map_err(AppError::from)?;
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
        let session = state.session.lock().await;
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

fn diag_kind(error: &CoreError) -> &'static str {
    match error {
        CoreError::SolverCaptcha => "solver_captcha",
        CoreError::SolverUnavailable(_) => "solver_unreachable",
        CoreError::SolverBodyCap => "solver_body_cap",
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
        | CoreError::SolverFailed(_) => "solver_error",
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
