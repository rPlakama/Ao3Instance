//! The web application: sign in to AO3, open a work, cache it, read it offline.
//!
//! Server-rendered on purpose (`docs/write-client-plan.md` D5): no SPA, no client
//! state to desynchronize. The stateful things are the SQLite cache, the reading
//! position and one persisted AO3 session (encrypted jar).

use std::sync::Arc;

use axum::{
    Router,
    extract::{Form, Path, Query, State},
    http::{HeaderName, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::ao3::actions;
use crate::ao3::governor::WriteGovernor;
use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::ao3::session::{AccountSession, SessionState};
use crate::ao3::work::parse_work_page;
use crate::config::Config;
use crate::crypto::{SecretKey, SecretString};
use crate::error::{Error, Result};
use crate::render;
use crate::store::{Store, sha256_hex};

/// Handler result: an `AppError` renders a full HTML error page.
type Handled<T> = std::result::Result<T, AppError>;

/// CSP for the chapter iframe: no scripts, no forms, inline styles only, images
/// from https only. Author content never runs in the app's origin.
const CHAPTER_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; \
     font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

pub struct AppState {
    pub store: Store,
    pub session: Mutex<AccountSession>,
    pub key: SecretKey,
    pub governor: WriteGovernor,
    pub write_enabled: bool,
}

enum AppError {
    BadRequest(String),
    Forbidden(String),
    NotFound,
    Upstream(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "Not cached. Open the work from the start page first.".to_string(),
            ),
            Self::Upstream(message) => (StatusCode::BAD_GATEWAY, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Html(render::error_page(status.as_u16(), &message))).into_response()
    }
}

impl From<Error> for AppError {
    fn from(error: Error) -> Self {
        match error {
            Error::Config(message)
            | Error::Io(message)
            | Error::Store(message)
            | Error::Crypto(message) => Self::Internal(message),
            Error::Network(message) | Error::UpstreamChanged(message) => Self::Upstream(message),
            Error::RateLimited { retry_after } => Self::Upstream(match retry_after {
                Some(secs) => format!("AO3 is rate limiting this instance; retry after {secs}s"),
                None => "AO3 is rate limiting this instance; wait and retry".to_string(),
            }),
            Error::BlockedByEdge { status } => {
                Self::Upstream(format!("AO3 blocked the request (HTTP {status})"))
            }
            Error::Validation(message) => Self::BadRequest(message),
            Error::LoggedOut | Error::SessionIncomplete => Self::Upstream(
                "The AO3 session is not usable. Import cookies on the Session page.".to_string(),
            ),
            Error::CsrfMissing | Error::CsrfExpired => Self::Upstream(
                "AO3 rejected the CSRF token. Verify the session on the Session page.".to_string(),
            ),
            Error::LoginRejected(message) => {
                Self::Upstream(format!("AO3 rejected the login: {message}"))
            }
            Error::PasswordChangeRequired => Self::Upstream(
                "AO3 requires a password change before this session can be used.".to_string(),
            ),
            Error::LogoutFailed(message) => {
                Self::Upstream(format!("Logout did not complete upstream: {message}"))
            }
            Error::Killed => Self::Internal("Upstream traffic is disabled by the operator".into()),
        }
    }
}

/// Builds the router. Split from [`run`] so tests can serve it on an ephemeral
/// port. Loads the persisted session (without probing it).
pub async fn router(config: &Config, store: Store) -> Result<Router> {
    let key = SecretKey::load_or_create(&config.key_file)?;
    let mut session = AccountSession::new(config)?;

    if let Some((account, jar_enc)) = store.load_session().await? {
        match Jar::decrypt(&key, &jar_enc) {
            Ok(jar) => match session.restore(&account, jar) {
                Ok(state) => eprintln!("session restored for {account} ({state})"),
                Err(error) => eprintln!("stored session refused: {error}"),
            },
            Err(error) => eprintln!("stored session could not be decrypted: {error}"),
        }
    }

    let state = Arc::new(AppState {
        store,
        session: Mutex::new(session),
        key,
        governor: WriteGovernor::new(config.min_request_spacing),
        write_enabled: config.write_enabled,
    });
    Ok(Router::new()
        .route("/", get(home))
        .route("/open", post(open_work))
        .route("/read/{id}", get(read_work))
        .route("/read/{id}/chapter/{idx}", get(read_chapter))
        .route("/search", get(search))
        .route("/progress", post(save_progress))
        .route("/session", get(session_page))
        .route("/session/import", post(session_import))
        .route("/session/login", post(session_login))
        .route("/session/probe", post(session_probe))
        .route("/session/adult", post(session_adult))
        .route("/session/logout", post(session_logout))
        .route("/write/preview", get(write_preview))
        .route("/write/subscribe", post(write_subscribe))
        .route("/write/unsubscribe", post(write_unsubscribe))
        .route("/write/kudos", post(write_kudos))
        .route("/write/reading", post(write_reading))
        .route("/audit", get(audit))
        .route("/health", get(health))
        .with_state(state))
}

/// Opens the database and serves until Ctrl-C.
pub async fn run(config: &Config) -> Result<()> {
    let pool = crate::db::open(&config.db_file).await?;
    let store = Store::new(pool);
    let app = router(config, store).await?;
    let addr = format!("{}:{}", config.bind, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| Error::Store(format!("bind {addr}: {e}")))?;
    println!("AO3Instance ready at http://{addr}/  (Ctrl-C to stop)");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|e| Error::Store(format!("server error: {e}")))?;
    Ok(())
}

