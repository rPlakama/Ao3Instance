//! Diagnostic plumbing for the web app: one bounded channel carries upstream
//! client events and inbound 4xx/5xx responses to a single writer task.
//!
//! The client callback is synchronous, so it can only enqueue; the writer task
//! is the one place that touches SQLite and stderr. Saturation drops the newest
//! event by policy (`init-audit.md` A7) — diagnostics must never wedge a
//! request.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::mpsc;

use super::AppState;
use crate::ao3::client::ErrorSink;
use crate::diag::{self, HttpDirection, HttpErrorEvent, Transport};
use crate::store::Store;

/// Bounded queue shared by the client sink and the inbound middleware.
pub const CHANNEL_CAPACITY: usize = 256;

pub fn channel() -> (mpsc::Sender<HttpErrorEvent>, mpsc::Receiver<HttpErrorEvent>) {
    mpsc::channel(CHANNEL_CAPACITY)
}

/// The client sink: enqueue and return; the request path never awaits SQLite.
pub fn sink(tx: mpsc::Sender<HttpErrorEvent>) -> ErrorSink {
    Arc::new(move |event: &HttpErrorEvent| {
        // Full or closed: drop newest. Losing a diagnostic is better than
        // stalling a user request.
        drop(tx.try_send(event.clone()));
    })
}

/// Drains events into SQLite and, when enabled, one stderr line each.
pub fn spawn_writer(
    store: Store,
    retention: i64,
    log_stderr: bool,
    mut rx: mpsc::Receiver<HttpErrorEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if log_stderr {
                diag::emit_stderr(&event);
            }
            if let Err(error) = store.record_http_error(&event, retention).await {
                eprintln!("http diagnostics: {error}");
            }
        }
    });
}

/// Records every inbound 4xx/5xx response. The path is the request path only;
/// the event constructor strips control characters and any query string.
pub async fn inbound(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();

    let response = next.run(request).await;

    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let error_kind = if status.is_server_error() {
            "inbound_server_error"
        } else {
            "inbound_client_error"
        };
        let event = HttpErrorEvent::new(
            HttpDirection::Inbound,
            Transport::Direct,
            method,
            path,
            error_kind,
            format!("inbound response HTTP {}", status.as_u16()),
        )
        .with_status(status.as_u16())
        .with_latency(latency_ms(started));
        drop(state.diag_tx.try_send(event));
    }
    response
}

fn latency_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
