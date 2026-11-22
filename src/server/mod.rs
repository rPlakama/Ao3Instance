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
mod reads;
mod session;
mod writes;

use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};
use tokio::sync::Mutex;

use crate::ao3::governor::WriteGovernor;
use crate::ao3::jar::Jar;
use crate::ao3::pseuds::Pseud;
use crate::ao3::session::AccountSession;
use crate::config::Config;
use crate::crypto::SecretKey;
use crate::db;
use crate::diag::HttpErrorEvent;
use crate::error::{Error, Result};
use crate::store::Store;

pub struct AppState {
    pub store: Store,
    pub session: Mutex<AccountSession>,
    pub key: SecretKey,
    pub governor: WriteGovernor,
    pub write_enabled: bool,
    /// Inbound 4xx/5xx events join the same queue as upstream client failures.
    pub diag_tx: tokio::sync::mpsc::Sender<HttpErrorEvent>,
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
    });
    let app = Router::new()
        .route("/", get(reads::home))
        .route("/open", post(reads::open_work))
        .route("/read/{id}", get(reads::read_work))
        .route("/read/{id}/chapter/{idx}", get(reads::read_chapter))
        .route("/search", get(reads::search))
        .route("/progress", post(reads::save_progress))
        .route("/session", get(session::session_page))
        .route("/session/import", post(session::session_import))
        .route("/session/login", post(session::session_login))
        .route("/session/probe", post(session::session_probe))
        .route("/session/adult", post(session::session_adult))
        .route("/session/logout", post(session::session_logout))
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

/// The account and whether its session can act, for read/write handlers.
pub(super) async fn session_identity(state: &AppState) -> (String, bool) {
    let session = state.session.lock().await;
    (session.account().to_string(), session.state().is_usable())
}
