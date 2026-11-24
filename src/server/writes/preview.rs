//! Confirmation preview routes before any public write reaches AO3.

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::ao3::actions;
use crate::render;
use crate::server::AppState;
use crate::server::error::{AppError, Handled};

use super::actions::CommentForm;
use super::{
    ensure_pseud, require_non_empty_target, require_usable_session, require_writes_enabled,
    validate_comment_content,
};

#[derive(Deserialize)]
pub(in crate::server) struct PreviewQuery {
    kind: Option<String>,
    id: Option<String>,
}

pub(in crate::server) async fn write_preview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PreviewQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(&state, "Sign in to AO3 on the Session page before writing.").await?;
    let raw_kind = query.kind.unwrap_or_default();
    let target_id = require_non_empty_target(
        query.id.as_deref().unwrap_or_default(),
        "missing write target",
    )?;

    if raw_kind.trim().eq_ignore_ascii_case("bookmark") {
        let description = work_prompt(&state, &target_id, "Bookmark").await?;
        return Ok(Html(render::confirm_page(
            "Confirm bookmark",
            &description,
            "/write/bookmark",
            &[("target_id", &target_id), ("confirm", "yes")],
            None,
        )));
    }
    if raw_kind.trim().eq_ignore_ascii_case("kudos") {
        let description = work_prompt(&state, &target_id, "Leave kudos on").await?;
        return Ok(Html(render::confirm_page(
            "Confirm kudos",
            &description,
            "/write/kudos",
            &[("target_id", &target_id), ("confirm", "yes")],
            None,
        )));
    }
    let kind = actions::normalize_kind(&raw_kind)
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let description = if kind == actions::WORK {
        work_prompt(&state, &target_id, "Subscribe to").await?
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
        None,
    )))
}

pub(in crate::server) async fn write_comment_preview(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentForm>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    let target_id = require_non_empty_target(&form.target_id, "missing comment target")?;
    validate_comment_content(&form.content)?;
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;
    let pseud = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        ensure_pseud(&state, &mut session, &account).await?
    };
    crate::server::solver::sync_solver_state(&state).await;
    let title = state
        .store
        .get_work(&target_id)
        .await?
        .map(|work| work.title)
        .unwrap_or_else(|| format!("work {target_id}"));
    Ok(Html(render::confirm_page(
        "Confirm comment",
        &format!(
            "Post this comment on “{title}” as {}? It will be public on AO3.",
            pseud.name
        ),
        "/write/comment",
        &[
            ("target_id", &target_id),
            ("content", &form.content),
            ("confirm", "yes"),
        ],
        Some(&form.content),
    )))
}

async fn work_prompt(state: &AppState, target_id: &str, verb: &str) -> Handled<String> {
    Ok(match state.store.get_work(target_id).await? {
        Some(work) => format!("{verb} “{}” on AO3?", work.title),
        None => format!("{verb} work {target_id} on AO3?"),
    })
}
