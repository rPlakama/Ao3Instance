//! Import public AO3 site skins as local themes.

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use crate::render::{self, NavSection};

#[derive(Deserialize)]
pub(super) struct ThemesQuery {
    applied: Option<String>,
    error: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn themes(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ThemesQuery>,
) -> Handled<Html<String>> {
    let refresh = query.refresh.as_deref() == Some("1");
    let html = fetch_public_page(&state, "/skins", refresh).await;
    let skins = html
        .as_deref()
        .map(crate::ao3::skins::parse_skin_list)
        .unwrap_or_default();
    let page = super::page_context(&state, "Themes — AO3Instance", NavSection::Session).await;
    let notice = query.applied.as_deref().map(|_| "Public skin imported.");
    Ok(Html(render::themes_page(
        &page,
        &skins,
        notice,
        query.error.as_deref(),
    )))
}

#[derive(Deserialize)]
pub(super) struct ApplyForm {
    href: String,
}

pub(super) async fn apply(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ApplyForm>,
) -> Handled<Redirect> {
    let rest = form.href.trim().strip_prefix("/skins/").unwrap_or_default();
    if rest.is_empty() || !rest.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "Expected an AO3 public skin path like /skins/942.".into(),
        ));
    }
    let path = format!("/skins/{rest}");
    let html = fetch_public_page(&state, &path, false).await.ok_or_else(|| {
        AppError::BadRequest(
            "AO3 did not return the public skin; check Diagnostics and try again.".into(),
        )
    })?;
    let css = crate::ao3::skins::parse_skin_css(&html).ok_or_else(|| {
        AppError::BadRequest(
            "That public skin has no custom CSS to import (wizard-only skins are not supported yet)."
                .into(),
        )
    })?;
    state.store.set_setting("display.skin", "public").await?;
    state.store.set_setting("display.skin_css", &css).await?;
    state.store.set_setting("display.skin_title", &path).await?;
    *state.skin.write().await = "public".to_string();
    *state.skin_css.write().await = Some(css);
    Ok(Redirect::to("/themes?applied=1"))
}

pub(super) async fn clear(State(state): State<Arc<AppState>>) -> Handled<Redirect> {
    state.store.set_setting("display.skin", "default").await?;
    state.store.set_setting("display.skin_css", "").await?;
    state.store.set_setting("display.skin_title", "").await?;
    *state.skin.write().await = "default".to_string();
    *state.skin_css.write().await = None;
    Ok(Redirect::to("/themes"))
}

async fn fetch_public_page(state: &AppState, path: &str, refresh: bool) -> Option<String> {
    super::cache::fetch_cached_optional(state, "", path, false, refresh)
        .await
        .map(|fetched| fetched.body)
}
