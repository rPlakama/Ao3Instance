//! AO3 session routes: status page, cookie import, password login, probe, adult
//! opt-in and logout.

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::ao3::session::{AccountSession, SessionState};
use crate::crypto::SecretString;
use crate::error::Result;
use crate::render;
use crate::render::NavSection;

#[derive(Deserialize)]
pub(super) struct SessionQuery {
    error: Option<String>,
    ok: Option<String>,
}

pub(super) async fn session_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionQuery>,
) -> Handled<Html<String>> {
    let page = super::page_context(&state, "Session — AO3Instance", NavSection::Session).await;
    let session = state.session.lock().await;
    Ok(Html(render::session_page(
        &page,
        session.account(),
        &session.state().to_string(),
        session.jar().is_complete(),
        session.jar().contains("view_adult"),
        query.ok.as_deref(),
        query.error.as_deref(),
    )))
}

fn redirect_session(kind: &str, message: &str) -> Redirect {
    Redirect::to(&format!(
        "/session?{kind}={}",
        percent_encode(message, SpaceMode::Percent20)
    ))
}

/// Cookie values come from a browser; they must never smuggle separators or
/// control characters into our Cookie header.
fn cookie_value(value: &str) -> Handled<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("cookie value is empty".into()));
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control() || ch == ';' || ch == ',')
    {
        return Err(AppError::BadRequest(
            "cookie value contains an illegal character".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn adult_cookie_jar(jar: &mut Jar, adult: bool) {
    if adult {
        jar.apply("view_adult=1; Path=/");
    }
}

pub(super) async fn persist_session(state: &AppState, session: &AccountSession) -> Result<()> {
    let encrypted = session.jar().encrypt(&state.key)?;
    state
        .store
        .save_session(session.account(), &encrypted)
        .await?;
    if let Some(pseud) = session.pseud() {
        state
            .store
            .set_session_pseud(session.account(), pseud.id, &pseud.name)
            .await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct ImportForm {
    account: String,
    session_cookie: String,
    credentials_cookie: String,
    adult: Option<String>,
}

pub(super) async fn session_import(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ImportForm>,
) -> Handled<Redirect> {
    let session_cookie = cookie_value(&form.session_cookie)?;
    let credentials_cookie = cookie_value(&form.credentials_cookie)?;
    let account = form.account.trim().to_string();
    if account.is_empty() {
        return Err(AppError::BadRequest("AO3 login is empty".into()));
    }

    let mut jar = Jar::new();
    jar.apply(&format!("_otwarchive_session={session_cookie}; Path=/"));
    jar.apply(&format!("user_credentials={credentials_cookie}; Path=/"));
    adult_cookie_jar(&mut jar, form.adult.is_some());

    let outcome = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        match session.import(&account, jar).await {
            Ok(SessionState::Active) => {
                persist_session(&state, &session)
                    .await
                    .map_err(AppError::from)?;
                Ok(redirect_session("ok", "Session imported and verified."))
            }
            Ok(other) => Ok(redirect_session(
                "error",
                &format!("Session imported but not active: {other}."),
            )),
            Err(error) => Ok(redirect_session(
                "error",
                &format!("Session import failed: {error}"),
            )),
        }
    };
    super::solver::sync_solver_state(&state).await;
    outcome
}

#[derive(Deserialize)]
pub(super) struct LoginForm {
    account: String,
    password: String,
    adult: Option<String>,
}

pub(super) async fn session_login(
    State(state): State<Arc<AppState>>,
    Form(form): Form<LoginForm>,
) -> Handled<Redirect> {
    let account = form.account.trim().to_string();
    if account.is_empty() {
        return Err(AppError::BadRequest("AO3 login is empty".into()));
    }
    let outcome = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        let result = session
            .login(&account, SecretString::new(form.password))
            .await;
        match result {
            Ok(SessionState::Active) => {
                adult_cookie_jar(session.client_mut().jar_mut(), form.adult.is_some());
                persist_session(&state, &session)
                    .await
                    .map_err(AppError::from)?;
                Ok(redirect_session("ok", "Signed in to AO3."))
            }
            Ok(other) => Ok(redirect_session(
                "error",
                &format!("Signed in, but the session is not active: {other}."),
            )),
            Err(error) => Ok(redirect_session("error", &format!("Login failed: {error}"))),
        }
    };
    super::solver::sync_solver_state(&state).await;
    outcome
}

pub(super) async fn session_probe(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    let outcome = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        match session.probe().await {
            Ok(SessionState::Active) => {
                persist_session(&state, &session)
                    .await
                    .map_err(AppError::from)?;
                Ok(redirect_session("ok", "Session verified."))
            }
            Ok(other) => Ok(redirect_session(
                "error",
                &format!("Session is not active: {other}."),
            )),
            Err(error) => Ok(redirect_session(
                "error",
                &format!("Session check failed: {error}"),
            )),
        }
    };
    super::solver::sync_solver_state(&state).await;
    outcome
}

pub(super) async fn session_adult(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    let mut session = state.session.lock().await;
    if session.account().is_empty() {
        return Ok(redirect_session(
            "error",
            "Import or sign in to a session before enabling adult content.",
        ));
    }
    adult_cookie_jar(session.client_mut().jar_mut(), true);
    if let Err(error) = persist_session(&state, &session).await {
        return Err(AppError::from(error));
    }
    Ok(redirect_session(
        "ok",
        "Adult content enabled for this session.",
    ))
}

pub(super) async fn session_logout(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    let prev_account = {
        let mut session = state.session.lock().await;
        let prev = session.account().to_string();
        session.clear_local();
        prev
    };
    if !prev_account.is_empty()
        && let Err(err) = state.store.invalidate_account_pages(&prev_account).await
    {
        eprintln!("invalidate account page_cache: {err}");
    }
    state.store.delete_session().await.map_err(AppError::from)?;
    Ok(redirect_session("ok", "Account removed locally."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_values_reject_separators() {
        assert!(cookie_value("abc123--def").is_ok());
        assert!(cookie_value("").is_err());
        assert!(cookie_value("a b").is_err());
        assert!(cookie_value("a;b").is_err());
        assert!(cookie_value("a\r\nb").is_err());
    }
}

#[derive(Deserialize)]
pub(super) struct SkinForm {
    skin: String,
    next: Option<String>,
}

/// Single-operator display preference; the cookie-free global is deliberate.
pub(super) async fn skin(
    State(state): State<Arc<AppState>>,
    Form(form): Form<SkinForm>,
) -> Handled<Redirect> {
    let skin = match form.skin.trim() {
        "auto" | "default" | "reversi" | "snow" | "frost" | "cloud" | "sepia" | "midnight" => {
            form.skin.trim()
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown skin {other:?}; expected auto, default, reversi, snow, frost, cloud, sepia or midnight"
            )));
        }
    };
    state.store.set_setting("display.skin", skin).await?;
    state.store.set_setting("display.skin_css", "").await?;
    *state.skin.write().await = skin.to_string();
    *state.skin_css.write().await = None;
    let next = form
        .next
        .as_deref()
        .filter(|next| next.starts_with('/') && !next.starts_with("//"))
        .unwrap_or("/");
    Ok(Redirect::to(next))
}

pub(super) async fn logout_confirm(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let page = super::page_context(&state, "Log out — AO3Instance", NavSection::Session).await;
    let account = state.session.lock().await.account().to_string();
    Ok(Html(render::logout_confirm_page(&page, &account)))
}
