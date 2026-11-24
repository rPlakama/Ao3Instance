//! Reading progress save endpoints (manual "Save Place" and auto-scroll beacon).

use std::sync::Arc;

use axum::{
    extract::{Form, State},
    response::Redirect,
};
use serde::Deserialize;

use crate::server::AppState;
use crate::server::error::{AppError, Handled};

#[derive(Deserialize)]
pub(in crate::server) struct AutoProgressForm {
    work_id: String,
    chapter: i64,
    pct: f64,
}

/// Fire-and-forget progress beacon from the reader. Returns no body.
pub(in crate::server) async fn auto_progress(
    State(state): State<Arc<AppState>>,
    Form(form): Form<AutoProgressForm>,
) -> Handled<axum::http::StatusCode> {
    if !form.pct.is_finite() {
        return Err(AppError::BadRequest("pct must be finite".into()));
    }
    store_progress(&state, &form.work_id, form.chapter, form.pct).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(in crate::server) struct ProgressForm {
    work_id: String,
    chapter: i64,
    pct: Option<f64>,
}

pub(in crate::server) async fn save_progress(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ProgressForm>,
) -> Handled<Redirect> {
    let pct = form.pct.filter(|v| v.is_finite()).unwrap_or(0.0);
    store_progress(&state, &form.work_id, form.chapter, pct).await?;
    Ok(Redirect::to(&format!(
        "/read/{}?chapter={}",
        form.work_id, form.chapter
    )))
}

async fn store_progress(state: &AppState, work_id: &str, chapter: i64, pct: f64) -> Handled<()> {
    if chapter < 1 {
        return Err(AppError::BadRequest("chapter must be positive".into()));
    }
    if state.store.get_work(work_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    state
        .store
        .save_progress_exact(work_id, chapter, pct)
        .await?;
    Ok(())
}
