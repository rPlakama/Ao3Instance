//! Replies, edits and deletes for cached AO3 comments.
//!
//! Each action follows the write spine: writes switch + usable session +
//! explicit confirmation -> governor -> intent -> exactly one request -> audit.
//! AO3's own rules are mirrored where the cache can prove them: edits/deletes
//! are only offered on the account's own comments, and a comment with replies
//! cannot be edited (`comments_controller.rb:251-259`).

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::super::AppState;
use super::super::error::{AppError, Handled};
use super::super::reads::{comment_reader_back, refresh_comments_scope};
use super::{
    ensure_pseud, record_write_failure, require_confirmation, require_non_empty_target,
    require_usable_session, require_writes_enabled, validate_comment_content,
};
use crate::ao3::actions;
use crate::error::Error;
use crate::render;
use crate::render::NavSection;
use crate::store::CommentWithScope;

#[derive(Deserialize)]
pub(in crate::server) struct CommentTargetQuery {
    work_id: Option<String>,
    comment_id: Option<String>,
    notice: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CommentTargetForm {
    work_id: String,
    comment_id: String,
    content: Option<String>,
    confirm: Option<String>,
}

async fn load_target(
    state: &AppState,
    work_id: &str,
    comment_id: &str,
) -> Handled<CommentWithScope> {
    state
        .store
        .find_comment(work_id, comment_id)
        .await?
        .ok_or(AppError::NotFound)
}

fn require_own(comment: &crate::ao3::comments::Comment, account: &str) -> Handled<()> {
    let own = comment
        .author_login
        .as_deref()
        .is_some_and(|login| login.eq_ignore_ascii_case(account));
    if own {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "you can only edit or delete your own comments".into(),
        ))
    }
}

async fn has_replies(
    state: &AppState,
    work_id: &str,
    scope: &str,
    comment_id: &str,
) -> Handled<bool> {
    let comments = state.store.get_comments(work_id, scope).await?;
    Ok(comments
        .iter()
        .any(|comment| comment.parent_id.as_deref() == Some(comment_id)))
}

#[allow(
    clippy::too_many_arguments,
    reason = "thin adapter from the handlers to the shared comment editor renderer"
)]
fn editor(
    state: &AppState,
    page: &render::PageContext,
    heading: &str,
    description: String,
    action: &str,
    work_id: &str,
    comment_id: &str,
    content: &str,
    parent: Option<&crate::ao3::comments::Comment>,
    errors: &[String],
    notice: Option<&str>,
    submit_label: &str,
) -> Handled<Html<String>> {
    let _ = state;
    let parent_author = parent.and_then(|comment| comment.author.as_deref());
    let parent_excerpt = parent.map(|comment| crate::render::sanitize(&comment.body_html));
    Ok(Html(render::comment_editor_page(
        page,
        heading,
        &description,
        action,
        work_id,
        comment_id,
        content,
        parent_author,
        parent_excerpt.as_deref(),
        errors,
        notice,
        submit_label,
    )))
}

/// `GET /write/comment/reply` — editor for one reply.
pub(in crate::server) async fn reply_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CommentTargetQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(
        query.work_id.as_deref().unwrap_or_default(),
        "missing work id",
    )?;
    let comment_id = require_non_empty_target(
        query.comment_id.as_deref().unwrap_or_default(),
        "missing comment id",
    )?;
    let target = load_target(&state, &work_id, &comment_id).await?;
    let page = super::super::page_context(&state, "Reply — AO3Instance", NavSection::Home).await;
    let author = target.comment.author.as_deref().unwrap_or("Anonymous");
    editor(
        &state,
        &page,
        "Reply to Comment",
        format!("Replying to {author} on this work."),
        "/write/comment/reply/preview",
        &work_id,
        &comment_id,
        "",
        Some(&target.comment),
        &[],
        query.notice.as_deref(),
        "Preview Reply",
    )
}

