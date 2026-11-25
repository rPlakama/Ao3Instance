//! Composer and work posting: `GET /works/new` -> local confirm -> one
//! `POST /works` on AO3 (`works_controller.rb#create`).
//!
//! The composer's options come from AO3's own new-work page (language ids and
//! creator pseuds are private database facts; see `ao3::compose`). Posting is a
//! non-idempotent create, so the pipeline is confirm -> intent -> one request ->
//! classify -> audit, and a timeout is recorded as ambiguous, never retried.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Bytes,
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use super::super::AppState;
use super::super::error::{AppError, Handled};
use super::{
    ensure_pseud, invalidate_user_list, record_write_failure, require_confirmation,
    require_non_empty_target, require_usable_session, require_writes_enabled,
};
use crate::ao3::actions::{self, PostMode};
use crate::ao3::compose::{self, NewWorkDefaults, NewWorkInput};
use crate::ao3::parse::{SpaceMode, percent_decode, percent_encode};
use crate::error::Error;
use crate::render;
use crate::render::NavSection;
use axum::extract::Form;

/// How long a parsed `/works/new` page is reused (options change rarely).
const DEFAULTS_TTL: Duration = Duration::from_secs(600);

#[derive(Deserialize, Default)]
pub(in crate::server) struct ComposeQuery {
    ok: Option<String>,
    error: Option<String>,
}

/// Composer submission. Parsed by hand from the form body because AO3-style
/// checkbox groups repeat the same key (`warnings=…&warnings=…`), which the
/// serde form extractor cannot collect into a `Vec`.
#[derive(Default)]
pub(in crate::server) struct ComposeForm {
    title: String,
    pseud_id: String,
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
    collections: String,
    recipients: String,
    chapter_title: String,
    content: String,
    /// `publish` or `draft` (the submit button that was pressed).
    action: Option<String>,
    confirm: Option<String>,
}

impl ComposeForm {
    fn mode(&self) -> Handled<PostMode> {
        match self.action.as_deref().unwrap_or("publish").trim() {
            "publish" | "post" => Ok(PostMode::Publish),
            "draft" | "save" => Ok(PostMode::Draft),
            other => Err(AppError::BadRequest(format!(
                "unknown posting action {other:?}"
            ))),
        }
    }

    fn from_fields(fields: &HashMap<String, Vec<String>>) -> Self {
        let one = |name: &str| {
            fields
                .get(name)
                .and_then(|values| values.first())
                .cloned()
                .unwrap_or_default()
        };
        Self {
            title: one("title"),
            pseud_id: one("pseud_id"),
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
            collections: one("collections"),
            recipients: one("recipients"),
            chapter_title: one("chapter_title"),
            content: one("content"),
            action: fields
                .get("action")
                .and_then(|values| values.first())
                .cloned(),
            confirm: fields
                .get("confirm")
                .and_then(|values| values.first())
                .cloned(),
        }
    }

    fn into_input(self) -> NewWorkInput {
        NewWorkInput {
            title: self.title,
            pseud_id: self.pseud_id,
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
            collections: self.collections,
            recipients: self.recipients,
            chapter_title: self.chapter_title,
            content: self.content,
        }
    }
}

/// Decodes `application/x-www-form-urlencoded` into name -> repeated values.
fn parse_compose_form(body: &[u8]) -> Handled<ComposeForm> {
    let text = std::str::from_utf8(body)
        .map_err(|_| AppError::BadRequest("form body is not valid UTF-8".into()))?;
    let mut fields: HashMap<String, Vec<String>> = HashMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_component(key)?;
        let value = decode_component(value)?;
        fields.entry(key).or_default().push(value);
    }
    Ok(ComposeForm::from_fields(&fields))
}

fn decode_component(raw: &str) -> Handled<String> {
    let plus_decoded = raw.replace('+', " ");
    percent_decode(&plus_decoded)
        .ok_or_else(|| AppError::BadRequest("invalid percent-encoding in form body".into()))
}

fn blank_input(defaults: &NewWorkDefaults) -> NewWorkInput {
    NewWorkInput {
        pseud_id: defaults
            .default_pseud()
            .map(|pseud| pseud.value.clone())
            .unwrap_or_default(),
        rating: defaults
            .default_rating()
            .map(|rating| rating.value.clone())
            .unwrap_or_default(),
        language_id: defaults
            .default_language()
            .map(|language| language.value.clone())
            .unwrap_or_default(),
        ..NewWorkInput::default()
    }
}

