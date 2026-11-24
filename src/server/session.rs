//! AO3 session routes: status page, cookie import, password login, probe, adult
//! opt-in and logout.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Form, Query, State},
    http::{HeaderMap, header},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use crate::ao3::cookie_file::parse_cookie_file;
use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::ao3::session::{AccountSession, SessionState};
use crate::crypto::SecretString;
use crate::error::Result;
use crate::render;
use crate::render::NavSection;

/// Upper bound on an uploaded cookie file (2 MiB).
const MAX_COOKIE_FILE_BYTES: usize = 2 * 1024 * 1024;

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
    let snap = state.session_snapshot.read().await.clone();
    Ok(Html(render::session_page(
        &page,
        &snap.account,
        &snap.state,
        snap.jar_complete,
        snap.adult_enabled,
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

/// Persists the current session jar whenever an account with a complete jar is
/// held in memory, so rotated cookies survive service restarts and updates.
pub(super) async fn persist_if_complete(state: &AppState) {
    let snapshot = {
        let session = state.session.lock().await;
        if session.account().is_empty() || !session.jar().is_complete() {
            return;
        }
        match session.jar().encrypt(&state.key) {
            Ok(encrypted) => Some((
                session.account().to_string(),
                encrypted,
                session.pseud().cloned(),
            )),
            Err(err) => {
                eprintln!("encrypt session jar: {err}");
                None
            }
        }
    };
    let Some((account, encrypted, pseud)) = snapshot else {
        return;
    };
    if let Err(err) = state.store.save_session(&account, &encrypted).await {
        eprintln!("persist rotated session: {err}");
        return;
    }
    if let Some(pseud) = pseud
        && let Err(err) = state
            .store
            .set_session_pseud(&account, pseud.id, &pseud.name)
            .await
    {
        eprintln!("persist session pseud: {err}");
    }
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
        session.reset_solver_session().await;
        let res = match session.import(&account, jar).await {
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
        };
        super::sync_session_snapshot(&state, &session).await;
        res
    };
    super::solver::sync_solver_state(&state).await;
    outcome
}

/// Imports cookies from an uploaded file chosen via the browser's native/XDG
/// file dialog (`cookies.txt`, JSON export, or raw `Cookie:` header).
pub(super) async fn session_import_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Handled<Redirect> {
    if body.len() > MAX_COOKIE_FILE_BYTES {
        return Ok(redirect_session(
            "error",
            "Cookie file is too large (limit is 2 MiB).",
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let Some(boundary) = extract_multipart_boundary(content_type) else {
        return Err(AppError::BadRequest(
            "expected multipart/form-data upload for cookie file".into(),
        ));
    };
    let fields = parse_multipart_fields(&boundary, &body);
    let mut file_text = String::new();
    let mut account = String::new();
    let mut adult = false;
    for (name, value) in fields {
        match name.as_str() {
            "cookie_file" => file_text = value,
            "account" => account = value.trim().to_string(),
            "adult" => adult = !value.trim().is_empty(),
            _ => {}
        }
    }
    let mut imported = match parse_cookie_file(&file_text) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Ok(redirect_session(
                "error",
                &format!("Cookie file import failed: {error}"),
            ));
        }
    };
    adult_cookie_jar(&mut imported.jar, adult);

    let (resolved_account, outcome) = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        session.reset_solver_session().await;

        let target_account = if account.is_empty() {
            match session.discover_account_from_jar(&imported.jar).await {
                Ok(detected) => detected,
                Err(error) => {
                    return Ok(redirect_session(
                        "error",
                        &format!("Cookie file import failed: {error}"),
                    ));
                }
            }
        } else {
            account
        };

        let result = match session.import(&target_account, imported.jar).await {
            Ok(SessionState::Active) => {
                persist_session(&state, &session)
                    .await
                    .map_err(AppError::from)?;
                Ok(redirect_session(
                    "ok",
                    &format!(
                        "Cookie file imported for {} ({} cookies) and session verified.",
                        session.account(),
                        imported.count
                    ),
                ))
            }
            Ok(other) => Ok(redirect_session(
                "error",
                &format!("Cookie file imported but session is not active: {other}."),
            )),
            Err(error) => Ok(redirect_session(
                "error",
                &format!("Cookie file import failed: {error}"),
            )),
        };
        super::sync_session_snapshot(&state, &session).await;
        (session.account().to_string(), result)
    };
    if !resolved_account.is_empty()
        && let Err(err) = state
            .store
            .invalidate_account_pages(&resolved_account)
            .await
    {
        eprintln!("invalidate account page_cache: {err}");
    }
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
        session.reset_solver_session().await;
        let result = session
            .login(&account, SecretString::new(form.password))
            .await;
        let res = match result {
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
        };
        super::sync_session_snapshot(&state, &session).await;
        res
    };
    super::solver::sync_solver_state(&state).await;
    outcome
}

pub(super) async fn session_probe(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    let outcome = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        let res = match session.verify_and_refresh().await {
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
        };
        super::sync_session_snapshot(&state, &session).await;
        res
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
    super::sync_session_snapshot(&state, &session).await;
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
        session.reset_solver_session().await;
        super::sync_session_snapshot(&state, &session).await;
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

/// Extracts the `boundary` parameter from a `multipart/form-data` header.
fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let media = parts.next()?.trim();
    if !media.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    for param in parts {
        let Some((key, val)) = param.trim().split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            let trimmed = val.trim().trim_matches('"');
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Minimal RFC 7578 `multipart/form-data` parser without extra crate deps.
fn parse_multipart_fields(boundary: &str, body: &[u8]) -> Vec<(String, String)> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut out = Vec::new();
    let mut cursor = match find_bytes(body, &delimiter) {
        Some(pos) => pos + delimiter.len(),
        None => return out,
    };

    while cursor < body.len() {
        // Epilogue marker `--` ends the multipart stream.
        if body[cursor..].starts_with(b"--") {
            break;
        }
        // Skip CRLF / LF right after the boundary token.
        if body[cursor..].starts_with(b"\r\n") {
            cursor += 2;
        } else if body[cursor..].starts_with(b"\n") {
            cursor += 1;
        }
        let next = match find_bytes(&body[cursor..], &delimiter) {
            Some(rel) => cursor + rel,
            None => break,
        };
        let mut part = &body[cursor..next];
        if let Some(stripped) = part.strip_suffix(b"\r\n") {
            part = stripped;
        } else if let Some(stripped) = part.strip_suffix(b"\n") {
            part = stripped;
        }

        let (header_bytes, value_bytes) = if let Some(sep) = find_bytes(part, b"\r\n\r\n") {
            (&part[..sep], &part[sep + 4..])
        } else if let Some(sep) = find_bytes(part, b"\n\n") {
            (&part[..sep], &part[sep + 2..])
        } else {
            cursor = next + delimiter.len();
            continue;
        };

        let headers_str = String::from_utf8_lossy(header_bytes);
        if let Some(name) = extract_disposition_name(&headers_str) {
            let value = String::from_utf8_lossy(value_bytes).into_owned();
            out.push((name, value));
        }
        cursor = next + delimiter.len();
    }
    out
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn extract_disposition_name(headers: &str) -> Option<String> {
    for line in headers.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        for param in rest.split(';') {
            let Some((k, v)) = param.trim().split_once('=') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case("name") {
                let name = v.trim().trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
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

    #[test]
    fn parses_multipart_cookie_file_upload() {
        let ct = "multipart/form-data; boundary=\"----WebKitFormBoundary123\"";
        let boundary = extract_multipart_boundary(ct).expect("boundary");
        assert_eq!(boundary, "----WebKitFormBoundary123");

        let payload = concat!(
            "------WebKitFormBoundary123\r\n",
            "Content-Disposition: form-data; name=\"account\"\r\n\r\n",
            "someone\r\n",
            "------WebKitFormBoundary123\r\n",
            "Content-Disposition: form-data; name=\"cookie_file\"; filename=\"cookies.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "_otwarchive_session=s1; user_credentials=1\r\n",
            "------WebKitFormBoundary123\r\n",
            "Content-Disposition: form-data; name=\"adult\"\r\n\r\n",
            "1\r\n",
            "------WebKitFormBoundary123--\r\n"
        );
        let fields = parse_multipart_fields(&boundary, payload.as_bytes());
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("account".into(), "someone".into()));
        assert_eq!(
            fields[1],
            (
                "cookie_file".into(),
                "_otwarchive_session=s1; user_credentials=1".into()
            )
        );
        assert_eq!(fields[2], ("adult".into(), "1".into()));
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
    let account = state.session_snapshot.read().await.account.clone();
    Ok(Html(render::logout_confirm_page(&page, &account)))
}
