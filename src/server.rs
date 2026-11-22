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
}

enum AppError {
    BadRequest(String),
    NotFound,
    Upstream(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
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
    });
    Ok(Router::new()
        .route("/", get(home))
        .route("/open", post(open_work))
        .route("/read/{id}", get(read_work))
        .route("/read/{id}/chapter/{idx}", get(read_chapter))
        .route("/progress", post(save_progress))
        .route("/session", get(session_page))
        .route("/session/import", post(session_import))
        .route("/session/login", post(session_login))
        .route("/session/probe", post(session_probe))
        .route("/session/adult", post(session_adult))
        .route("/session/logout", post(session_logout))
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

async fn home(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let works = state.store.list_works(50).await?;
    Ok(Html(render::home_page(&works, None)))
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
    // Cache first: re-opening a cached work is an offline read, not a fetch.
    if form.refresh.is_none() && state.store.get_work(&id).await?.is_some() {
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
    state
        .store
        .upsert_work(&page, &sha256_hex(html.as_bytes()))
        .await?;
    Ok(Redirect::to(&format!("/read/{id}")))
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
    Ok(Html(render::read_page(&work, &chapters, current, progress)))
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

async fn health() -> &'static str {
    "ok"
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
