//! The web application: sign in to AO3, open a work, cache it, read it offline.
//!
//! Server-rendered on purpose (`docs/write-client-plan.md` D5): no SPA, no client
//! state to desynchronize. The stateful things are the SQLite cache, the reading
//! position and one persisted AO3 session (encrypted jar).
//!
//! The route handlers live in three segments — reads, session, writes — and the
//! router below is the only place that knows their URLs.

mod cache;
mod diag;
mod error;
mod lists;
mod reads;
mod session;
mod solver;
mod themes;
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

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub account: String,
    pub state: String,
    pub usable: bool,
    pub jar_complete: bool,
    pub adult_enabled: bool,
    pub clearance_active: bool,
    pub pseud_name: Option<String>,
}

impl SessionSnapshot {
    pub(super) fn from_session(session: &AccountSession) -> Self {
        Self {
            account: session.account().to_string(),
            state: session.state().to_string(),
            usable: session.state().is_usable(),
            jar_complete: session.jar().is_complete(),
            adult_enabled: session.jar().contains("view_adult"),
            clearance_active: session.clearance_snapshot().is_some(),
            pseud_name: session.pseud().map(|pseud| pseud.name.clone()),
        }
    }
}

pub struct AppState {
    pub store: Store,
    pub session: Mutex<AccountSession>,
    /// Lock-free read snapshot of session chrome state so offline page renders
    /// never block behind slow upstream or FlareSolverr I/O.
    pub session_snapshot: tokio::sync::RwLock<SessionSnapshot>,
    pub key: SecretKey,
    pub governor: WriteGovernor,
    pub write_enabled: bool,
    /// Inbound 4xx/5xx events join the same queue as upstream client failures.
    pub diag_tx: tokio::sync::mpsc::Sender<HttpErrorEvent>,
    /// Last solver probe result, refreshed by `/diagnostics/solver/probe`.
    pub solver_status: tokio::sync::RwLock<SolverStatus>,
    /// Single-flight guard: one solve (and one Chromium) at a time.
    pub solver_lock: tokio::sync::Mutex<()>,
    /// Global display preference (built-in theme or `public`).
    pub skin: tokio::sync::RwLock<String>,
    /// Custom CSS imported from an AO3 public site skin.
    pub skin_css: tokio::sync::RwLock<Option<String>>,
    /// One solver-fetched search page awaiting its redirect, short-lived.
    pub solved_search: tokio::sync::Mutex<Option<SolvedSearch>>,
    /// Options parsed from AO3's `/works/new` form, reused briefly by the
    /// composer (language ids and creator pseuds are only known from that page).
    pub compose_defaults:
        tokio::sync::RwLock<Option<(std::time::Instant, crate::ao3::compose::NewWorkDefaults)>>,
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
        .filter(|skin| crate::consts::is_stored_skin(skin))
        .unwrap_or_else(|| "default".to_string());
    let saved_skin_css = store
        .get_setting("display.skin_css")
        .await
        .ok()
        .flatten()
        .filter(|css| !css.trim().is_empty());

    let mut restored_imported = false;
    // A stored jar that cannot authenticate is useless and must never shadow a
    // future import: clear it instead of restoring it on every start. The account
    // name is kept for this run so the session page can explain what happened.
    let mut unusable_stored_session = false;
    if let Some(saved) = store.load_session().await? {
        match Jar::decrypt(&key, &saved.jar_enc) {
            Ok(jar) => match session.restore(&saved.account, jar) {
                Ok(state) => {
                    if let (Some(id), Some(name)) = (saved.pseud_id, saved.pseud_name) {
                        session.set_pseud(Pseud { id, name });
                    }
                    if matches!(
                        state,
                        crate::ao3::session::SessionState::SessionIncomplete
                            | crate::ao3::session::SessionState::LoggedOut
                    ) {
                        unusable_stored_session = true;
                        eprintln!(
                            "stored session for {} cannot authenticate ({state}); cleared, re-import required",
                            saved.account
                        );
                    } else {
                        restored_imported = state == crate::ao3::session::SessionState::Imported;
                        eprintln!("session restored for {} ({state})", saved.account);
                    }
                }
                Err(error) => {
                    unusable_stored_session = true;
                    eprintln!("stored session refused ({error}); cleared, re-import required");
                }
            },
            Err(error) => {
                unusable_stored_session = true;
                eprintln!(
                    "stored session could not be decrypted ({error}); cleared, re-import required"
                );
            }
        }
    }
    if unusable_stored_session && let Err(error) = store.delete_session().await {
        eprintln!("could not clear unusable stored session: {error}");
    }