/// `POST /write/comment/reply/preview` — validate + confirm.
pub(in crate::server) async fn reply_preview(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentTargetForm>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let comment_id = require_non_empty_target(&form.comment_id, "missing comment id")?;
    let content = form.content.unwrap_or_default();
    let page =
        super::super::page_context(&state, "Confirm Reply — AO3Instance", NavSection::Home).await;
    if let Err(error) = validate_comment_content(&content) {
        let target = load_target(&state, &work_id, &comment_id).await?;
        let description = format!(
            "Replying to {} on this work.",
            target.comment.author.as_deref().unwrap_or("Anonymous")
        );
        return editor(
            &state,
            &page,
            "Reply to Comment",
            description,
            "/write/comment/reply/preview",
            &work_id,
            &comment_id,
            &content,
            Some(&target.comment),
            &[error_page_message(&error)],
            None,
            "Preview Reply",
        );
    }
    let target = load_target(&state, &work_id, &comment_id).await?;
    Ok(Html(render::confirm_page(
        "Confirm Reply",
        &format!(
            "Post this reply to {} on AO3?",
            target.comment.author.as_deref().unwrap_or("this comment")
        ),
        "/write/comment/reply",
        &[
            ("work_id", work_id.as_str()),
            ("comment_id", comment_id.as_str()),
            ("content", content.as_str()),
            ("confirm", "yes"),
        ],
        Some(&content),
    )))
}

/// `POST /write/comment/reply` — one confirmed reply.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "every other failure shares the same user-facing notice"
)]
pub(in crate::server) async fn reply_save(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentTargetForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let comment_id = require_non_empty_target(&form.comment_id, "missing comment id")?;
    let content = form.content.unwrap_or_default();
    validate_comment_content(&content)?;
    let target = load_target(&state, &work_id, &comment_id).await?;
    let scope = target.scope;

    state
        .governor
        .begin("comment")
        .await
        .map_err(AppError::from)?;
    let (intent, result) = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        let pseud = ensure_pseud(&state, &mut session, &account).await?;
        let intent = state
            .store
            .record_intent(&account, "reply_comment", "Comment", &comment_id)
            .await?;
        let result =
            actions::post_comment_reply(&mut session, &comment_id, pseud.id, &content).await;
        (intent, result)
    };
    crate::server::solver::sync_solver_state(&state).await;
    let flag = match result {
        Ok(outcome) => {
            let evidence = outcome
                .comment_id
                .map(|id| format!("comment {id}"))
                .unwrap_or_else(|| "posted".to_string());
            state
                .store
                .finish_intent(intent, "done", "replied", Some(&evidence))
                .await?;
            if refresh_comments_scope(&state, &work_id, &scope)
                .await
                .is_err()
            {
                eprintln!("refresh comments cache after reply failed; cached copy kept");
            }
            "comment_posted"
        }
        Err(error) => {
            record_write_failure(&state, "comment", intent, &error).await?;
            match error {
                Error::SpamRejected => "comment_spam",
                Error::RateLimited { .. } => "comment_rate_limited",
                _ => "comment_failed",
            }
        }
    };
    Ok(Redirect::to(
        &comment_reader_back(&state, &work_id, &scope, flag).await,
    ))
}

/// `GET /write/comment/edit` — editor prefilled with the cached comment text.
pub(in crate::server) async fn edit_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CommentTargetQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(
        query.work_id.as_deref().unwrap_or_default(),
        "missing work id",
    )?;
    let comment_id = require_non_empty_target(
        query.comment_id.as_deref().unwrap_or_default(),
        "missing comment id",
    )?;
    let target = load_target(&state, &work_id, &comment_id).await?;
    require_own(&target.comment, &account)?;
    if has_replies(&state, &work_id, &target.scope, &comment_id).await? {
        return Err(AppError::BadRequest(
            "comments with replies cannot be edited on AO3".into(),
        ));
    }
    let page =
        super::super::page_context(&state, "Edit Comment — AO3Instance", NavSection::Home).await;
    let content = target.comment.body_html.clone();
    editor(
        &state,
        &page,
        "Edit Comment",
        "Edit your comment. AO3 marks it as edited and keeps the thread position.".to_string(),
        "/write/comment/edit/preview",
        &work_id,
        &comment_id,
        &content,
        None,
        &[],
        query.notice.as_deref(),
        "Preview Edit",
    )
}

/// `POST /write/comment/edit/preview` — validate + confirm.
pub(in crate::server) async fn edit_preview(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentTargetForm>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let comment_id = require_non_empty_target(&form.comment_id, "missing comment id")?;
    let content = form.content.unwrap_or_default();
    let target = load_target(&state, &work_id, &comment_id).await?;
    require_own(&target.comment, &account)?;
    let page =
        super::super::page_context(&state, "Confirm Edit — AO3Instance", NavSection::Home).await;
    if let Err(error) = validate_comment_content(&content) {
        let original = target.comment.body_html.clone();
        return editor(
            &state,
            &page,
            "Edit Comment",
            "Edit your comment.".to_string(),
            "/write/comment/edit/preview",
            &work_id,
            &comment_id,
            &original,
            None,
            &[error_page_message(&error)],
            None,
            "Preview Edit",
        );
    }
    Ok(Html(render::confirm_page(
        "Confirm Comment Edit",
        "Replace this comment's text on AO3? The thread keeps its position.",
        "/write/comment/edit",
        &[
            ("work_id", work_id.as_str()),
            ("comment_id", comment_id.as_str()),
            ("content", content.as_str()),
            ("confirm", "yes"),
        ],
        Some(&content),
    )))
}

