//! Write routes (W4): confirmation -> governor -> intent -> one request -> audit.
//!
//! Every handler here refuses when `AO3I_WRITE_ENABLED` is off, and every
//! upstream call goes through the direct `Client`; the session stays the only
//! state that authorises a write.

mod actions;
mod chapter;
mod edit_work;
mod preview;
mod work;

use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, Redirect},
};

use super::AppState;
use super::error::{AppError, Handled};
use super::session::persist_session;
use crate::ao3::actions as ao3_actions;
use crate::ao3::pseuds::Pseud;
use crate::ao3::session::{AccountSession, SessionState};
use crate::error::Error;
use crate::render;
use crate::render::NavSection;

pub(super) use actions::{
    write_bookmark, write_comment, write_kudos, write_reading, write_subscribe, write_unsubscribe,
};
pub(super) use chapter::{chapter_editor, chapter_preview, chapter_save};
pub(super) use edit_work::{edit_work, edit_work_preview, edit_work_save};
pub(super) use preview::{write_comment_preview, write_preview};
pub(super) use work::{compose, delete_work, publish_draft, write_work, write_work_preview};

/// Decodes `application/x-www-form-urlencoded` into name -> repeated values.
///
/// AO3-style checkbox groups repeat keys (`warnings=...&warnings=...`), which
/// serde's form extractor cannot collect into a `Vec`; the composer and the work
/// editor both parse the body by hand for that reason.
pub(super) fn parse_form_body(
    body: &[u8],
) -> Handled<std::collections::HashMap<String, Vec<String>>> {
    let text = std::str::from_utf8(body)
        .map_err(|_| AppError::BadRequest("form body is not valid UTF-8".into()))?;
    let mut fields: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = form_component(key)?;
        let value = form_component(value)?;
        fields.entry(key).or_default().push(value);
    }
    Ok(fields)
}

fn form_component(raw: &str) -> Handled<String> {
    let plus_decoded = raw.replace('+', " ");
    crate::ao3::parse::percent_decode(&plus_decoded)
        .ok_or_else(|| AppError::BadRequest("invalid percent-encoding in form body".into()))
}

pub(super) fn require_writes_enabled(state: &AppState) -> Handled<()> {
    if !state.write_enabled {
        return Err(AppError::Forbidden(
            "Writes are disabled (set AO3I_WRITE_ENABLED=true).".into(),
        ));
    }
    Ok(())
}

pub(super) fn require_confirmation(confirm: Option<&str>) -> Handled<()> {
    if confirm != Some("yes") {
        return Err(AppError::BadRequest(
            "A deliberate confirmation is required for public actions.".into(),
        ));
    }
    Ok(())
}

pub(super) fn require_non_empty_target(raw: &str, missing_message: &str) -> Handled<String> {
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(missing_message.into()));
    }
    Ok(trimmed)
}

/// Ensures writes are enabled and a usable AO3 session exists, lazily
/// activating a restored (`Imported`) session from disk if needed.
pub(super) async fn require_usable_session(
    state: &AppState,
    signed_out_message: &str,
) -> Handled<String> {
    let (account, usable) = ensure_usable_session(state).await;
    if !usable || account.is_empty() {
        return Err(AppError::Forbidden(signed_out_message.into()));
    }
    Ok(account)
}

/// Returns `(account, usable)`, lazily activating a restored (`Imported`)
/// session from disk so writes keep working seamlessly after restarts/updates.
async fn ensure_usable_session(state: &AppState) -> (String, bool) {
    let (account, usable, needs_activation) = {
        let session = state.session.lock().await;
        (
            session.account().to_string(),
            session.state().is_usable(),
            !session.account().is_empty()
                && matches!(
                    session.state(),
                    SessionState::Imported | SessionState::Blocked
                )
                && session.jar().is_complete(),
        )
    };
    if usable || !needs_activation {
        return (account, usable);
    }
    let activated = {
        let _solver_guard = state.solver_lock.lock().await;
        let mut session = state.session.lock().await;
        drop(session.ensure_active().await);
        super::sync_session_snapshot(state, &session).await;
        (session.account().to_string(), session.state().is_usable())
    };
    super::solver::sync_solver_state(state).await;
    activated
}