async fn shutdown() {
    drop(tokio::signal::ctrl_c().await);
}

#[derive(Deserialize)]
struct HomeQuery {
    notice: Option<String>,
}

async fn home(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HomeQuery>,
) -> Handled<Html<String>> {
    let works = state.store.list_works(50).await?;
    let flash = query.notice.as_deref().and_then(notice_text);
    Ok(Html(render::home_page(&works, flash)))
}

#[derive(Deserialize)]
struct OpenForm {
    id: String,
    /// `refresh=1` re-fetches even when the work is cached.
    refresh: Option<String>,
}

async fn open_work(
    State(state): State<Arc<AppState>>,
    Form(form): Form<OpenForm>,
) -> Handled<Redirect> {
    let Some(id) = normalize_work_id(&form.id) else {
        return Err(AppError::BadRequest(
            "Expected an AO3 work id (e.g. 12345678) or a /works/<id> URL.".into(),
        ));
    };
    let existing = state.store.get_work(&id).await?;
    // Cache first: re-opening a cached work is an offline read, not a fetch.
    if form.refresh.is_none() && existing.is_some() {
        return Ok(Redirect::to(&format!("/read/{id}")));
    }
    let html = fetch_work_html(&state, &id).await?;
    let mut page = match parse_work_page(&html) {
        Ok(page) => page,
        Err(Error::UpstreamChanged(_)) if looks_like_adult_gate(&html) => {
            return Err(AppError::Upstream(
                "This work is behind AO3's adult-content gate. Enable adult content on the \
                 Session page (or sign in), then reopen the work."
                    .into(),
            ));
        }
        Err(Error::UpstreamChanged(_)) if looks_like_login_gate(&html) => {
            return Err(AppError::Upstream(
                "AO3 says this work needs a signed-in account. Import your cookies on the \
                 Session page, then reopen the work."
                    .into(),
            ));
        }
        Err(error) => return Err(AppError::from(error)),
    };
    // The id in our URL is authoritative: the page's links may be relative or
    // (for synthetic fixtures) point elsewhere.
    page.ao3_id = id.clone();
    let content_hash = sha256_hex(html.as_bytes());
    match existing {
        Some(work) if work.content_hash == content_hash => {
            // Same bytes upstream: record the check, do not rewrite the cache.
            state.store.touch_checked(&id).await?;
            Ok(Redirect::to(&format!("/read/{id}?checked=1")))
        }
        Some(_) => {
            state.store.upsert_work(&page, &content_hash).await?;
            Ok(Redirect::to(&format!("/read/{id}?updated=1")))
        }
        None => {
            state.store.upsert_work(&page, &content_hash).await?;
            Ok(Redirect::to(&format!("/read/{id}")))
        }
    }
}

