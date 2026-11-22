//! Write routes (W4): confirmation -> governor -> intent -> one request -> audit.
//!
//! Every handler here refuses when `AO3I_WRITE_ENABLED` is off, and every
//! upstream call goes through the direct `Client`; the session stays the only
//! state that authorises a write.

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::error::{AppError, Handled};
use super::session::persist_session;
use super::{AppState, session_identity};
use crate::ao3::actions;
use crate::ao3::pseuds::Pseud;
use crate::ao3::session::AccountSession;
use crate::error::Error;
use crate::render;
use crate::render::NavSection;

pub(super) fn notice_text(key: &str) -> Option<&'static str> {
    match key {
        "subscribed" => Some("Subscribed on AO3."),
        "unsubscribed" => Some("Unsubscribed on AO3."),
        "already_subscribed" => Some("Already subscribed according to the local ledger."),
        "not_subscribed" => Some("No active subscription in the local ledger."),
        "write_failed" => Some("AO3 rejected the action; see the audit log."),
        "write_rate_limited" => Some("Rate limited by the client governor; try again later."),
        "write_disabled" => Some("Writes are disabled (set AO3I_WRITE_ENABLED=true)."),
        "kudos_given" => Some("Kudos given."),
        "kudos_already" => Some("You had already left kudos on this work."),
        "kudos_failed" => Some("AO3 refused the kudos; see the audit log."),
        "reading_for_later" => Some("Marked for later."),
        "reading_read" => Some("Marked as read."),
        "reading_failed" => Some("AO3 refused the reading update; see the audit log."),
        "comment_posted" => Some("Comment posted on AO3."),
        "comment_failed" => Some("AO3 rejected the comment; see the audit log."),
        "comment_spam" => Some("AO3's spam filter rejected the comment; reword and try again."),
        "comment_rate_limited" => Some("AO3's comment limit is 3 per 15 minutes; try again later."),
        "bookmark_saved" => Some("Bookmarked on AO3."),
        "bookmark_already" => Some("Already bookmarked according to the local ledger."),
        "bookmark_failed" => Some("AO3 rejected the bookmark; see the audit log."),
        _ => None,
    }
}
pub(super) async fn audit(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let entries = state.store.recent_audit(100).await?;
    let cached = state.store.cached_work_ids().await?;
    let page = super::page_context(&state, "Audit — AO3Instance", NavSection::Audit).await;
    Ok(Html(render::audit_page(&page, &entries, &cached)))
}
pub(super) async fn redirect_after_write(
    state: &AppState,
    kind: &str,
    target_id: &str,
    flag: &str,
) -> Redirect {
    if kind == actions::WORK
        && state
            .store
            .get_work(target_id)
            .await
            .ok()
            .flatten()
            .is_some()
    {
        Redirect::to(&format!("/read/{target_id}?notice={flag}"))
    } else {
        Redirect::to(&format!("/?notice={flag}"))
    }
}

#[derive(Deserialize)]
pub(super) struct PreviewQuery {
    kind: Option<String>,
    id: Option<String>,
}

pub(super) async fn write_preview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PreviewQuery>,
) -> Handled<Html<String>> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden(
            "Sign in to AO3 on the Session page before writing.".into(),
        ));
    }
    let raw_kind = query.kind.unwrap_or_default();
    let target_id = query.id.unwrap_or_default().trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing write target".into()));
    }
    if raw_kind.trim().eq_ignore_ascii_case("bookmark") {
        let description = match state.store.get_work(&target_id).await? {
            Some(work) => format!("Bookmark “{}” on AO3?", work.title),
            None => format!("Bookmark work {target_id} on AO3?"),
        };
        return Ok(Html(render::confirm_page(
            "Confirm bookmark",
            &description,
            "/write/bookmark",
            &[("target_id", &target_id), ("confirm", "yes")],
            None,
        )));
    }
    if raw_kind.trim().eq_ignore_ascii_case("kudos") {
        let description = match state.store.get_work(&target_id).await? {
            Some(work) => format!("Leave kudos on “{}” on AO3?", work.title),
            None => format!("Leave kudos on work {target_id} on AO3?"),
        };
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
        match state.store.get_work(&target_id).await? {
            Some(work) => format!("Subscribe to “{}” on AO3?", work.title),
            None => format!("Subscribe to work {target_id} on AO3?"),
        }
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

#[derive(Deserialize)]
pub(super) struct WriteForm {
    kind: String,
    target_id: String,
    confirm: Option<String>,
}

pub(super) async fn write_subscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<WriteForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    if form.confirm.as_deref() != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    let kind = form
        .kind
        .as_str()
        .pipe_kind()
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing subscription target".into()));
    }

    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
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
        let mut session = state.session.lock().await;
        actions::subscribe(&mut session, &account, kind, &target_id).await
    };
    match result {
        Ok(outcome) => {
            state
                .store
                .upsert_subscription(&account, kind, &target_id, outcome.item_id, "active")
                .await?;
            state
                .store
                .finish_intent(intent, "done", "subscribed", outcome.message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "subscribed").await)
        }
        Err(error) => {
            let retry_after = if let Error::RateLimited { retry_after } = &error {
                *retry_after
            } else {
                None
            };
            state
                .governor
                .record_outcome("subscribe", retry_after)
                .await;
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            let flag = if retry_after.is_some() {
                "write_rate_limited"
            } else {
                "write_failed"
            };
            Ok(redirect_after_write(&state, kind, &target_id, flag).await)
        }
    }
}

