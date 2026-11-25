//! Chapter editor and write handlers: AO3's chapter index, chapter form and the
//! one confirmed create/update request (`chapters_controller.rb#create/#update`).

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use super::super::AppState;
use super::super::error::{AppError, Handled};
use super::{
    record_write_failure, require_confirmation, require_non_empty_target, require_usable_session,
    require_writes_enabled,
};
use crate::ao3::actions::{self, ChapterSave};
use crate::ao3::chapters::{
    ChapterFields, ChapterIndex, parse_chapter_editor, parse_chapter_index, validate_chapter,
};
use crate::error::Error;
use crate::render;
use crate::render::NavSection;

#[derive(Deserialize)]
pub(in crate::server) struct ChapterEditorQuery {
    work_id: Option<String>,
    chapter_id: Option<String>,
    notice: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ChapterForm {
    work_id: String,
    chapter_id: Option<String>,
    title: Option<String>,
    position: Option<String>,
    wip_length: Option<String>,
    summary: Option<String>,
    notes: Option<String>,
    endnotes: Option<String>,
    content: Option<String>,
    action: Option<String>,
    confirm: Option<String>,
}

impl ChapterForm {
    fn fields(&self) -> ChapterFields {
        ChapterFields {
            title: self.title.clone().unwrap_or_default(),
            position: self.position.clone().unwrap_or_default(),
            wip_length: self.wip_length.clone().unwrap_or_default(),
            summary: self.summary.clone().unwrap_or_default(),
            notes: self.notes.clone().unwrap_or_default(),
            endnotes: self.endnotes.clone().unwrap_or_default(),
            content: self.content.clone().unwrap_or_default(),
        }
    }
}

async fn work_title(state: &AppState, work_id: &str) -> String {
    state
        .store
        .get_work(work_id)
        .await
        .ok()
        .flatten()
        .map(|work| work.title)
        .unwrap_or_else(|| format!("work {work_id}"))
}

#[allow(
    clippy::too_many_arguments,
    reason = "thin adapter from the handler to the chapter editor renderer"
)]
fn editor_html(
    page: &render::PageContext,
    work_id: &str,
    chapter_id: Option<&str>,
    work_title: &str,
    default_position: usize,
    fields: &ChapterFields,
    errors: &[String],
    notice: Option<&str>,
) -> Html<String> {
    Html(render::chapter_editor_page(
        page,
        work_id,
        chapter_id,
        work_title,
        default_position,
        fields,
        errors,
        notice,
    ))
}

/// `GET /write/chapter/new` and `GET /write/chapter/edit`.
pub(in crate::server) async fn chapter_editor(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ChapterEditorQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(&state, "Sign in to AO3 on the Session page before writing.").await?;
    let work_id = require_non_empty_target(
        query.work_id.as_deref().unwrap_or_default(),
        "missing work id",
    )?;
    let page =
        super::super::page_context(&state, "Chapter — AO3Instance", NavSection::Dashboard).await;
    let chapter_id = query
        .chapter_id
        .as_deref()
        .filter(|id| !id.trim().is_empty());

    let fields = if let Some(chapter_id) = chapter_id {
        let path = format!(
            "/works/{}/chapters/{}/edit",
            work_id.trim(),
            chapter_id.trim()
        );
        let html = super::super::reads::fetch_page_html(&state, &path, &work_id).await?;
        parse_chapter_editor(&html).map_err(AppError::from)?
    } else {
        ChapterFields::default()
    };
    let (index, default_position) = if chapter_id.is_none() {
        let path = format!("/works/{}/navigate", work_id.trim());
        let html = super::super::reads::fetch_page_html(&state, &path, &work_id).await?;
        let index = parse_chapter_index(&html).map_err(AppError::from)?;
        let position = index.chapters.len() + 1;
        (index, position)
    } else {
        (ChapterIndex::default(), 1)
    };
    let title = if index.work_title.is_empty() {
        work_title(&state, &work_id).await
    } else {
        index.work_title.clone()
    };
    let errors: Vec<String> = query.error.iter().cloned().collect();
    Ok(editor_html(
        &page,
        &work_id,
        chapter_id,
        &title,
        default_position,
        &fields,
        &errors,
        query.notice.as_deref(),
    ))
}

/// `POST /write/chapter/preview` — local validation + confirmation.
pub(in crate::server) async fn chapter_preview(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ChapterForm>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(&state, "Sign in to AO3 on the Session page before writing.").await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let fields = form.fields();
    let errors = validate_chapter(&fields);
    let page = super::super::page_context(
        &state,
        "Confirm Chapter — AO3Instance",
        NavSection::Dashboard,
    )
    .await;
    if !errors.is_empty() {
        let title = work_title(&state, &work_id).await;
        let position = fields.position.trim().parse::<usize>().unwrap_or(1);
        return Ok(Html(render::chapter_editor_page(
            &page,
            &work_id,
            form.chapter_id.as_deref(),
            &title,
            position,
            &fields,
            &errors,
            None,
        )));
    }
    let title = work_title(&state, &work_id).await;
    let action = form.action.as_deref().unwrap_or("post");
    let action = match (form.chapter_id.is_some(), action) {
        (true, _) => "update",
        (false, "draft") => "draft",
        _ => "post",
    };
    Ok(Html(render::chapter_confirm_page(
        &page,
        &work_id,
        form.chapter_id.as_deref(),
        &title,
        &fields,
        action,
    )))
}

