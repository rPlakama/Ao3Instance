//! The web application: sign in to AO3, open a work, cache it, read it offline.
//!
//! Server-rendered on purpose (`docs/write-client-plan.md` D5): no SPA, no client
//! state to desynchronize. The stateful things are the SQLite cache, the reading
//! position and one persisted AO3 session (encrypted jar).
//!
//! The route handlers live in three segments — reads, session, writes — and the
//! router below is the only place that knows their URLs.

mod diag;
mod error;
mod lists;
mod reads;
mod session;
mod solver;
mod writes;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tokio::sync::Mutex;

use crate::ao3::governor::WriteGovernor;
use crate::ao3::jar::Jar;
use crate::ao3::pseuds::Pseud;
use crate::ao3::session::AccountSession;
use crate::ao3::solver::SolverStatus;
use crate::config::Config;
use crate::crypto::AAD_SOLVER;
use crate::crypto::SecretKey;
use crate::db;
use crate::diag::HttpErrorEvent;
use crate::error::{Error, Result};
use crate::render;
use crate::render::{NavSection, PageContext, SolverSummary};
use crate::store::Store;

pub struct AppState {
    pub store: Store,
    pub session: Mutex<AccountSession>,
    pub key: SecretKey,
    pub governor: WriteGovernor,
    pub write_enabled: bool,
    /// Inbound 4xx/5xx events join the same queue as upstream client failures.
    pub diag_tx: tokio::sync::mpsc::Sender<HttpErrorEvent>,
    /// Last solver probe result, refreshed by `/diagnostics/solver/probe`.
    pub solver_status: tokio::sync::RwLock<SolverStatus>,
    /// Single-flight guard: one solve (and one Chromium) at a time.
    pub solver_lock: tokio::sync::Mutex<()>,
    /// Global display preference (`default` or `reversi`).
    pub skin: tokio::sync::RwLock<String>,
    /// One solver-fetched search page awaiting its redirect, short-lived.
    pub solved_search: tokio::sync::Mutex<Option<SolvedSearch>>,
}

pub struct SolvedSearch {
    pub html: String,
    pub at: i64,
}