/// Loads (and briefly caches) the options from AO3's own new-work form.
async fn load_defaults(state: &AppState, refresh: bool) -> Handled<NewWorkDefaults> {
    if !refresh
        && let Some((fetched_at, defaults)) = state.compose_defaults.read().await.as_ref()
        && fetched_at.elapsed() < DEFAULTS_TTL
    {
        return Ok(defaults.clone());
    }
    let html = super::super::reads::fetch_page_html(state, "/works/new", "new").await?;
    let mut defaults = compose::parse_new_work_page(&html).map_err(AppError::from)?;
    // A one-pseud account renders the byline as a hidden id with no label; fill
    // the display name from the session's pseud discovery when possible.
    if defaults
        .pseuds
        .iter()
        .any(|pseud| pseud.label.starts_with("Pseud #"))
    {
        let account = require_usable_session(
            state,
            "Sign in to AO3 on the Session page before posting a work.",
        )
        .await?;
        let pseud = {
            let _solver_guard = state.solver_lock.lock().await;
            let mut session = state.session.lock().await;
            ensure_pseud(state, &mut session, &account).await.ok()
        };
        if let Some(pseud) = pseud {
            let id = pseud.id.to_string();
            for option in &mut defaults.pseuds {
                if option.value == id {
                    option.label = pseud.name.clone();
                }
            }
        }
    }
    *state.compose_defaults.write().await = Some((Instant::now(), defaults.clone()));
    Ok(defaults)
}

async fn render_compose(
    state: &AppState,
    input: &NewWorkInput,
    errors: &[String],
    notice: Option<&str>,
) -> Handled<Html<String>> {
    let defaults = match load_defaults(state, false).await {
        Ok(defaults) => defaults,
        Err(error) => return Err(error),
    };
    let page =
        super::super::page_context(state, "New Work — AO3Instance", NavSection::Browse).await;
    Ok(Html(render::compose_page(
        &page, &defaults, input, errors, notice,
    )))
}

#[derive(Deserialize)]
pub(in crate::server) struct TargetForm {
    target_id: String,
    confirm: Option<String>,
    /// `drafts` when the action was started from the drafts list.
    #[serde(rename = "return")]
    return_path: Option<String>,
}

/// `POST /write/work/publish` — publish one draft (`PUT /works/:id/post_draft`).
pub(in crate::server) async fn publish_draft(
    State(state): State<Arc<AppState>>,
    Form(form): Form<TargetForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let target_id = require_non_empty_target(&form.target_id, "missing work id")?;
    let account =
        require_usable_session(&state, "Sign in to AO3 on the Session page first.").await?;
    state
        .governor
        .begin("publish_draft")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "publish_draft", "Work", &target_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::publish_draft(&mut session, &target_id).await
    };
    super::super::solver::sync_solver_state(&state).await;
    match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "work-published",
                    Some(&format!("work {target_id}")),
                )
                .await?;
            invalidate_user_list(&state, &account, "works").await;
            // Opening the freshly published work caches it and proves it is live.
            Ok(Redirect::to(&format!("/open/{target_id}")))
        }
        Err(error) => {
            let retry_after = record_write_failure(&state, "publish_draft", intent, &error).await?;
            let flag = if retry_after.is_some() {
                "work_publish_rate_limited"
            } else {
                "work_publish_failed"
            };
            Ok(Redirect::to(&format!("/my/works/drafts?notice={flag}")))
        }
    }
}

/// `POST /write/work/delete` — delete one work (posted or draft) on AO3.
pub(in crate::server) async fn delete_work(
    State(state): State<Arc<AppState>>,
    Form(form): Form<TargetForm>,
) -> Handled<Redirect> {
    require_writes_enabled(&state)?;
    require_confirmation(form.confirm.as_deref())?;
    let target_id = require_non_empty_target(&form.target_id, "missing work id")?;
    let account =
        require_usable_session(&state, "Sign in to AO3 on the Session page first.").await?;
    state
        .governor
        .begin("delete_work")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "delete_work", "Work", &target_id)
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::delete_work(&mut session, &target_id).await
    };
    super::super::solver::sync_solver_state(&state).await;
    match result {
        Ok(()) => {
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    "work-deleted",
                    Some(&format!("work {target_id}")),
                )
                .await?;
            if let Err(error) = state.store.delete_cached_work(&target_id).await {
                eprintln!("delete cached work after AO3 delete: {error}");
            }
            invalidate_user_list(&state, &account, "works").await;
            Ok(Redirect::to("/my/works?notice=work_deleted"))
        }
        Err(error) => {
            let retry_after = record_write_failure(&state, "delete_work", intent, &error).await?;
            let flag = if retry_after.is_some() {
                "work_delete_rate_limited"
            } else {
                "work_delete_failed"
            };
            let base = if form.return_path.as_deref() == Some("drafts") {
                "/my/works/drafts"
            } else {
                "/my/works"
            };
            Ok(Redirect::to(&format!("{base}?notice={flag}")))
        }
    }
}

