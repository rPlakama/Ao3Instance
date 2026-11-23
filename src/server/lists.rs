//! Offline list pages: browse, fandoms, dashboard and the account ledgers.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
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
    RawQuery(raw): RawQuery,
) -> Handled<Html<String>> {
    let pairs = query_pairs(raw.as_deref().unwrap_or_default());
    let page = first(&pairs, "page")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    if first(&pairs, "local").as_deref() == Some("1") {
        return render_browse(&state, browse_query(page), Some(tag)).await;
    }

    let sort = first(&pairs, "sort").unwrap_or_else(|| "date".to_string());
    let direction = if sort == "title" { "asc" } else { "desc" };
    let filter = render::TagFilter {
        sort,
        direction: direction.to_string(),
        query: first(&pairs, "q").unwrap_or_default(),
        include: all(&pairs, "include_tag"),
        exclude: all(&pairs, "exclude_tag"),
        include_fandoms: first(&pairs, "include_fandoms").unwrap_or_default(),
        include_characters: first(&pairs, "include_characters").unwrap_or_default(),
        include_relationships: first(&pairs, "include_relationships").unwrap_or_default(),
        include_additional: first(&pairs, "include_additional").unwrap_or_default(),
        other_include: first(&pairs, "other_tags_include").unwrap_or_default(),
        exclude_fandoms: first(&pairs, "exclude_fandoms").unwrap_or_default(),
        exclude_characters: first(&pairs, "exclude_characters").unwrap_or_default(),
        exclude_relationships: first(&pairs, "exclude_relationships").unwrap_or_default(),
        exclude_additional: first(&pairs, "exclude_additional").unwrap_or_default(),
        other_exclude: first(&pairs, "other_tags_exclude").unwrap_or_default(),
        complete: first(&pairs, "complete").unwrap_or_default(),
        crossover: first(&pairs, "crossover").unwrap_or_default(),
        words_from: first(&pairs, "words_from").unwrap_or_default(),
        words_to: first(&pairs, "words_to").unwrap_or_default(),
        language: first(&pairs, "language").unwrap_or_default(),
    };
    let mut include_tags = filter.include.clone();
    for value in [
        &filter.include_fandoms,
        &filter.include_characters,
        &filter.include_relationships,
        &filter.include_additional,
        &filter.other_include,
    ] {
        include_tags.extend(tag_list(value));
    }
    include_tags.push(tag.clone());
    let mut exclude_tags = filter.exclude.clone();
    for value in [
        &filter.exclude_fandoms,
        &filter.exclude_characters,
        &filter.exclude_relationships,
        &filter.exclude_additional,
        &filter.other_exclude,
    ] {
        exclude_tags.extend(tag_list(value));
    }
    let words = match (
        filter.words_from.trim().parse::<u64>().ok(),
        filter.words_to.trim().parse::<u64>().ok(),
    ) {
        (Some(from), Some(to)) => Some(format!("{from}-{to}")),
        (Some(from), None) => Some(format!(">{from}")),
        (None, Some(to)) => Some(format!("<{to}")),
        (None, None) => None,
    };
    let live_query = crate::ao3::search::LiveSearchQuery {
        query: filter.query.clone(),
        include_tags,
        exclude_tags,
        complete: nonempty(&filter.complete),
        crossover: nonempty(&filter.crossover),
        word_count: words,
        language: nonempty(&filter.language),
        sort: Some(filter.sort.clone()),
        direction: Some(filter.direction.clone()),
        page,
        ..crate::ao3::search::LiveSearchQuery::default()
    };
    let path = live_query.path(page);
    if let Some(body) = fetch_live_tag_page(&state, &path).await
        && let Ok(results) = crate::ao3::search::parse_search_results(&body)
    {
        let page_ctx =
            super::page_context(&state, &format!("{tag} — AO3Instance"), NavSection::Fandoms).await;
        return Ok(Html(render::live_tag_page(
            &page_ctx, &tag, page, &results, &filter,
        )));
    }
    // Offline/cached fallback: the tag page still shows local works.
    render_browse(&state, browse_query(page), Some(tag)).await
}

fn browse_query(page: i64) -> BrowseQuery {
    BrowseQuery {
        rating: None,
        author: None,
        fandom: None,
        tag: None,
        language: None,
        status: None,
        sort: None,
        page: Some(page),
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

fn tag_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

fn query_pairs(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |text: &str| crate::ao3::parse::percent_decode(&text.replace('+', " "));
            Some((decode(key)?, decode(value)?))
        })
        .collect()
}

fn first(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

fn all(pairs: &[(String, String)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
        .collect()
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