/// Builds the router. Split from [`run`] so tests can serve it on an ephemeral
/// port. Loads the persisted session (without probing it).
pub async fn router(config: &Config, store: Store) -> Result<Router> {
    let key = SecretKey::load_or_create(&config.key_file)?;
    let mut session = AccountSession::new(config)?;

    // One bounded queue for both directions; the writer task owns SQLite.
    let (diag_tx, diag_rx) = diag::channel();
    diag::spawn_writer(
        store.clone(),
        config.http_log_retention,
        config.log_http_errors,
        diag_rx,
    );
    session
        .client_mut()
        .set_error_sink(diag::sink(diag_tx.clone()));

    // Restore a still-valid Cloudflare clearance, if one was persisted.
    if let Ok(Some(saved)) = store.load_solver_clearance().await
        && saved.expires_at > crate::time::now_secs()
    {
        match key.decrypt(AAD_SOLVER, &saved.ciphertext) {
            Ok(cookie_header) => {
                session.restore_clearance(&saved.user_agent, &cookie_header, saved.expires_at);
            }
            Err(error) => {
                eprintln!("stored solver clearance could not be decrypted: {error}");
                if let Err(error) = store.clear_solver_clearance().await {
                    eprintln!("could not clear solver clearance: {error}");
                }
            }
        }
    }
    let solver_status = session.probe_solver().await;
    let saved_skin = store
        .get_setting("display.skin")
        .await
        .ok()
        .flatten()
        .filter(|skin| matches!(skin.as_str(), "auto" | "default" | "reversi"))
        .unwrap_or_else(|| "default".to_string());

    if let Some(saved) = store.load_session().await? {
        match Jar::decrypt(&key, &saved.jar_enc) {
            Ok(jar) => match session.restore(&saved.account, jar) {
                Ok(state) => {
                    if let (Some(id), Some(name)) = (saved.pseud_id, saved.pseud_name) {
                        session.set_pseud(Pseud { id, name });
                    }
                    eprintln!("session restored for {} ({state})", saved.account);
                }
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
        diag_tx,
        solver_status: tokio::sync::RwLock::new(solver_status),
        solver_lock: tokio::sync::Mutex::new(()),
        skin: tokio::sync::RwLock::new(saved_skin),
        solved_search: tokio::sync::Mutex::new(None),
    });
    let app = Router::new()
        .route("/", get(reads::home))
        .route("/open", post(reads::open_work))
        .route("/open/{id}", get(reads::open_and_read))
        .route("/read/{id}", get(reads::read_work))
        .route("/read/{id}/chapter/{idx}", get(reads::read_chapter))
        .route("/search", get(reads::search).post(reads::search_post))
        .route("/progress", post(reads::save_progress))
        .route("/progress/auto", post(reads::auto_progress))
        .route("/session", get(session::session_page))
        .route("/session/import", post(session::session_import))
        .route("/session/login", post(session::session_login))
        .route("/session/probe", post(session::session_probe))
        .route("/session/adult", post(session::session_adult))
        .route("/session/logout", post(session::session_logout))
        .route("/users/logout/confirm", get(session::logout_confirm))
        .route("/write/preview", get(writes::write_preview))
        .route("/write/subscribe", post(writes::write_subscribe))
        .route("/write/unsubscribe", post(writes::write_unsubscribe))
        .route("/write/kudos", post(writes::write_kudos))
        .route("/write/reading", post(writes::write_reading))
        .route(
            "/write/comment/preview",
            post(writes::write_comment_preview),
        )
        .route("/write/comment", post(writes::write_comment))
        .route("/write/bookmark", post(writes::write_bookmark))
        .route("/audit", get(writes::audit))
        .route("/users/login", get(session::session_page))
        .route("/works", get(lists::browse))
        .route("/works/search", get(reads::search).post(reads::search_post))
        .route("/tags/search", get(lists::tag_search))
        .route("/tags/{tag}/works", get(lists::tag_browse))
        .route("/fandoms", get(lists::fandoms))
        .route("/about", get(lists::about))
        .route("/dashboard", get(lists::dashboard))
        .route("/subscriptions", get(lists::subscriptions))
        .route("/bookmarks", get(lists::bookmarks))
        .route("/reading", get(lists::reading))
        .route("/history", get(lists::history))
        .route("/works/{id}", get(reads::work_alias))
        .route("/works/{id}/chapters/{idx}", get(reads::chapter_alias))
        .route("/assets/ao3.css", get(stylesheet))
        .route("/assets/reader.js", get(reader_js))
        .route("/favicon.ico", get(favicon))
        .route("/skin", post(session::skin))
        .route("/solve", post(solver::solve))
        .route("/diagnostics", get(solver::diagnostics))
        .route("/diagnostics/clear", post(solver::clear_logs))
        .route("/diagnostics/solver/probe", post(solver::probe))
        .route(
            "/diagnostics/solver/clearance",
            post(solver::clear_clearance),
        )
        .route("/health", get(health))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, diag::inbound));
    Ok(app)
}

/// Opens the database and serves until Ctrl-C.
pub async fn run(config: &Config) -> Result<()> {
    let pool = db::open(&config.db_file).await?;
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

async fn health() -> &'static str {
    "ok"
}

/// Browsers request this unprompted; answering cheaply keeps the diagnostics
/// table free of meaningless 404 rows.
async fn favicon() -> axum::http::StatusCode {
    axum::http::StatusCode::NO_CONTENT
}

/// Serves the embedded stylesheet with conservative caching and nosniff.
async fn reader_js() -> Response {
    let mut response = render::READER_JS.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn stylesheet() -> Response {
    let mut response = render::AO3_CSS.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Chrome data shared by every page: session greeting, cache count and solver
/// state. Read-only; the per-request skin preference comes later.
pub(super) async fn page_context(state: &AppState, title: &str, active: NavSection) -> PageContext {
    let (account, session_state, clearance_active) = {
        let session = state.session.lock().await;
        (
            session.account().to_string(),
            session.state().to_string(),
            session.clearance_snapshot().is_some(),
        )
    };
    let cached_count = state.store.count_works().await.unwrap_or(0);
    let skin = state.skin.read().await.clone();
    let solver = {
        let status = state.solver_status.read().await;
        SolverSummary {
            mode: status.mode.to_string(),
            reachable: status.reachable,
            clearance_active,
        }
    };
    PageContext::new(title, active)
        .with_session(account, session_state)
        .with_write_enabled(state.write_enabled)
        .with_cached_count(cached_count)
        .with_solver(solver)
        .with_skin(skin)
}

/// The account and whether its session can act, for read/write handlers.
pub(super) async fn session_identity(state: &AppState) -> (String, bool) {
    let session = state.session.lock().await;
    (session.account().to_string(), session.state().is_usable())
}