/// `GET /works/new` (AO3's new-work page, re-rendered locally).
pub(in crate::server) async fn compose(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ComposeQuery>,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before posting a work.",
    )
    .await?;
    let defaults = load_defaults(&state, query.error.is_some()).await?;
    let input = blank_input(&defaults);
    let errors: Vec<String> = query.error.iter().cloned().collect();
    let page =
        super::super::page_context(&state, "New Work — AO3Instance", NavSection::Browse).await;
    Ok(Html(render::compose_page(
        &page,
        &defaults,
        &input,
        &errors,
        query.ok.as_deref(),
    )))
}

/// `POST /write/work/preview` — local validation + confirmation, no network
/// write and (normally) no network read either.
pub(in crate::server) async fn write_work_preview(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Html<String>> {
    require_writes_enabled(&state)?;
    let form = parse_compose_form(&body)?;
    require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before posting a work.",
    )
    .await?;
    let mode = form.mode()?;
    let input = form.into_input();
    let errors = compose::validate(&input);
    if !errors.is_empty() {
        return render_compose(&state, &input, &errors, None).await;
    }
    let defaults = load_defaults(&state, false).await?;
    let pseud_label = defaults
        .pseuds
        .iter()
        .find(|pseud| pseud.value == input.pseud_id)
        .map(|pseud| pseud.label.clone())
        .unwrap_or_else(|| format!("pseud {}", input.pseud_id));
    let page =
        super::super::page_context(&state, "Confirm Work — AO3Instance", NavSection::Browse).await;
    let mode_name = match mode {
        PostMode::Publish => "publish",
        PostMode::Draft => "draft",
    };
    Ok(Html(render::confirm_work_page(
        &page,
        &defaults,
        &input,
        mode_name,
        &pseud_label,
    )))
}

/// `POST /write/work` — the one real create request.
pub(in crate::server) async fn write_work(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Handled<Response> {
    require_writes_enabled(&state)?;
    let form = parse_compose_form(&body)?;
    require_confirmation(form.confirm.as_deref())?;
    let mode = form.mode()?;
    let account = require_usable_session(
        &state,
        "Sign in to AO3 on the Session page before posting a work.",
    )
    .await?;
    let input = form.into_input();
    let errors = compose::validate(&input);
    if !errors.is_empty() {
        let html = render_compose(&state, &input, &errors, None).await?;
        return Ok(html.into_response());
    }

    state
        .governor
        .begin("post_work")
        .await
        .map_err(AppError::from)?;
    let intent = state
        .store
        .record_intent(&account, "post_work", "Work", "new")
        .await?;
    let result = {
        let mut session = state.session.lock().await;
        actions::post_work(&mut session, &input, mode).await
    };
    super::super::solver::sync_solver_state(&state).await;
    match result {
        Ok(posted) => {
            let work_id = posted.work_id.unwrap_or_default();
            state
                .store
                .finish_intent(
                    intent,
                    "done",
                    if posted.draft {
                        "work-draft"
                    } else {
                        "work-posted"
                    },
                    Some(&format!("work {work_id}")),
                )
                .await?;
            if posted.draft {
                let message = format!(
                    "Draft saved on AO3 as work {work_id}. AO3 deletes drafts after about 29 days."
                );
                Ok(Redirect::to(&format!(
                    "/session?ok={}",
                    percent_encode(&message, SpaceMode::Percent20)
                ))
                .into_response())
            } else {
                // Opening the new work caches it and proves it is live.
                Ok(Redirect::to(&format!("/open/{work_id}")).into_response())
            }
        }
        Err(error) => {
            if matches!(error, Error::Network(_)) {
                // The request may have reached AO3: never encourage a blind retry.
                state.governor.record_outcome("post_work", None).await;
                state
                    .store
                    .finish_intent(
                        intent,
                        "ambiguous",
                        "ambiguous",
                        Some("posting timed out; verify the work on AO3 before trying again"),
                    )
                    .await?;
                let html = render_compose(
                    &state,
                    &input,
                    &[format!(
                        "Posting timed out ({error}). The work may already exist on AO3 — check \
                         your works list there before retrying."
                    )],
                    None,
                )
                .await?;
                return Ok(html.into_response());
            }
            record_write_failure(&state, "post_work", intent, &error).await?;
            let html = render_compose(&state, &input, &[error.to_string()], None).await?;
            Ok(html.into_response())
        }
    }
}