    let initial_snapshot = SessionSnapshot::from_session(&session);
    let state = Arc::new(AppState {
        store,
        session: Mutex::new(session),
        session_snapshot: tokio::sync::RwLock::new(initial_snapshot),
        key,
        governor: WriteGovernor::new(config.min_request_spacing),
        write_enabled: config.write_enabled,
        diag_tx,
        solver_status: tokio::sync::RwLock::new(solver_status),
        solver_lock: tokio::sync::Mutex::new(()),
        skin: tokio::sync::RwLock::new(saved_skin),
        skin_css: tokio::sync::RwLock::new(saved_skin_css),
        solved_search: tokio::sync::Mutex::new(None),
        compose_defaults: tokio::sync::RwLock::new(None),
    });
    if restored_imported {
        let bg_state = Arc::clone(&state);
        tokio::spawn(async move {
            let result = {
                let _solver_guard = bg_state.solver_lock.lock().await;
                let mut session = bg_state.session.lock().await;
                let res = session.ensure_active().await;
                sync_session_snapshot(&bg_state, &session).await;
                res
            };
            match result {
                Ok(new_state) => {
                    session::persist_if_complete(&bg_state).await;
                    solver::sync_solver_state(&bg_state).await;
                    eprintln!("restored session verified on startup ({new_state})");
                }
                Err(error) => {
                    eprintln!("restored session startup check deferred: {error}");
                }
            }
        });
    }
    let app = Router::new()
        .route("/", get(reads::home))
        .route("/open", post(reads::open_work))
        .route("/open/{id}", get(reads::open_and_read))
        .route("/read/{id}", get(reads::read_work))
        .route("/read/{id}/comments", post(reads::load_comments))
        .route("/read/{id}/chapter/{idx}", get(reads::read_chapter))
        .route("/search", get(reads::search).post(reads::search_post))
        .route("/progress", post(reads::save_progress))
        .route("/progress/auto", post(reads::auto_progress))
        .route("/session", get(session::session_page))
        .route("/session/import", post(session::session_import))
        .route("/session/import-file", post(session::session_import_file))
        .route("/session/login", post(session::session_login))
        .route("/session/probe", post(session::session_probe))
        .route("/session/adult", post(session::session_adult))
        .route("/session/logout", post(session::session_logout))
        .route("/users/logout/confirm", get(session::logout_confirm))
        .route("/works/new", get(writes::compose))
        .route("/write/work/preview", post(writes::write_work_preview))
        .route("/write/work", post(writes::write_work))
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
        .route("/autocomplete/{kind}", get(lists::autocomplete))
        .route("/fandoms", get(lists::fandoms))
        .route("/media/{medium}/fandoms", get(lists::medium_fandoms))
        .route("/about", get(lists::about))
        .route("/dashboard", get(lists::dashboard))
        .route("/subscriptions", get(lists::subscriptions))
        .route("/bookmarks", get(lists::bookmarks))
        .route("/reading", get(lists::reading))
        .route("/history", get(lists::history))
        .route("/works/{id}", get(reads::work_alias))
        .route("/works/{id}/chapters/{idx}", get(reads::chapter_alias))
        .route("/assets/ao3.css", get(stylesheet))
        .route("/assets/ui.js", get(ui_js))
        .route("/assets/reader.js", get(reader_js))
        .route("/favicon.ico", get(favicon))
        .route("/skin", post(session::skin))
        .route("/themes", get(themes::themes))
        .route("/themes/apply", post(themes::apply))
        .route("/themes/clear", post(themes::clear))
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
        .layer(middleware::from_fn(same_origin_guard))
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

/// Serves embedded static JavaScript with conservative caching and nosniff.
async fn ui_js() -> Response {
    static_asset(render::UI_JS, "text/javascript; charset=utf-8")
}

async fn reader_js() -> Response {
    static_asset(render::READER_JS, "text/javascript; charset=utf-8")
}

async fn stylesheet() -> Response {
    static_asset(render::AO3_CSS, "text/css; charset=utf-8")
}

fn static_asset(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
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

/// Rejects cross-site mutating requests (`Sec-Fetch-Site` / `Origin`) from
/// browsers while allowing same-origin navigation and non-browser clients
/// (`docs/security-standards.md` §7, T2).
async fn same_origin_guard(request: axum::extract::Request, next: middleware::Next) -> Response {
    if matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        return next.run(request).await;
    }
    let headers = request.headers();
    if let Some(site) = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        && !matches!(site, "same-origin" | "none")
    {
        return (
            axum::http::StatusCode::FORBIDDEN,
            axum::response::Html(render::error_page(
                403,
                "Cross-site request blocked (Sec-Fetch-Site must be same-origin).",
            )),
        )
            .into_response();
    }
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
    {
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::trim);
        if !origin_matches_host(origin, host) {
            return (
                axum::http::StatusCode::FORBIDDEN,
                axum::response::Html(render::error_page(
                    403,
                    "Cross-site request blocked (Origin does not match Host).",
                )),
            )
                .into_response();
        }
    }
    next.run(request).await
}