/// Records a failed write uniformly across all write handlers: feeds any
/// upstream `Retry-After` back into the write governor and closes the audit row.
pub(super) async fn record_write_failure(
    state: &AppState,
    governor_action: &'static str,
    intent: i64,
    error: &Error,
) -> Handled<Option<u64>> {
    let retry_after = if let Error::RateLimited { retry_after } = error {
        *retry_after
    } else {
        None
    };
    state
        .governor
        .record_outcome(governor_action, retry_after)
        .await;
    state
        .store
        .finish_intent(intent, "failed", "failed", Some(&error.to_string()))
        .await?;
    Ok(retry_after)
}

pub(super) async fn ensure_pseud(
    state: &AppState,
    session: &mut AccountSession,
    account: &str,
) -> Handled<Pseud> {
    if let Some(pseud) = session.pseud().cloned() {
        return Ok(pseud);
    }
    let pseud = ao3_actions::discover_default_pseud(session, account)
        .await
        .map_err(AppError::from)?;
    session.set_pseud(pseud.clone());
    super::sync_session_snapshot(state, session).await;
    persist_session(state, session)
        .await
        .map_err(AppError::from)?;
    Ok(pseud)
}

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
        "work_published" => Some("Draft published on AO3."),
        "work_publish_failed" => Some("AO3 could not publish that draft; see the audit log."),
        "work_publish_rate_limited" => {
            Some("AO3 is rate limiting this instance; try publishing again later.")
        }
        "work_deleted" => Some("Work deleted on AO3."),
        "work_updated" => Some("Work updated on AO3."),
        "work_update_failed" => Some("AO3 rejected the work update; see the audit log."),
        "work_update_rate_limited" => {
            Some("AO3 is rate limiting this instance; try the update again later.")
        }
        "chapter_posted" => Some("Chapter posted on AO3."),
        "chapter_draft" => Some("Chapter saved as a draft on AO3."),
        "chapter_updated" => Some("Chapter updated on AO3."),
        "chapter_failed" => Some("AO3 rejected the chapter; see the audit log."),
        "chapter_rate_limited" => {
            Some("AO3 is rate limiting this instance; try the chapter again later.")
        }
        "work_delete_failed" => Some("AO3 could not delete that work; see the audit log."),
        "work_delete_rate_limited" => {
            Some("AO3 is rate limiting this instance; try deleting again later.")
        }
        "comments_loaded" => Some("Comments loaded from AO3 into the offline cache."),
        "comments_failed" => {
            Some("Could not load comments from AO3; the cached copy is unchanged.")
        }
        "comments_blocked" => {
            Some("AO3's edge blocked the comments request; check Diagnostics and try later.")
        }
        "comments_rate_limited" => Some("AO3 is rate limiting this instance; wait a minute."),
        "comments_gate" => Some(
            "AO3 wants a signed-in session or enabled adult content for these comments; \
             check the Session page.",
        ),
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
    if kind == ao3_actions::WORK
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

/// Invalidates the 30-minute cached AO3 list pages for an account after a
/// write mutates that list upstream.
pub(super) async fn invalidate_user_list(state: &AppState, account: &str, list: &str) {
    let seg = crate::ao3::parse::percent_encode(account, crate::ao3::parse::SpaceMode::Percent20);
    let prefix = format!("/users/{seg}/{list}");
    if let Err(err) = state.store.invalidate_cached_prefix(account, &prefix).await {
        eprintln!("invalidate page_cache {prefix}: {err}");
    }
}

pub(super) fn validate_comment_content(content: &str) -> Handled<()> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("the comment is empty".into()));
    }
    if trimmed.chars().count() > ao3_actions::COMMENT_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "the comment is longer than {} characters",
            ao3_actions::COMMENT_MAX_CHARS
        )));
    }
    Ok(())
}
