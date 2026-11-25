//! Editing an existing work's tags/metadata through AO3's own edit form.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use super::super::AppState;
use super::super::error::{AppError, Handled};
use super::super::reads::{fetch_page_html, refresh_cached_work};
use super::work::load_defaults;
use super::{
    parse_form_body, record_write_failure, require_confirmation, require_non_empty_target,
    require_usable_session, require_writes_enabled,
};
use crate::ao3::actions;
use crate::ao3::compose::{EditWorkFields, NewWorkDefaults, validate_edit_work};
use crate::error::Error;
use crate::render;
use crate::render::NavSection;

#[derive(Deserialize)]
pub(in crate::server) struct EditWorkQuery {
    work_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct EditWorkForm {
    work_id: String,
    title: String,
    rating: String,
    warnings: Vec<String>,
    fandom: String,
    categories: Vec<String>,
    relationships: String,
    characters: String,
    additional_tags: String,
    language_id: String,
    summary: String,
    notes: String,
    endnotes: String,
    collections: String,
    recipients: String,
    confirm: Option<String>,
}

impl EditWorkForm {
    fn from_fields(fields: &std::collections::HashMap<String, Vec<String>>) -> Self {
        let one = |name: &str| {
            fields
                .get(name)
                .and_then(|values| values.first())
                .cloned()
                .unwrap_or_default()
        };
        Self {
            work_id: one("work_id"),
            title: one("title"),
            rating: one("rating"),
            warnings: fields.get("warnings").cloned().unwrap_or_default(),
            fandom: one("fandom"),
            categories: fields.get("categories").cloned().unwrap_or_default(),
            relationships: one("relationships"),
            characters: one("characters"),
            additional_tags: one("additional_tags"),
            language_id: one("language_id"),
            summary: one("summary"),
            notes: one("notes"),
            endnotes: one("endnotes"),
            collections: one("collections"),
            recipients: one("recipients"),
            confirm: fields
                .get("confirm")
                .and_then(|values| values.first())
                .cloned(),
        }
    }

    fn into_fields(self) -> EditWorkFields {
        EditWorkFields {
            title: self.title,
            rating: self.rating,
            warnings: self.warnings,
            fandom: self.fandom,
            categories: self.categories,
            relationships: self.relationships,
            characters: self.characters,
            additional_tags: self.additional_tags,
            language_id: self.language_id,
            summary: self.summary,
            notes: self.notes,
            endnotes: self.endnotes,
            collections: self.collections,
            recipients: self.recipients,
        }
    }
}

fn render_editor(
    state: &AppState,
    page: &render::PageContext,
    work_id: &str,
    defaults: &NewWorkDefaults,
    fields: &EditWorkFields,
    errors: &[String],
    notice: Option<&str>,
) -> Html<String> {
    let _ = state;
    Html(render::edit_work_page(
        page, defaults, work_id, fields, errors, notice,
    ))
}

/// `GET /write/work/edit?work_id=` — prefilled from AO3's own edit form.
pub(in crate::server) async fn edit_work(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EditWorkQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(&state, "Sign in to AO3 on the Session page before writing.").await?;
    let work_id = require_non_empty_target(
        query.work_id.as_deref().unwrap_or_default(),
        "missing work id",
    )?;
    let defaults = load_defaults(&state, false).await?;
    let page =
        super::super::page_context(&state, "Edit Work — AO3Instance", NavSection::Dashboard).await;
    let path = format!("/works/{}/edit", work_id.trim());
    let html = fetch_page_html(&state, &path, &work_id).await?;
    let fields = crate::ao3::compose::parse_edit_work_page(&html).map_err(AppError::from)?;
    Ok(render_editor(
        &state,
        &page,
        &work_id,
        &defaults,
        &fields,
        &[],
        None,
    ))
}

/// `POST /write/work/edit/preview` — local validation + confirmation.
pub(in crate::server) async fn edit_work_preview(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(&state, "Sign in to AO3 on the Session page before writing.").await?;
    let form = EditWorkForm::from_fields(&parse_form_body(&body)?);
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let fields = form.into_fields();
    let errors = validate_edit_work(&fields);
    if !errors.is_empty() {
        let defaults = load_defaults(&state, false).await?;
        let page =
            super::super::page_context(&state, "Edit Work — AO3Instance", NavSection::Dashboard)
                .await;
        return Ok(render_editor(
            &state, &page, &work_id, &defaults, &fields, &errors, None,
        ));
    }
    let page = super::super::page_context(
        &state,
        "Confirm Work Update — AO3Instance",
        NavSection::Dashboard,
    )
    .await;
    Ok(Html(render::confirm_edit_page(&page, &work_id, &fields)))
}

/// `POST /write/work/edit` — the one real `PATCH /works/<id>`.
pub(in crate::server) async fn edit_work_save(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Response> {
    require_writes_enabled(&state)?;
    require_confirmation(
        EditWorkForm::from_fields(&parse_form_body(&body)?)
            .confirm
            .as_deref(),
    )?;
    let account =
        require_usable_session(&state, "Sign in to AO3 on the Session page before writing.")
            .await?;
    let form = EditWorkForm::from_fields(&parse_form_body(&body)?);
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let fields = form.into_fields();
    let errors = validate_edit_work(&fields);
    let page =
        super::super::page_context(&state, "Edit Work — AO3Instance", NavSection::Dashboard).await;
    if !errors.is_empty() {
        let defaults = load_defaults(&state, false).await?;
        return Ok(
            render_editor(&state, &page, &work_id, &defaults, &fields, &errors, None)
                .into_response(),
        );
    }

    let was_cached = state.store.get_work(&work_id).await?.is_some();
    state
        .governor
        .begin("edit_work")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "update_work", "Work", &work_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::update_work(&mut session, &work_id, &fields).await
    };
    super::super::solver::sync_solver_state(&state).await;
    match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "work-updated",
                    Some(&format!("work {work_id}")),
                )
                .await?;
            if was_cached {
                // Best effort: the write already succeeded upstream.
                if refresh_cached_work(&state, &work_id).await.is_err() {
                    eprintln!("refresh work cache after work edit failed; cached copy kept");
                }
                Ok(Redirect::to(&format!("/read/{work_id}?notice=work_updated")).into_response())
            } else {
                Ok(Redirect::to(&format!("/open/{work_id}")).into_response())
            }
        }
        Err(error) => {
            if matches!(error, Error::Network(_)) {
                state.governor.record_outcome("edit_work", None).await;
                state
                    .store
                    .finish_intent(
                        intent,
                        "ambiguous",
                        "ambiguous",
                        Some("work edit timed out; verify the work on AO3 before retrying"),
                    )
                    .await?;
            } else {
                record_write_failure(&state, "edit_work", intent, &error).await?;
            }
            let defaults = load_defaults(&state, false).await?;
            Ok(render_editor(
                &state,
                &page,
                &work_id,
                &defaults,
                &fields,
                &[error.to_string()],
                None,
            )
            .into_response())
        }
    }
}
