//! The web application: open a work, cache it, read it offline.
//!
//! Server-rendered on purpose (`docs/write-client-plan.md` D5): no SPA, no client
//! state to desynchronize. The only stateful thing is the SQLite cache.

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

use crate::ao3::client::Client;
use crate::ao3::work::parse_work_page;
use crate::config::Config;
use crate::error::{Error, Result};

/// Handler result: an `AppError` renders a full HTML error page.
type Handled<T> = std::result::Result<T, AppError>;
use crate::render;
use crate::store::{Store, sha256_hex};

/// CSP for the chapter iframe: no scripts, no forms, inline styles only, images
/// from https only. Author content never runs in the app's origin.
const CHAPTER_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; \
     font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

pub struct AppState {
    pub store: Store,
    pub client: Mutex<Client>,
    pub base_url: String,
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
            Error::LoggedOut
            | Error::SessionIncomplete
            | Error::CsrfMissing
            | Error::CsrfExpired
            | Error::LoginRejected(_)
            | Error::PasswordChangeRequired
            | Error::LogoutFailed(_) => {
                Self::Upstream("The AO3 session is not usable; re-authenticate.".to_string())
            }
            Error::Killed => Self::Internal("Upstream traffic is disabled by the operator".into()),
        }
    }
}

/// Builds the router. Split from [`run`] so tests can serve it on an ephemeral port.
pub fn router(config: &Config, store: Store) -> Result<Router> {
    let client = Client::with_spacing(
        &config.base_url,
        &config.user_agent,
        config.min_request_spacing,
    )?;
    let state = Arc::new(AppState {
        store,
        client: Mutex::new(client),
        base_url: config.base_url.clone(),
    });
    Ok(Router::new()
        .route("/", get(home))
        .route("/open", post(open_work))
        .route("/read/{id}", get(read_work))
        .route("/read/{id}/chapter/{idx}", get(read_chapter))
        .route("/health", get(health))
        .with_state(state))
}

/// Opens the database and serves until Ctrl-C.
pub async fn run(config: &Config) -> Result<()> {
    let pool = crate::db::open(&config.db_file).await?;
    let store = Store::new(pool);
    let app = router(config, store)?;
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
    let mut page = parse_work_page(&html).map_err(AppError::from)?;
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
    let mut client = state.client.lock().await;
    let response = client.get(&path).await.map_err(AppError::from)?;
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
            "This work needs a signed-in session (restricted work). Account support is the next milestone.".into(),
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
    let current = query
        .chapter
        .unwrap_or_else(|| chapters.first().map(|chapter| chapter.idx).unwrap_or(1));
    Ok(Html(render::read_page(&work, &chapters, current)))
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

async fn health() -> &'static str {
    "ok"
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
}