async fn fetch_work_html(state: &AppState, id: &str) -> Handled<String> {
    // One request, user-initiated; `view_full_work` asks AO3 for all chapters in
    // a single page (ADR-0002: no crawling, no paging picks).
    let path = format!("/works/{id}?view_full_work=true");
    let mut session = state.session.lock().await;
    let response = session
        .client_mut()
        .get(&path)
        .await
        .map_err(AppError::from)?;
    match response.status {
        200 => Ok(response.body),
        404 => Err(AppError::NotFound),
        429 => Err(AppError::Upstream(
            "AO3 is rate limiting this instance; wait a minute and retry.".into(),
        )),
        403 | 503 => Err(AppError::Upstream(format!(
            "AO3 refused the request (HTTP {}); it may be a bot challenge.",
            response.status
        ))),
        302 | 303 => Err(AppError::Upstream(
            "AO3 redirected this work (restricted or login-gated). Sign in on the Session page."
                .into(),
        )),
        status => Err(AppError::Upstream(format!(
            "Unexpected HTTP {status} from AO3."
        ))),
    }
}

#[derive(Deserialize)]
struct ReadQuery {
    chapter: Option<i64>,
    /// Set by a refresh that found no upstream change.
    checked: Option<String>,
    /// Set by a refresh that stored a new version.
    updated: Option<String>,
    /// Fixed-key notice from a write action (never raw user input).
    notice: Option<String>,
}

async fn read_work(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
) -> Handled<Html<String>> {
    let work = state.store.get_work(&id).await?.ok_or(AppError::NotFound)?;
    let chapters = state.store.get_chapters(&id).await?;
    let progress = state.store.get_progress(&id).await?;
    let current = query
        .chapter
        .or(progress)
        .unwrap_or_else(|| chapters.first().map(|chapter| chapter.idx).unwrap_or(1));
    let notice = if query.checked.is_some() {
        Some("Checked AO3 — no changes since the cached copy.")
    } else if query.updated.is_some() {
        Some("Updated from AO3.")
    } else {
        query.notice.as_deref().and_then(notice_text)
    };
    let (account, usable) = session_identity(&state).await;
    let subscribed = if usable && !account.is_empty() {
        state
            .store
            .get_subscription(&account, actions::WORK, &id)
            .await?
            .is_some_and(|row| row.state == "active")
    } else {
        false
    };
    let can_write = usable && !account.is_empty() && state.write_enabled;
    let kudos_sent = can_write && state.store.has_kudos(&account, &id).await?;
    let reading_state = if can_write {
        state.store.get_reading_state(&account, &id).await?
    } else {
        None
    };
    Ok(Html(render::read_page(
        &work,
        &chapters,
        &render::ReadContext {
            current,
            progress,
            notice,
            subscribed,
            can_write,
            kudos_sent,
            reading_state: reading_state.as_deref(),
        },
    )))
}

