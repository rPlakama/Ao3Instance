//! Offline list pages: browse, fandoms, dashboard and the account ledgers.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use crate::render::{self, NavSection};

#[derive(Deserialize)]
pub(super) struct BrowseQuery {
    rating: Option<String>,
    author: Option<String>,
    fandom: Option<String>,
    tag: Option<String>,
    language: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
}

pub(super) async fn browse(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BrowseQuery>,
) -> Handled<Html<String>> {
    render_browse(&state, query, None).await
}

pub(super) async fn tag_browse(
    State(state): State<Arc<AppState>>,
    Path(tag): Path<String>,
    Query(query): Query<BrowseQuery>,
) -> Handled<Html<String>> {
    render_browse(&state, query, Some(tag)).await
}

async fn render_browse(
    state: &AppState,
    query: BrowseQuery,
    tag_override: Option<String>,
) -> Handled<Html<String>> {
    if tag_override.as_deref().is_some_and(str::is_empty) {
        return Err(AppError::BadRequest("tag is empty".into()));
    }
    let (account, _) = super::session_identity(state).await;
    let mut filter = render::filter_from(
        query.rating,
        query.author,
        query.fandom,
        query.tag,
        query.language,
        query.status,
        query.sort,
        query.page,
    );
    if let Some(tag) = tag_override {
        filter.tag = Some(tag);
    }
    let result = state.store.browse_works(&account, &filter).await?;
    let title = filter
        .tag
        .as_deref()
        .or(filter.fandom.as_deref())
        .map_or_else(
            || "Browse — AO3Instance".to_string(),
            |scope| format!("{scope} — AO3Instance"),
        );
    let page = super::page_context(state, &title, NavSection::Browse).await;
    Ok(Html(render::browse_page(&page, &result, &filter)))
}

pub(super) async fn fandoms(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let fandoms = state.store.list_fandoms().await?;
    let page = super::page_context(&state, "Fandoms — AO3Instance", NavSection::Fandoms).await;
    Ok(Html(render::fandoms_page(&page, &fandoms)))
}

pub(super) async fn dashboard(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let (account, _) = super::session_identity(&state).await;
    let stats = state.store.cache_stats().await?;
    let subscriptions = state.store.list_subscriptions(&account).await?;
    let bookmarks = state.store.list_bookmarks(&account).await?;
    let reading = state.store.list_reading_history(&account).await?;
    let page = super::page_context(&state, "Dashboard — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::dashboard_page(
        &page,
        &stats,
        &subscriptions,
        &bookmarks,
        &reading,
    )))
}

pub(super) async fn subscriptions(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let (account, _) = super::session_identity(&state).await;
    let entries = state.store.list_subscriptions(&account).await?;
    let page =
        super::page_context(&state, "Subscriptions — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::subscriptions_page(&page, &entries)))
}

pub(super) async fn bookmarks(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let (account, _) = super::session_identity(&state).await;
    let entries = state.store.list_bookmarks(&account).await?;
    let page = super::page_context(&state, "Bookmarks — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::bookmarks_page(&page, &entries)))
}

pub(super) async fn reading(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let (account, _) = super::session_identity(&state).await;
    let entries = state.store.list_reading_history(&account).await?;
    let page = super::page_context(&state, "Reading — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::reading_page(&page, &entries)))
}

pub(super) async fn about(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let page = super::page_context(&state, "About — AO3Instance", NavSection::About).await;
    Ok(Html(render::about_page(&page)))
}
