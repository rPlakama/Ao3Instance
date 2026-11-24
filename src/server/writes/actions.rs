//! Confirmed write execution handlers (subscribe, unsubscribe, kudos, reading,
//! comment, bookmark).

use std::sync::Arc;

use axum::{
    extract::{Form, State},
    response::Redirect,
};
use serde::Deserialize;

use crate::ao3::actions;
use crate::error::Error;
use crate::server::AppState;
use crate::server::error::{AppError, Handled};

use super::{
    ensure_pseud, invalidate_user_list, record_write_failure, redirect_after_write,
    require_confirmation, require_non_empty_target, require_usable_session, require_writes_enabled,
    validate_comment_content,
};

#[derive(Deserialize)]
pub(in crate::server) struct WriteForm {
    kind: String,
    target_id: String,
    confirm: Option<String>,
    subscription_id: Option<i64>,
}

pub(in crate::server) async fn write_subscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<WriteForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let kind = actions::normalize_kind(&form.kind)
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let target_id = require_non_empty_target(&form.target_id, "missing subscription target")?;
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;

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
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        actions::subscribe(&mut session, &account, kind, &target_id).await
    };
    crate::server::solver::sync_solver_state(&state).await;
    match result {
        Ok(outcome) => {
            state
                .store
                .upsert_subscription(&account, kind, &target_id, outcome.item_id, "active")
                .await?;
            invalidate_user_list(&state, &account, "subscriptions").await;
            state
                .store
                .finish_intent(intent, "done", "subscribed", outcome.message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "subscribed").await)
        }
        Err(error) => {
            let retry_after = record_write_failure(&state, "subscribe", intent, &error).await?;
            let flag = if retry_after.is_some() {
                "write_rate_limited"
            } else {
                "write_failed"
            };
            Ok(redirect_after_write(&state, kind, &target_id, flag).await)
        }
    }
}

pub(in crate::server) async fn write_unsubscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<WriteForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    let kind = actions::normalize_kind(&form.kind)
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let target_id = form.target_id.trim().to_string();
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;

    let subscription_id = if let Some(id) = form.subscription_id {
        id
    } else {
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
        subscription_id
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
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        actions::unsubscribe(&mut session, &account, subscription_id).await
    };
    crate::server::solver::sync_solver_state(&state).await;
    match result {
        Ok(message) => {
            state
                .store
                .upsert_subscription(&account, kind, &target_id, None, "removed")
                .await?;
            invalidate_user_list(&state, &account, "subscriptions").await;
            state
                .store
                .finish_intent(intent, "done", "unsubscribed", message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "unsubscribed").await)
        }
        Err(error) => {
            let retry_after = record_write_failure(&state, "unsubscribe", intent, &error).await?;
            let flag = if retry_after.is_some() {
                "write_rate_limited"
            } else {
                "write_failed"
            };
            Ok(redirect_after_write(&state, kind, &target_id, flag).await)
        }
    }
}

#[derive(Deserialize)]
pub(in crate::server) struct KudosForm {
    target_id: String,
    confirm: Option<String>,
}

pub(in crate::server) async fn write_kudos(
    State(state): State<Arc<AppState>>,
    Form(form): Form<KudosForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let target_id = require_non_empty_target(&form.target_id, "missing kudos target")?;
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;

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
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        actions::kudos(&mut session, &target_id).await
    };
    crate::server::solver::sync_solver_state(&state).await;
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
            record_write_failure(&state, "kudos", intent, &error).await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "kudos_failed").await)
        }
    }
}

#[derive(Deserialize)]
pub(in crate::server) struct ReadingForm {
    target_id: String,
    action: String,
}

pub(in crate::server) async fn write_reading(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ReadingForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    let for_later = match form.action.trim() {
        "later" => true,
        "read" => false,
        _ => return Err(AppError::BadRequest("unknown reading action".into())),
    };
    let target_id = require_non_empty_target(&form.target_id, "missing reading target")?;
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;

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
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        actions::set_reading_flag(&mut session, &target_id, for_later).await
    };
    crate::server::solver::sync_solver_state(&state).await;
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
            invalidate_user_list(&state, &account, "readings").await;
            state
                .store
                .finish_intent(intent, "done", action, None)
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, action).await)
        }
        Err(error) => {
            record_write_failure(&state, "reading", intent, &error).await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "reading_failed").await)
        }
    }
}

#[derive(Deserialize)]
pub(in crate::server) struct CommentForm {
    pub(super) target_id: String,
    pub(super) content: String,
    confirm: Option<String>,
}

pub(in crate::server) async fn write_comment(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let target_id = require_non_empty_target(&form.target_id, "missing comment target")?;
    validate_comment_content(&form.content)?;
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;

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
            .record_intent(&account, "comment", actions::WORK, &target_id)
            .await?;
        let result =
            actions::post_comment(&mut session, &target_id, pseud.id, form.content.as_str()).await;
        (intent, result)
    };
    crate::server::solver::sync_solver_state(&state).await;

    match result {
        Ok(outcome) => {
            let evidence = outcome
                .comment_id
                .map(|id| format!("comment {id}"))
                .unwrap_or_else(|| "posted".to_string());
            state
                .store
                .finish_intent(intent, "done", "commented", Some(&evidence))
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "comment_posted").await)
        }
        Err(error) => {
            record_write_failure(&state, "comment", intent, &error).await?;
            let flag = if matches!(error, Error::SpamRejected) {
                "comment_spam"
            } else if matches!(error, Error::RateLimited { .. }) {
                "comment_rate_limited"
            } else {
                "comment_failed"
            };
            Ok(redirect_after_write(&state, actions::WORK, &target_id, flag).await)
        }
    }
}

#[derive(Deserialize)]
pub(in crate::server) struct BookmarkForm {
    target_id: String,
    confirm: Option<String>,
}

pub(in crate::server) async fn write_bookmark(
    State(state): State<Arc<AppState>>,
    Form(form): Form<BookmarkForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let target_id = require_non_empty_target(&form.target_id, "missing bookmark target")?;
    let account = require_usable_session(&state, "Sign in to AO3 first.").await?;

    if state.store.has_bookmark(&account, &target_id).await? {
        return Ok(
            redirect_after_write(&state, actions::WORK, &target_id, "bookmark_already").await,
        );
    }

    state
        .governor
        .begin("bookmark")
        .await
        .map_err(AppError::from)?;

    let (intent, result) = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        let pseud = ensure_pseud(&state, &mut session, &account).await?;
        let intent = state
            .store
            .record_intent(&account, "bookmark", actions::WORK, &target_id)
            .await?;
        let result = actions::bookmark(&mut session, &target_id, pseud.id).await;
        (intent, result)
    };
    crate::server::solver::sync_solver_state(&state).await;

    match result {
        Ok(bookmark_id) => {
            state
                .store
                .mark_bookmark(&account, &target_id, bookmark_id)
                .await?;
            invalidate_user_list(&state, &account, "bookmarks").await;
            let evidence = bookmark_id.map(|id| format!("bookmark {id}"));
            state
                .store
                .finish_intent(intent, "done", "bookmarked", evidence.as_deref())
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "bookmark_saved").await)
        }
        Err(error) => {
            record_write_failure(&state, "bookmark", intent, &error).await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "bookmark_failed").await)
        }
    }
}