async fn read_chapter(
    State(state): State<Arc<AppState>>,
    Path((id, idx)): Path<(String, i64)>,
) -> Handled<Response> {
    let work = state.store.get_work(&id).await?.ok_or(AppError::NotFound)?;
    let chapters = state.store.get_chapters(&id).await?;
    let chapter = chapters
        .iter()
        .find(|chapter| chapter.idx == idx)
        .ok_or(AppError::NotFound)?;
    let mut response = Html(render::chapter_document(&work.title, chapter)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CHAPTER_CSP),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Handled<Html<String>> {
    let raw = query.q.unwrap_or_default();
    let works = state.store.search_works(&raw, 50).await?;
    Ok(Html(render::search_page(&raw, &works)))
}

#[derive(Deserialize)]
struct ProgressForm {
    work_id: String,
    chapter: i64,
}

async fn save_progress(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ProgressForm>,
) -> Handled<Redirect> {
    if state.store.get_work(&form.work_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    state
        .store
        .save_progress(&form.work_id, form.chapter)
        .await?;
    Ok(Redirect::to(&format!(
        "/read/{}?chapter={}",
        form.work_id, form.chapter
    )))
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionQuery {
    error: Option<String>,
    ok: Option<String>,
}

async fn session_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionQuery>,
) -> Handled<Html<String>> {
    let session = state.session.lock().await;
    Ok(Html(render::session_page(
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

async fn persist_session(state: &AppState, session: &AccountSession) -> Result<()> {
    let encrypted = session.jar().encrypt(&state.key)?;
    state
        .store
        .save_session(session.account(), &encrypted)
        .await
}

#[derive(Deserialize)]
struct ImportForm {
    account: String,
    session_cookie: String,
    credentials_cookie: String,
    adult: Option<String>,
}

async fn session_import(
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
}

#[derive(Deserialize)]
struct LoginForm {
    account: String,
    password: String,
    adult: Option<String>,
}

async fn session_login(
    State(state): State<Arc<AppState>>,
    Form(form): Form<LoginForm>,
) -> Handled<Redirect> {
    let account = form.account.trim().to_string();
    if account.is_empty() {
        return Err(AppError::BadRequest("AO3 login is empty".into()));
    }
    let mut session = state.session.lock().await;
    match session
        .login(&account, SecretString::new(form.password))
        .await
    {
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
}

async fn session_probe(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
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
}

async fn session_adult(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
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

async fn session_logout(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    let mut session = state.session.lock().await;
    let logout = session.logout().await;
    state.store.delete_session().await.map_err(AppError::from)?;
    match logout {
        Ok(_) => Ok(redirect_session("ok", "Logged out of AO3.")),
        Err(error) => Ok(redirect_session(
            "error",
            &format!("Local session cleared, but upstream logout failed: {error}"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Writes (W4): confirmation -> governor -> intent -> one request -> audit
// ---------------------------------------------------------------------------

fn notice_text(key: &str) -> Option<&'static str> {
    match key {
        "subscribed" => Some("Subscribed on AO3."),
        "unsubscribed" => Some("Unsubscribed on AO3."),
        "already_subscribed" => Some("Already subscribed according to the local ledger."),
        "not_subscribed" => Some("No active subscription in the local ledger."),
        "write_failed" => Some("AO3 rejected the action; see the audit log."),
        "write_rate_limited" => Some("Rate limited by the client governor; try again later."),
        "write_disabled" => Some("Writes are disabled (set AO3I_WRITE_ENABLED=true)."),
        "kudos_given" => Some("Kudos given."),
        "kudos_already" => Some("You had already left kudos on this work."),
        "kudos_failed" => Some("AO3 refused the kudos; see the audit log."),
        "reading_for_later" => Some("Marked for later."),
        "reading_read" => Some("Marked as read."),
        "reading_failed" => Some("AO3 refused the reading update; see the audit log."),
        _ => None,
    }
}

async fn session_identity(state: &AppState) -> (String, bool) {
    let session = state.session.lock().await;
    (session.account().to_string(), session.state().is_usable())
}

async fn redirect_after_write(
    state: &AppState,
    kind: &str,
    target_id: &str,
    flag: &str,
) -> Redirect {
    if kind == actions::WORK
        && state
            .store
            .get_work(target_id)
            .await
            .ok()
            .flatten()
            .is_some()
    {
        Redirect::to(&format!("/read/{target_id}?notice={flag}"))
    } else {
        Redirect::to(&format!("/?notice={flag}"))
    }
}

#[derive(Deserialize)]
struct PreviewQuery {
    kind: Option<String>,
    id: Option<String>,
}

async fn write_preview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PreviewQuery>,
) -> Handled<Html<String>> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden(
            "Sign in to AO3 on the Session page before writing.".into(),
        ));
    }
    let raw_kind = query.kind.unwrap_or_default();
    let target_id = query.id.unwrap_or_default().trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing write target".into()));
    }
    if raw_kind.trim().eq_ignore_ascii_case("kudos") {
        let description = match state.store.get_work(&target_id).await? {
            Some(work) => format!("Leave kudos on “{}” on AO3?", work.title),
            None => format!("Leave kudos on work {target_id} on AO3?"),
        };
        return Ok(Html(render::confirm_page(
            "Confirm kudos",
            &description,
            "/write/kudos",
            &[("target_id", &target_id), ("confirm", "yes")],
        )));
    }
    let kind = actions::normalize_kind(&raw_kind)
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let description = if kind == actions::WORK {
        match state.store.get_work(&target_id).await? {
            Some(work) => format!("Subscribe to “{}” on AO3?", work.title),
            None => format!("Subscribe to work {target_id} on AO3?"),
        }
    } else {
        format!("Subscribe to {kind} {target_id} on AO3?")
    };
    Ok(Html(render::confirm_page(
        "Confirm subscription",
        &description,
        "/write/subscribe",
        &[
            ("kind", kind),
            ("target_id", &target_id),
            ("confirm", "yes"),
        ],
    )))
}

#[derive(Deserialize)]
struct WriteForm {
    kind: String,
    target_id: String,
    confirm: Option<String>,
}

async fn write_subscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<WriteForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    if form.confirm.as_deref() != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    let kind = form
        .kind
        .as_str()
        .pipe_kind()
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing subscription target".into()));
    }

    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
    if let Some(row) = state
        .store
        .get_subscription(&account, kind, &target_id)
        .await?
        && row.state == "active"
    {
        return Ok(redirect_after_write(&state, kind, &target_id, "already_subscribed").await);
    }

    state
        .governor
        .begin("subscribe")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "subscribe", kind, &target_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::subscribe(&mut session, &account, kind, &target_id).await
    };
    match result {
        Ok(outcome) => {
            state
                .store
                .upsert_subscription(&account, kind, &target_id, outcome.item_id, "active")
                .await?;
            state
                .store
                .finish_intent(intent, "done", "subscribed", outcome.message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "subscribed").await)
        }
        Err(error) => {
            let retry_after = if let Error::RateLimited { retry_after } = &error {
                *retry_after
            } else {
                None
            };
            state
                .governor
                .record_outcome("subscribe", retry_after)
                .await;
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            let flag = if retry_after.is_some() {
                "write_rate_limited"
            } else {
                "write_failed"
            };
            Ok(redirect_after_write(&state, kind, &target_id, flag).await)
        }
    }
}

async fn write_unsubscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<WriteForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let kind = form
        .kind
        .as_str()
        .pipe_kind()
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let target_id = form.target_id.trim().to_string();
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
    let Some(row) = state
        .store
        .get_subscription(&account, kind, &target_id)
        .await?
    else {
        return Ok(redirect_after_write(&state, kind, &target_id, "not_subscribed").await);
    };
    if row.state != "active" {
        return Ok(redirect_after_write(&state, kind, &target_id, "not_subscribed").await);
    }
    let Some(subscription_id) = row.subscription_id else {
        return Ok(redirect_after_write(&state, kind, &target_id, "not_subscribed").await);
    };

    state
        .governor
        .begin("unsubscribe")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "unsubscribe", kind, &target_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::unsubscribe(&mut session, &account, subscription_id).await
    };
    match result {
        Ok(message) => {
            state
                .store
                .upsert_subscription(&account, kind, &target_id, None, "removed")
                .await?;
            state
                .store
                .finish_intent(intent, "done", "unsubscribed", message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "unsubscribed").await)
        }
        Err(error) => {
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "write_failed").await)
        }
    }
}

