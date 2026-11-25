//! `POST /read/<id>/comments` — one explicit fetch of AO3's comment thread.
//!
//! Comments are a read, not a write: the handler never requires a session or the
//! write switch, and it makes exactly one upstream GET per click. The parsed
//! thread replaces the cache for the reader's commentable scope; any failure
//! leaves the previous cache untouched and explains itself with a fixed notice.

use std::sync::Arc;

use axum::{
    extract::{Form, Path, State},
    response::Redirect,
};
use serde::Deserialize;

use super::{
    comment_scope, fetch_page_html, is_truthy, looks_like_adult_gate, looks_like_login_gate,
};
use crate::ao3::comments::parse_comments_page;
use crate::server::AppState;
use crate::server::error::{AppError, Handled};

#[derive(Deserialize)]
pub(in crate::server) struct LoadCommentsForm {
    /// Reader chapter index; the handler resolves it to AO3's chapter id.
    chapter: Option<i64>,
    /// AO3-compatible Entire Work flag (work-level comments).
    full_work: Option<String>,
}

pub(in crate::server) async fn load_comments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Form(form): Form<LoadCommentsForm>,
) -> Handled<Redirect> {
    if state.store.get_work(&id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let chapters = state.store.get_chapters(&id).await?;
    let full_work = is_truthy(form.full_work.as_deref());
    let current = form
        .chapter
        .filter(|idx| chapters.iter().any(|chapter| chapter.idx == *idx))
        .or_else(|| chapters.first().map(|chapter| chapter.idx))
        .unwrap_or(1);
    let (scope, path) = comment_scope(&id, &chapters, current, full_work);

    let mut back = format!("/read/{id}?chapter={current}");
    if full_work {
        back.push_str("&view_full_work=true");
    }
    back.push_str("&show_comments=1");

    let flag = match fetch_page_html(&state, &path, &id).await {
        Ok(html) => match parse_comments_page(&html) {
            Ok(parsed) => {
                match state
                    .store
                    .replace_comments(&id, &scope, &parsed.comments, parsed.total, parsed.has_more)
                    .await
                {
                    Ok(()) => "comments_loaded",
                    Err(error) => {
                        eprintln!("comments cache: {error}");
                        "comments_failed"
                    }
                }
            }
            Err(error) => {
                if looks_like_adult_gate(&html) || looks_like_login_gate(&html) {
                    "comments_gate"
                } else {
                    eprintln!("comments parse: {error}");
                    "comments_failed"
                }
            }
        },
        Err(error) => comments_failure_flag(&error),
    };
    Ok(Redirect::to(&format!("{back}&notice={flag}#comments")))
}

/// Fixed-notice flag for a failed fetch. The handler redirects back to the
/// reader instead of rendering an error page: the reader keeps the cached
/// comments and the notice explains what happened.
fn comments_failure_flag(error: &AppError) -> &'static str {
    match error {
        AppError::Blocked { .. } => "comments_blocked",
        AppError::Upstream(message) if message.contains("rate limiting") => "comments_rate_limited",
        AppError::Upstream(message) if message.contains("redirected") => "comments_gate",
        AppError::BadRequest(_)
        | AppError::Forbidden(_)
        | AppError::NotFound
        | AppError::Upstream(_)
        | AppError::Unavailable(_)
        | AppError::TooManyRequests(_)
        | AppError::BlockedSearch { .. }
        | AppError::Internal(_) => "comments_failed",
    }
}