pub(super) async fn write_unsubscribe(
    State(state): State<Arc<AppState>>,
    Form(form): Form<WriteForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let kind = form
        .kind
        .as_str()
        .pipe_kind()
        .ok_or_else(|| AppError::BadRequest("unknown subscription kind".into()))?;
    let target_id = form.target_id.trim().to_string();
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
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
        let mut session = state.session.lock().await;
        actions::unsubscribe(&mut session, &account, subscription_id).await
    };
    match result {
        Ok(message) => {
            state
                .store
                .upsert_subscription(&account, kind, &target_id, None, "removed")
                .await?;
            state
                .store
                .finish_intent(intent, "done", "unsubscribed", message.as_deref())
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "unsubscribed").await)
        }
        Err(error) => {
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, kind, &target_id, "write_failed").await)
        }
    }
}

#[derive(Deserialize)]
pub(super) struct KudosForm {
    target_id: String,
    confirm: Option<String>,
}

pub(super) async fn write_kudos(
    State(state): State<Arc<AppState>>,
    Form(form): Form<KudosForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    if form.confirm.as_deref() != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing kudos target".into()));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
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
        let mut session = state.session.lock().await;
        actions::kudos(&mut session, &target_id).await
    };
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
            let retry_after = if let Error::RateLimited { retry_after } = &error {
                *retry_after
            } else {
                None
            };
            state.governor.record_outcome("kudos", retry_after).await;
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "kudos_failed").await)
        }
    }
}

#[derive(Deserialize)]
pub(super) struct ReadingForm {
    target_id: String,
    action: String,
}

pub(super) async fn write_reading(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ReadingForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let for_later = match form.action.trim() {
        "later" => true,
        "read" => false,
        _ => return Err(AppError::BadRequest("unknown reading action".into())),
    };
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing reading target".into()));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }

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
        let mut session = state.session.lock().await;
        actions::set_reading_flag(&mut session, &target_id, for_later).await
    };
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
            state
                .store
                .finish_intent(intent, "done", action, None)
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, action).await)
        }
        Err(error) => {
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "reading_failed").await)
        }
    }
}

#[derive(Deserialize)]
pub(super) struct CommentForm {
    target_id: String,
    content: String,
    confirm: Option<String>,
}

pub(super) async fn ensure_pseud(
    state: &AppState,
    session: &mut AccountSession,
    account: &str,
) -> Handled<Pseud> {
    if let Some(pseud) = session.pseud().cloned() {
        return Ok(pseud);
    }
    let pseud = actions::discover_default_pseud(session, account)
        .await
        .map_err(AppError::from)?;
    session.set_pseud(pseud.clone());
    persist_session(state, session)
        .await
        .map_err(AppError::from)?;
    Ok(pseud)
}

fn validate_comment_content(content: &str) -> Handled<()> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("the comment is empty".into()));
    }
    if trimmed.chars().count() > actions::COMMENT_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "the comment is longer than {} characters",
            actions::COMMENT_MAX_CHARS
        )));
    }
    Ok(())
}

pub(super) async fn write_comment_preview(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentForm>,
) -> Handled<Html<String>> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing comment target".into()));
    }
    validate_comment_content(&form.content)?;
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
    let pseud = {
        let mut session = state.session.lock().await;
        ensure_pseud(&state, &mut session, &account).await?
    };
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

pub(super) async fn write_comment(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CommentForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    if form.confirm.as_deref() != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing comment target".into()));
    }
    validate_comment_content(&form.content)?;
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }

    state
        .governor
        .begin("comment")
        .await
        .map_err(AppError::from)?;

    let (intent, result) = {
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
            let retry_after = if let Error::RateLimited { retry_after } = &error {
                *retry_after
            } else {
                None
            };
            state.governor.record_outcome("comment", retry_after).await;
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
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
pub(super) struct BookmarkForm {
    target_id: String,
    confirm: Option<String>,
}

pub(super) async fn write_bookmark(
    State(state): State<Arc<AppState>>,
    Form(form): Form<BookmarkForm>,
) -> Handled<Redirect> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    if form.confirm.as_deref() != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    let target_id = form.target_id.trim().to_string();
    if target_id.is_empty() {
        return Err(AppError::BadRequest("missing bookmark target".into()));
    }
    let (account, usable) = session_identity(&state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden("Sign in to AO3 first.".into()));
    }
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
        let mut session = state.session.lock().await;
        let pseud = ensure_pseud(&state, &mut session, &account).await?;
        let intent = state
            .store
            .record_intent(&account, "bookmark", actions::WORK, &target_id)
            .await?;
        let result = actions::bookmark(&mut session, &target_id, pseud.id).await;
        (intent, result)
    };

    match result {
        Ok(bookmark_id) => {
            state
                .store
                .mark_bookmark(&account, &target_id, bookmark_id)
                .await?;
            let evidence = bookmark_id.map(|id| format!("bookmark {id}"));
            state
                .store
                .finish_intent(intent, "done", "bookmarked", evidence.as_deref())
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "bookmark_saved").await)
        }
        Err(error) => {
            state
                .store
                .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
                .await?;
            Ok(redirect_after_write(&state, actions::WORK, &target_id, "bookmark_failed").await)
        }
    }
}
/// `&str` → normalized subscription kind (form-friendly).
trait KindStr {
    fn pipe_kind(self) -> Option<&'static str>;
}

impl KindStr for &str {
    fn pipe_kind(self) -> Option<&'static str> {
        actions::normalize_kind(self)
    }
}