#[derive(Deserialize)]
struct KudosForm {
    target_id: String,
    confirm: Option<String>,
}

async fn write_kudos(
    State(state): State<Arc<AppState>>,
    Form(form): Form<KudosForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    if form.confirm.as_deref() != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing kudos target".into()));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
    if state.store.has_kudos(&account, &target_id).await? {
        return Ok(redirect_after_write(&state, actions::WORK, &target_id, "kudos_already").await);
    }

    state
        .governor
        .begin("kudos")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "kudos", actions::WORK, &target_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::kudos(&mut session, &target_id).await
    };
    match result {
        Ok(outcome) if outcome.already => {
            state.store.mark_kudos(&account, &target_id).await?;
            state
                .store
                .finish_intent(intent, "done", "already-done", outcome.message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "kudos_already").await)
        }
        Ok(_) => {
            state.store.mark_kudos(&account, &target_id).await?;
            state
                .store
                .finish_intent(intent, "done", "kudos-given", None)
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "kudos_given").await)
        }
        Err(error) => {
            let retry_after = if let Error::RateLimited { retry_after } = &error {
                *retry_after
            } else {
                None
            };
            state.governor.record_outcome("kudos", retry_after).await;
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "kudos_failed").await)
        }
    }
}

#[derive(Deserialize)]
struct ReadingForm {
    target_id: String,
    action: String,
}

