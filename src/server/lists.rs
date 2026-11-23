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
    local: Option<String>,
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
    if query.local.as_deref() == Some("1") {
        return render_browse(&state, query, Some(tag)).await;
    }
    let page = query.page.unwrap_or(1).max(1);
    let encoded = crate::ao3::parse::percent_encode(&tag, crate::ao3::parse::SpaceMode::Percent20);
    let path = if page > 1 {
        format!("/tags/{encoded}/works?page={page}")
    } else {
        format!("/tags/{encoded}/works")
    };
    if let Some(body) = fetch_live_tag_page(&state, &path).await
        && let Ok(results) = crate::ao3::search::parse_search_results(&body)
    {
        let page_ctx =
            super::page_context(&state, &format!("{tag} — AO3Instance"), NavSection::Fandoms).await;
        return Ok(Html(render::live_tag_page(&page_ctx, &tag, page, &results)));
    }
    // Offline/cached fallback: the tag page still shows local works.
    render_browse(&state, query, Some(tag)).await
}

async fn fetch_live_tag_page(state: &AppState, path: &str) -> Option<String> {
    let response = {
        let mut session = state.session.lock().await;
        session
            .client_mut()
            .get_with_timeout(path, std::time::Duration::from_secs(10))
            .await
            .ok()?
    };
    if response.status == 200 {
        return Some(response.body);
    }
    let available = {
        let reachable = state.solver_status.read().await.reachable;
        let session = state.session.lock().await;
        reachable && session.solver_mode() != crate::config::SolverMode::Off
    };
    if available {
        super::solver::solve_search(state, path, false).await.ok()
    } else {
        None
    }
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

#[derive(Deserialize)]
pub(super) struct TagSearchQuery {
    q: Option<String>,
}

pub(super) async fn tag_search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TagSearchQuery>,
) -> Handled<Html<String>> {
    let raw = query.q.unwrap_or_default();
    let hits = state.store.search_tags(&raw, 100).await?;
    let page = super::page_context(&state, "Tag Search — AO3Instance", NavSection::Fandoms).await;
    Ok(Html(render::tag_search_page(&page, &raw, &hits)))
}

#[derive(Deserialize)]
pub(super) struct HistoryQuery {
    range: Option<String>,
}

pub(super) async fn history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HistoryQuery>,
) -> Handled<Html<String>> {
    let range = query.range.as_deref().unwrap_or("week");
    let since = match range {
        "today" => crate::time::now_secs() - 86_400,
        "all" => 0,
        _ => crate::time::now_secs() - 7 * 86_400,
    };
    let entries = state.store.recent_accesses(200, since).await?;
    let page = super::page_context(&state, "History — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::history_page(&page, &entries, range)))
}
