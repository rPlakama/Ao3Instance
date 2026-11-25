//! Bookmark edit and delete: confirmation, one request, audit, list refresh.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::super::AppState;
use super::super::error::{AppError, Handled};
use super::{
    invalidate_user_list, parse_form_body, record_write_failure, require_confirmation,
    require_non_empty_target, require_usable_session, require_writes_enabled,
};
use crate::ao3::account_lists::{
    BookmarkEditFields, parse_bookmark_editor, validate_bookmark_fields,
};
use crate::ao3::actions;
use crate::render;
use crate::render::NavSection;

#[derive(Deserialize)]
pub(in crate::server) struct BookmarkTargetQuery {
    bookmark_id: Option<String>,
    work_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct BookmarkEditForm {
    bookmark_id: String,
    work_id: String,
    notes: String,
    tags: String,
    collections: String,
    private: bool,
    rec: bool,
    confirm: Option<String>,
}

impl BookmarkEditForm {
    fn from_fields(fields: &std::collections::HashMap<String, Vec<String>>) -> Self {
        let one = |name: &str| {
            fields
                .get(name)
                .and_then(|values| values.first())
                .cloned()
                .unwrap_or_default()
        };
        let flag = |name: &str| {
            fields.get(name).is_some_and(|values| {
                values
                    .iter()
                    .any(|value| matches!(value.as_str(), "1" | "on" | "true"))
            })
        };
        Self {
            bookmark_id: one("bookmark_id"),
            work_id: one("work_id"),
            notes: one("notes"),
            tags: one("tags"),
            collections: one("collections"),
            private: flag("private"),
            rec: flag("rec"),
            confirm: fields
                .get("confirm")
                .and_then(|values| values.first())
                .cloned(),
        }
    }

    fn into_fields(self) -> BookmarkEditFields {
        BookmarkEditFields {
            notes: self.notes,
            tags: self.tags,
            collections: self.collections,
            private: self.private,
            rec: self.rec,
            bookmarkable_title: String::new(),
            work_id: None,
        }
    }
}

async fn work_title(state: &AppState, work_id: &str) -> String {
    if work_id.is_empty() {
        return "this bookmark".to_string();
    }
    state
        .store
        .get_work(work_id)
        .await
        .ok()
        .flatten()
        .map(|work| work.title)
        .unwrap_or_else(|| format!("work {work_id}"))
}

fn render_editor(
    page: &render::PageContext,
    bookmark_id: &str,
    work_id: &str,
    title: &str,
    fields: &BookmarkEditFields,
    errors: &[String],
    notice: Option<&str>,
) -> Html<String> {
    Html(render::bookmark_edit_page(
        page,
        bookmark_id,
        work_id,
        title,
        fields,
        errors,
        notice,
    ))
}

/// `GET /write/bookmark/edit` — prefilled from AO3's bookmark edit page.
pub(in crate::server) async fn edit_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BookmarkTargetQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before editing bookmarks.",
    )
    .await?;
    let bookmark_id = require_non_empty_target(
        query.bookmark_id.as_deref().unwrap_or_default(),
        "missing bookmark id",
    )?;
    let path = format!("/bookmarks/{}/edit", bookmark_id.trim());
    let html = super::super::reads::fetch_page_html(&state, &path, "").await?;
    let fields = parse_bookmark_editor(&html).ok_or_else(|| {
        AppError::Upstream(
            "AO3 did not show this bookmark's edit page (not the bookmark owner, or it changed?)"
                .into(),
        )
    })?;
    let work_id = fields
        .work_id
        .clone()
        .unwrap_or_else(|| query.work_id.clone().unwrap_or_default());
    let title = fields.bookmarkable_title.clone();
    let page =
        super::super::page_context(&state, "Edit Bookmark — AO3Instance", NavSection::Dashboard)
            .await;
    Ok(render_editor(
        &page,
        &bookmark_id,
        &work_id,
        &title,
        &fields,
        &[],
        None,
    ))
}