fn origin_matches_host(origin: &str, host: Option<&str>) -> bool {
    let Some(host) = host.filter(|h| !h.is_empty()) else {
        return false;
    };
    let (scheme, authority) = if let Some(rest) = origin.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = origin.strip_prefix("https://") {
        ("https", rest)
    } else {
        return false;
    };
    let authority = authority.trim_end_matches('/');
    if authority.eq_ignore_ascii_case(host) {
        return true;
    }
    let default_port = if scheme == "https" { ":443" } else { ":80" };
    authority
        .strip_suffix(default_port)
        .is_some_and(|stripped| stripped.eq_ignore_ascii_case(host))
        || host
            .strip_suffix(default_port)
            .is_some_and(|stripped| authority.eq_ignore_ascii_case(stripped))
}

/// Updates the lock-free session snapshot from an already-locked session.
pub(super) async fn sync_session_snapshot(state: &AppState, session: &AccountSession) {
    *state.session_snapshot.write().await = SessionSnapshot::from_session(session);
}

/// Chrome data shared by every page: session greeting, cache count and solver
/// state. Reads the lock-free snapshot so offline pages never stall behind
/// upstream or FlareSolverr I/O.
pub(super) async fn page_context(state: &AppState, title: &str, active: NavSection) -> PageContext {
    let snap = state.session_snapshot.read().await.clone();
    let cached_count = state.store.count_works().await.unwrap_or(0);
    let skin = state.skin.read().await.clone();
    let skin_css = state.skin_css.read().await.clone();
    let solver = {
        let status = state.solver_status.read().await;
        SolverSummary {
            mode: status.mode.to_string(),
            reachable: status.reachable,
            clearance_active: snap.clearance_active,
        }
    };
    PageContext::new(title, active)
        .with_session(snap.account, snap.state)
        .with_write_enabled(state.write_enabled)
        .with_cached_count(cached_count)
        .with_solver(solver)
        .with_skin(skin)
        .with_skin_css(skin_css)
}

/// The account and whether its session can act, for read/write handlers.
pub(super) async fn session_identity(state: &AppState) -> (String, bool) {
    let snap = state.session_snapshot.read().await;
    (snap.account.clone(), snap.usable)
}

#[cfg(test)]
mod tests {
    use super::origin_matches_host;

    #[test]
    fn origin_host_matching_allows_same_origin_and_rejects_cross_origin() {
        assert!(origin_matches_host(
            "http://127.0.0.1:8099",
            Some("127.0.0.1:8099")
        ));
        assert!(origin_matches_host(
            "https://ao3.example.test",
            Some("ao3.example.test")
        ));
        assert!(origin_matches_host(
            "https://ao3.example.test:443",
            Some("ao3.example.test")
        ));
        assert!(!origin_matches_host(
            "https://evil.example",
            Some("127.0.0.1:8099")
        ));
        assert!(!origin_matches_host("null", Some("127.0.0.1:8099")));
        assert!(!origin_matches_host("http://127.0.0.1:8099", None));
    }
}