/// `POST /write/comment/edit` — one confirmed `PATCH /comments/<id>`.
pub(in crate::server) async fn edit_save(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentTargetForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let comment_id = require_non_empty_target(&form.comment_id, "missing comment id")?;
    let content = form.content.unwrap_or_default();
    let target = load_target(&state, &work_id, &comment_id).await?;
    require_own(&target.comment, &account)?;
    let scope = target.scope;
    if let Err(error) = validate_comment_content(&content) {
        return Err(AppError::BadRequest(error_page_message(&error)));
    }
    state
        .governor
        .begin("comment_edit")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "edit_comment", "Comment", &comment_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::update_comment(&mut session, &comment_id, &content).await
    };
    crate::server::solver::sync_solver_state(&state).await;
    let flag = match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "comment-updated",
                    Some(&format!("comment {comment_id}")),
                )
                .await?;
            if refresh_comments_scope(&state, &work_id, &scope)
                .await
                .is_err()
            {
                eprintln!("refresh comments cache after reply failed; cached copy kept");
            }
            "comment_updated"
        }
        Err(error) => {
            record_write_failure(&state, "comment_edit", intent, &error).await?;
            "comment_edit_failed"
        }
    };
    Ok(Redirect::to(
        &comment_reader_back(&state, &work_id, &scope, flag).await,
    ))
}

/// `GET /write/comment/delete` — destructive confirmation page.
pub(in crate::server) async fn delete_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CommentTargetQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(
        query.work_id.as_deref().unwrap_or_default(),
        "missing work id",
    )?;
    let comment_id = require_non_empty_target(
        query.comment_id.as_deref().unwrap_or_default(),
        "missing comment id",
    )?;
    let target = load_target(&state, &work_id, &comment_id).await?;
    require_own(&target.comment, &account)?;
    let replacing = has_replies(&state, &work_id, &target.scope, &comment_id).await?;
    let detail = if replacing {
        "This comment has replies: AO3 replaces its text with a deleted-comment marker so the thread stays intact."
    } else {
        "This removes the comment from AO3 and cannot be undone."
    };
    Ok(Html(render::confirm_page(
        "Delete Comment",
        detail,
        "/write/comment/delete",
        &[
            ("work_id", work_id.as_str()),
            ("comment_id", comment_id.as_str()),
            ("confirm", "yes"),
        ],
        None,
    )))
}

/// `POST /write/comment/delete` — one confirmed `DELETE /comments/<id>`.
pub(in crate::server) async fn delete_save(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentTargetForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before commenting.",
    )
    .await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let comment_id = require_non_empty_target(&form.comment_id, "missing comment id")?;
    let target = load_target(&state, &work_id, &comment_id).await?;
    require_own(&target.comment, &account)?;
    let scope = target.scope;
    state
        .governor
        .begin("comment_delete")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "delete_comment", "Comment", &comment_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::delete_comment(&mut session, &comment_id).await
    };
    crate::server::solver::sync_solver_state(&state).await;
    let flag = match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "comment-deleted",
                    Some(&format!("comment {comment_id}")),
                )
                .await?;
            if refresh_comments_scope(&state, &work_id, &scope)
                .await
                .is_err()
            {
                eprintln!("refresh comments cache after reply failed; cached copy kept");
            }
            "comment_deleted"
        }
        Err(error) => {
            record_write_failure(&state, "comment_delete", intent, &error).await?;
            "comment_delete_failed"
        }
    };
    Ok(Redirect::to(
        &comment_reader_back(&state, &work_id, &scope, flag).await,
    ))
}

fn error_page_message(error: &AppError) -> String {
    match error {
        AppError::BadRequest(message) | AppError::Forbidden(message) => message.clone(),
        AppError::NotFound => "not found".to_string(),
        AppError::Upstream(_) | AppError::Unavailable(_) => {
            "AO3 did not accept the request".to_string()
        }
        AppError::Blocked { .. }
        | AppError::TooManyRequests(_)
        | AppError::BlockedSearch { .. }
        | AppError::Internal(_) => "the write could not be prepared".to_string(),
    }
}