/// `POST /write/bookmark/edit/preview` — local validation + confirmation.
pub(in crate::server) async fn edit_preview(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before editing bookmarks.",
    )
    .await?;
    let form = BookmarkEditForm::from_fields(&parse_form_body(&body)?);
    let bookmark_id = require_non_empty_target(&form.bookmark_id, "missing bookmark id")?;
    let work_id = form.work_id.clone();
    let fields = form.into_fields();
    let errors = validate_bookmark_fields(&fields);
    let title = work_title(&state, &work_id).await;
    let page =
        super::super::page_context(&state, "Edit Bookmark — AO3Instance", NavSection::Dashboard)
            .await;
    if !errors.is_empty() {
        return Ok(render_editor(
            &page,
            &bookmark_id,
            &work_id,
            &title,
            &fields,
            &errors,
            None,
        ));
    }
    let mut hidden: Vec<(&str, &str)> = vec![
        ("bookmark_id", bookmark_id.as_str()),
        ("work_id", work_id.as_str()),
        ("notes", fields.notes.as_str()),
        ("tags", fields.tags.as_str()),
        ("collections", fields.collections.as_str()),
        ("confirm", "yes"),
    ];
    if fields.private {
        hidden.push(("private", "1"));
    }
    if fields.rec {
        hidden.push(("rec", "1"));
    }
    Ok(Html(render::confirm_page(
        "Confirm Bookmark Edit",
        "Update this bookmark's notes, tags and flags on AO3?",
        "/write/bookmark/edit",
        &hidden,
        Some(&fields.notes),
    )))
}

/// `POST /write/bookmark/edit` — one confirmed `PATCH /bookmarks/<id>`.
pub(in crate::server) async fn edit_save(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    let form = BookmarkEditForm::from_fields(&parse_form_body(&body)?);
    require_confirmation(form.confirm.as_deref())?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before editing bookmarks.",
    )
    .await?;
    let bookmark_id = require_non_empty_target(&form.bookmark_id, "missing bookmark id")?;
    let fields = form.into_fields();
    validate_bookmark_fields(&fields)
        .first()
        .map_or(Ok(()), |error| Err(AppError::BadRequest(error.clone())))?;
    state
        .governor
        .begin("bookmark_edit")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "edit_bookmark", "Bookmark", &bookmark_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::update_bookmark(&mut session, &bookmark_id, &fields).await
    };
    crate::server::solver::sync_solver_state(&state).await;
    match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "bookmark-updated",
                    Some(&format!("bookmark {bookmark_id}")),
                )
                .await?;
            invalidate_user_list(&state, &account, "bookmarks").await;
            Ok(Redirect::to("/bookmarks?notice=bookmark_updated"))
        }
        Err(error) => {
            record_write_failure(&state, "bookmark_edit", intent, &error).await?;
            Ok(Redirect::to("/bookmarks?notice=bookmark_update_failed"))
        }
    }
}

/// `GET /write/bookmark/delete` — destructive confirmation.
pub(in crate::server) async fn delete_page(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BookmarkTargetQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before editing bookmarks.",
    )
    .await?;
    let bookmark_id = require_non_empty_target(
        query.bookmark_id.as_deref().unwrap_or_default(),
        "missing bookmark id",
    )?;
    let work_id = query.work_id.clone().unwrap_or_default();
    let title = work_title(&state, &work_id).await;
    Ok(Html(render::confirm_page(
        "Delete Bookmark",
        &format!(
            "Delete the bookmark on “{title}”? This removes it from AO3 and cannot be undone."
        ),
        "/write/bookmark/delete",
        &[
            ("bookmark_id", bookmark_id.as_str()),
            ("work_id", work_id.as_str()),
            ("confirm", "yes"),
        ],
        None,
    )))
}

/// `POST /write/bookmark/delete` — one confirmed `DELETE /bookmarks/<id>`.
pub(in crate::server) async fn delete_save(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    let form = BookmarkEditForm::from_fields(&parse_form_body(&body)?);
    require_confirmation(form.confirm.as_deref())?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before editing bookmarks.",
    )
    .await?;
    let bookmark_id = require_non_empty_target(&form.bookmark_id, "missing bookmark id")?;
    let work_id = form.work_id.clone();
    state
        .governor
        .begin("bookmark_delete")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "delete_bookmark", "Bookmark", &bookmark_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::delete_bookmark(&mut session, &bookmark_id).await
    };
    crate::server::solver::sync_solver_state(&state).await;
    match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "bookmark-deleted",
                    Some(&format!("bookmark {bookmark_id}")),
                )
                .await?;
            if !work_id.is_empty()
                && let Err(error) = state.store.remove_bookmark(&account, &work_id).await
            {
                eprintln!("remove bookmark receipt: {error}");
            }
            invalidate_user_list(&state, &account, "bookmarks").await;
            Ok(Redirect::to("/bookmarks?notice=bookmark_deleted"))
        }
        Err(error) => {
            record_write_failure(&state, "bookmark_delete", intent, &error).await?;
            Ok(Redirect::to("/bookmarks?notice=bookmark_delete_failed"))
        }
    }
}