async fn write_reading(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ReadingForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let for_later = match form.action.trim() {
        "later" => true,
        "read" => false,
        _ => return Err(AppError::BadRequest("unknown reading action".into())),
    };
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing reading target".into()));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }

    state
        .governor
        .begin("reading")
        .await
        .map_err(AppError::from)?;
    let action = if for_later {
        "reading_for_later"
    } else {
        "reading_read"
    };
    let intent = state
        .store
        .record_intent(&account, action, actions::WORK, &target_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::set_reading_flag(&mut session, &target_id, for_later).await
    };
    match result {
        Ok(()) => {
            state
                .store
                .set_reading_state(
                    &account,
                    &target_id,
                    if for_later { "for_later" } else { "read" },
                )
                .await?;
            state
                .store
                .finish_intent(intent, "done", action, None)
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, action).await)
        }
        Err(error) => {
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "reading_failed").await)
        }
    }
}

async fn audit(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let entries = state.store.recent_audit(100).await?;
    Ok(Html(render::audit_page(&entries)))
}

async fn health() -> &'static str {
    "ok"
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `&str` → normalized subscription kind (form-friendly).
trait KindStr {
    fn pipe_kind(self) -> Option<&'static str>;
}

impl KindStr for &str {
    fn pipe_kind(self) -> Option<&'static str> {
        actions::normalize_kind(self)
    }
}

/// AO3's adult interstitial has no work title, so the parser reports drift; the
/// gate markers let us say "enable adult content" instead of "AO3 changed".
fn looks_like_adult_gate(html: &str) -> bool {
    html.contains("This work could have adult content")
        || html.contains("id=\"adult\"")
        || html.contains("Proceed")
}

/// Restricted works redirect to the login page; the marker check covers the
/// rendered login form some paths serve with 200.
fn looks_like_login_gate(html: &str) -> bool {
    html.contains("This work is only available to registered users")
        || (html.contains("id=\"loginform\"") && !html.contains("class=\"title heading\""))
}

/// Accepts a bare id or any URL containing `/works/<id>`.
fn normalize_work_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }
    let marker = "/works/";
    let start = trimmed.find(marker)? + marker.len();
    let digits: String = trimmed[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_ids_are_normalised_from_ids_and_urls() {
        assert_eq!(normalize_work_id("12345").as_deref(), Some("12345"));
        assert_eq!(
            normalize_work_id("https://archiveofourown.org/works/12345").as_deref(),
            Some("12345")
        );
        assert_eq!(
            normalize_work_id(" https://archiveofourown.org/works/12345?view_full_work=true ")
                .as_deref(),
            Some("12345")
        );
        assert_eq!(
            normalize_work_id("https://archiveofourown.org/works/12345/chapters/1").as_deref(),
            Some("12345")
        );
        assert_eq!(normalize_work_id(""), None);
        assert_eq!(normalize_work_id("   "), None);
        assert_eq!(normalize_work_id("not a work"), None);
        assert_eq!(
            normalize_work_id("https://archiveofourown.org/tags/x/works"),
            None
        );
    }

    #[test]
    fn cookie_values_reject_separators() {
        assert!(cookie_value("abc123--def").is_ok());
        assert!(cookie_value("").is_err());
        assert!(cookie_value("a b").is_err());
        assert!(cookie_value("a;b").is_err());
        assert!(cookie_value("a\r\nb").is_err());
    }

    #[test]
    fn gate_markers_are_recognised() {
        let adult = "<div id=\"adult\">This work could have adult content. Proceed</div>";
        assert!(looks_like_adult_gate(adult));
        assert!(!looks_like_login_gate(adult));
        let login = "<div id=\"loginform\">This work is only available to registered users</div>";
        assert!(looks_like_login_gate(login));
    }
}