fn chapter_save_mode(chapter_id: Option<&str>, action: Option<&str>) -> Handled<ChapterSave> {
    if chapter_id.is_some() {
        return Ok(ChapterSave::Update);
    }
    match action.unwrap_or("post").trim() {
        "post" | "publish" => Ok(ChapterSave::Post),
        "draft" | "save" => Ok(ChapterSave::Draft),
        "update" => Err(AppError::BadRequest(
            "a chapter id is required to update a chapter".into(),
        )),
        other => Err(AppError::BadRequest(format!(
            "unknown chapter action {other:?}"
        ))),
    }
}

/// `POST /write/chapter` — the one real create/update request.
pub(in crate::server) async fn chapter_save(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ChapterForm>,
) -> Handled<Response> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let account =
        require_usable_session(&state, "Sign in to AO3 on the Session page before writing.")
            .await?;
    let work_id = require_non_empty_target(&form.work_id, "missing work id")?;
    let chapter_id = form
        .chapter_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let mode = chapter_save_mode(chapter_id.as_deref(), form.action.as_deref())?;
    let fields = form.fields();
    let errors = validate_chapter(&fields);
    let page =
        super::super::page_context(&state, "Chapter — AO3Instance", NavSection::Dashboard).await;
    if !errors.is_empty() {
        let title = work_title(&state, &work_id).await;
        return Ok(Html(render::chapter_editor_page(
            &page,
            &work_id,
            chapter_id.as_deref(),
            &title,
            1,
            &fields,
            &errors,
            None,
        ))
        .into_response());
    }

    state
        .governor
        .begin("chapter")
        .await
        .map_err(AppError::from)?;
    let intent_action = if mode == ChapterSave::Update {
        "update_chapter"
    } else {
        "post_chapter"
    };
    let intent = state
        .store
        .record_intent(
            &account,
            intent_action,
            "Chapter",
            &format!("work {work_id}"),
        )
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::save_chapter(&mut session, &work_id, chapter_id.as_deref(), &fields, mode).await
    };
    super::super::solver::sync_solver_state(&state).await;
    match result {
        Ok(saved) => {
            let outcome = match mode {
                ChapterSave::Update => "chapter-updated",
                ChapterSave::Draft => "chapter-draft",
                ChapterSave::Post => "chapter-posted",
            };
            let evidence = saved
                .chapter_id
                .as_deref()
                .map_or_else(|| "saved".to_string(), |id| format!("chapter {id}"));
            state
                .store
                .finish_intent(intent, "done", outcome, Some(&evidence))
                .await?;
            // Best effort: the write already succeeded upstream, so a refresh
            // failure only means the reader shows the previous cache.
            if super::super::reads::refresh_cached_work(&state, &work_id)
                .await
                .is_err()
            {
                eprintln!("refresh work cache after chapter save failed; cached copy kept");
            }
            let flag = match mode {
                ChapterSave::Update => "chapter_updated",
                ChapterSave::Draft => "chapter_draft",
                ChapterSave::Post => "chapter_posted",
            };
            Ok(Redirect::to(&format!("/read/{work_id}?notice={flag}")).into_response())
        }
        Err(error) => {
            if matches!(error, Error::Network(_)) {
                state.governor.record_outcome("chapter", None).await;
                state
                    .store
                    .finish_intent(
                        intent,
                        "ambiguous",
                        "ambiguous",
                        Some("chapter save timed out; verify the chapter on AO3 before retrying"),
                    )
                    .await?;
                let title = work_title(&state, &work_id).await;
                return Ok(Html(render::chapter_editor_page(
                    &page,
                    &work_id,
                    chapter_id.as_deref(),
                    &title,
                    1,
                    &fields,
                    &[
                        "Saving the chapter timed out. It may already be saved on AO3 — check \
                         the chapter list there before retrying."
                            .to_string(),
                    ],
                    None,
                ))
                .into_response());
            }
            record_write_failure(&state, "chapter", intent, &error).await?;
            let title = work_title(&state, &work_id).await;
            Ok(Html(render::chapter_editor_page(
                &page,
                &work_id,
                chapter_id.as_deref(),
                &title,
                1,
                &fields,
                &[error.to_string()],
                None,
            ))
            .into_response())
        }
    }
}
