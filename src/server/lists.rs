//! Offline list pages: browse, fandoms, dashboard and the account ledgers.

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderValue, header},
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

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
    let tag_path = live_query.tag_works_path(
        &tag,
        nonempty(&filter.words_from).as_deref(),
        nonempty(&filter.words_to).as_deref(),
        page,
    );
    let refresh = first(&pairs, "refresh").as_deref() == Some("1");
    let cache_account = super::cache::cache_account(&state).await;

    let fetched = match fetch_tag_page(&state, &cache_account, &tag_path, refresh).await {
        Some(page_html) => Some(page_html),
        None => {
            // Non-canonical / unwrangled tags may 404 on `/tags/<tag>/works`;
            // fall back to AO3's `/works/search` endpoint.
            let search_path = live_query.path(page);
            super::cache::fetch_cached_optional(&state, &cache_account, &search_path, true, refresh)
                .await
        }
    };

    if let Some(fetched) = fetched
        && let Ok(mut results) = crate::ao3::search::parse_search_results(&fetched.body)
    {
        if results.facets.is_empty() && !results.works.is_empty() {
            results.facets = crate::ao3::search::LiveTagFacets::from_works(&results.works);
        }
        let page_ctx =
            super::page_context(&state, &format!("{tag} — AO3Instance"), NavSection::Fandoms).await;
        return Ok(Html(render::live_tag_page(
            &page_ctx,
            &tag,
            page,
            &results,
            &filter,
            Some((fetched.from_cache, fetched.fetched_at)),
        )));
    }
    // Offline/cached fallback: the tag page still shows local works.
    render_browse(&state, browse_query(page), Some(tag)).await
}

async fn fetch_tag_page(
    state: &AppState,
    cache_account: &str,
    path: &str,
    refresh: bool,
) -> Option<super::cache::CachedHtml> {
    match super::cache::fetch_cached_page(state, cache_account, path, true, refresh).await {
        Ok(fetched) => Some(fetched),
        Err(super::cache::PageFetchError::Status {
            status: 301 | 302 | 303 | 307 | 308,
            location: Some(location),
        }) => {
            // Follow one canonical-tag redirect (e.g. synonym -> canonical tag works page).
            let next_path = extract_tag_redirect_path(&location)?;
            super::cache::fetch_cached_optional(state, cache_account, &next_path, true, refresh)
                .await
        }
        Err(_) => None,
    }
}

fn extract_tag_redirect_path(location: &str) -> Option<String> {
    let trimmed = location.trim();
    if trimmed.starts_with("/tags/") {
        return Some(trimmed.to_string());
    }
    let after_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))?;
    let (_, rest) = after_scheme.split_once('/')?;
    let path = format!("/{rest}");
    path.starts_with("/tags/").then_some(path)
}

#[derive(Deserialize)]
pub(super) struct AutocompleteQuery {
    term: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutocompleteItem {
    id: String,
    name: String,
}

/// Live tag autocomplete proxy (`GET /autocomplete/{kind}?term=...`), backed by
/// AO3's `/autocomplete/{kind}.json` (30-min SQLite cache) with an offline
/// fallback over cached works' tags.
pub(super) async fn autocomplete(
    State(state): State<Arc<AppState>>,
    Path(kind): Path<String>,
    Query(query): Query<AutocompleteQuery>,
) -> Handled<Response> {
    let kind = match kind.as_str() {
        "tag" | "fandom" | "character" | "relationship" | "freeform" => kind.as_str(),
        _ => return Err(AppError::BadRequest("unsupported autocomplete kind".into())),
    };
    let term = query.term.as_deref().unwrap_or_default().trim();
    if term.is_empty() || term.len() > 200 {
        return Ok(json_response("[]"));
    }

    let encoded = crate::ao3::parse::percent_encode(term, crate::ao3::parse::SpaceMode::Percent20);
    let upstream_path = format!("/autocomplete/{kind}.json?term={encoded}");
    if let Some(fetched) =
        super::cache::fetch_cached_optional(&state, "", &upstream_path, false, false).await
    {
        let raw = crate::ao3::solver::extract_json_body(&fetched.body);
        if let Ok(mut items) = serde_json::from_str::<Vec<AutocompleteItem>>(&raw) {
            items.truncate(25);
            let body = serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string());
            return Ok(json_response(&body));
        }
    }

    // Offline / upstream-unavailable fallback: search locally cached tags.
    let hits = state.store.search_tags(term, 50).await.unwrap_or_default();
    let mut seen = HashSet::new();
    let items: Vec<AutocompleteItem> = hits
        .into_iter()
        .filter(|hit| kind == "tag" || hit.kind == kind)
        .filter(|hit| seen.insert(hit.name.clone()))
        .take(15)
        .map(|hit| AutocompleteItem {
            id: hit.name.clone(),
            name: hit.name,
        })
        .collect();
    let body = serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string());
    Ok(json_response(&body))
}

fn json_response(body: &str) -> Response {
    let mut response = body.to_string().into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    response
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

#[derive(Deserialize)]
pub(super) struct FandomsQuery {
    local: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn fandoms(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FandomsQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    if !local
        && let Some(fetched) =
            super::cache::fetch_cached_optional(&state, "", "/media", false, refresh).await
    {
        let media = crate::ao3::account_lists::parse_media_index(&fetched.body);
        if !media.is_empty() {
            let page =
                super::page_context(&state, "Fandoms — AO3Instance", NavSection::Fandoms).await;
            return Ok(Html(render::live_media_page(
                &page,
                &media,
                Some((fetched.from_cache, fetched.fetched_at)),
            )));
        }
    }
    let fandoms = state.store.list_fandoms().await?;
    let page = super::page_context(&state, "Fandoms — AO3Instance", NavSection::Fandoms).await;
    Ok(Html(render::fandoms_page(&page, &fandoms)))
}

#[derive(Deserialize)]
pub(super) struct RefreshQuery {
    refresh: Option<String>,
}

pub(super) async fn medium_fandoms(
    State(state): State<Arc<AppState>>,
    Path(medium): Path<String>,
    Query(query): Query<RefreshQuery>,
) -> Handled<Html<String>> {
    let refresh = query.refresh.as_deref() == Some("1");
    let encoded =
        crate::ao3::parse::percent_encode(&medium, crate::ao3::parse::SpaceMode::Percent20)
            .replace("%2A", "*");
    let path = format!("/media/{encoded}/fandoms");
    if let Some(fetched) =
        super::cache::fetch_cached_optional(&state, "", &path, false, refresh).await
    {
        let data = crate::ao3::account_lists::parse_medium_fandoms(&fetched.body);
        let title = if data.title.is_empty() {
            format!("{medium} — AO3Instance")
        } else {
            format!("{} — AO3Instance", data.title)
        };
        let page = super::page_context(&state, &title, NavSection::Fandoms).await;
        return Ok(Html(render::live_medium_fandoms_page(
            &page,
            &medium,
            &data,
            Some((fetched.from_cache, fetched.fetched_at)),
        )));
    }
    let fandoms = state.store.list_fandoms().await?;
    let page = super::page_context(
        &state,
        &format!("{medium} — AO3Instance"),
        NavSection::Fandoms,
    )
    .await;
    Ok(Html(render::fandoms_page(&page, &fandoms)))
}

pub(super) async fn dashboard(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RefreshQuery>,
) -> Handled<Html<String>> {
    let refresh = query.refresh.as_deref() == Some("1");
    let (account, _) = super::session_identity(&state).await;
    let cache_acct = super::cache::cache_account(&state).await;
    let stats = state.store.cache_stats().await?;
    let subscriptions = state.store.list_subscriptions(&account).await?;
    let bookmarks = state.store.list_bookmarks(&account).await?;
    let reading = state.store.list_reading_history(&account).await?;
    let cached_works = state.store.cached_work_ids().await?;

    let (live_subs, live_bm, live_read) = if !cache_acct.is_empty() {
        let user_seg =
            crate::ao3::parse::percent_encode(&cache_acct, crate::ao3::parse::SpaceMode::Percent20);
        let subs_path = format!("/users/{user_seg}/subscriptions");
        let bm_path = format!("/users/{user_seg}/bookmarks");
        let read_path = format!("/users/{user_seg}/readings");
        let live_subs =
            super::cache::fetch_cached_optional(&state, &cache_acct, &subs_path, true, refresh)
                .await
                .and_then(|fetched| crate::ao3::account_lists::parse_subscriptions(&fetched.body));
        let live_bm =
            super::cache::fetch_cached_optional(&state, &cache_acct, &bm_path, true, refresh)
                .await
                .and_then(|fetched| crate::ao3::account_lists::parse_bookmarks(&fetched.body));
        let live_read =
            super::cache::fetch_cached_optional(&state, &cache_acct, &read_path, true, refresh)
                .await
                .and_then(|fetched| crate::ao3::account_lists::parse_readings(&fetched.body));
        (live_subs, live_bm, live_read)
    } else {
        (None, None, None)
    };

    let live = render::DashboardLive {
        subscriptions: live_subs.as_ref(),
        bookmarks: live_bm.as_ref(),
        readings: live_read.as_ref(),
        cached_works: &cached_works,
    };
    let page = super::page_context(&state, "Dashboard — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::dashboard_page(
        &page,
        &stats,
        &subscriptions,
        &bookmarks,
        &reading,
        &live,
    )))
}

#[derive(Deserialize)]
pub(super) struct SubscriptionsQuery {
    #[serde(rename = "type")]
    kind: Option<String>,
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn subscriptions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubscriptionsQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    let page_num = query.page.unwrap_or(1).max(1);
    let filter_type = match query.kind.as_deref() {
        Some("works") => Some("works"),
        Some("series") => Some("series"),
        Some("users") => Some("users"),
        _ => None,
    };
    let cache_acct = super::cache::cache_account(&state).await;
    if !local && !cache_acct.is_empty() {
        let user_seg =
            crate::ao3::parse::percent_encode(&cache_acct, crate::ao3::parse::SpaceMode::Percent20);
        let mut qs = Vec::new();
        if let Some(kind) = filter_type {
            qs.push(format!("type={kind}"));
        }
        if page_num > 1 {
            qs.push(format!("page={page_num}"));
        }
        let path = if qs.is_empty() {
            format!("/users/{user_seg}/subscriptions")
        } else {
            format!("/users/{user_seg}/subscriptions?{}", qs.join("&"))
        };
        if let Some(fetched) =
            super::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh).await
            && let Some(parsed) = crate::ao3::account_lists::parse_subscriptions(&fetched.body)
        {
            let cached_works = state.store.cached_work_ids().await?;
            let page =
                super::page_context(&state, "Subscriptions — AO3Instance", NavSection::Dashboard)
                    .await;
            return Ok(Html(render::live_subscriptions_page(
                &page,
                &parsed,
                page_num,
                filter_type,
                &cached_works,
                Some((fetched.from_cache, fetched.fetched_at)),
            )));
        }
    }
    let (account, _) = super::session_identity(&state).await;
    let entries = state.store.list_subscriptions(&account).await?;
    let page =
        super::page_context(&state, "Subscriptions — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::subscriptions_page(&page, &entries)))
}

#[derive(Deserialize)]
pub(super) struct BookmarksQuery {
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn bookmarks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BookmarksQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    let page_num = query.page.unwrap_or(1).max(1);
    let cache_acct = super::cache::cache_account(&state).await;
    if !local && !cache_acct.is_empty() {
        let user_seg =
            crate::ao3::parse::percent_encode(&cache_acct, crate::ao3::parse::SpaceMode::Percent20);
        let path = if page_num > 1 {
            format!("/users/{user_seg}/bookmarks?page={page_num}")
        } else {
            format!("/users/{user_seg}/bookmarks")
        };
        if let Some(fetched) =
            super::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh).await
            && let Some(parsed) = crate::ao3::account_lists::parse_bookmarks(&fetched.body)
        {
            let page =
                super::page_context(&state, "Bookmarks — AO3Instance", NavSection::Dashboard).await;
            return Ok(Html(render::live_bookmarks_page(
                &page,
                &parsed,
                page_num,
                Some((fetched.from_cache, fetched.fetched_at)),
            )));
        }
    }
    let (account, _) = super::session_identity(&state).await;
    let entries = state.store.list_bookmarks(&account).await?;
    let page = super::page_context(&state, "Bookmarks — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::bookmarks_page(&page, &entries)))
}

#[derive(Deserialize)]
pub(super) struct ReadingQuery {
    show: Option<String>,
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn reading(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReadingQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    let to_read = query.show.as_deref() == Some("to-read");
    let page_num = query.page.unwrap_or(1).max(1);
    let (account, _) = super::session_identity(&state).await;
    let local_entries = state.store.list_reading_history(&account).await?;
    let cache_acct = super::cache::cache_account(&state).await;
    if !local && !cache_acct.is_empty() {
        let user_seg =
            crate::ao3::parse::percent_encode(&cache_acct, crate::ao3::parse::SpaceMode::Percent20);
        let mut qs = Vec::new();
        if to_read {
            qs.push("show=to-read".to_string());
        }
        if page_num > 1 {
            qs.push(format!("page={page_num}"));
        }
        let path = if qs.is_empty() {
            format!("/users/{user_seg}/readings")
        } else {
            format!("/users/{user_seg}/readings?{}", qs.join("&"))
        };
        if let Some(fetched) =
            super::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh).await
            && let Some(parsed) = crate::ao3::account_lists::parse_readings(&fetched.body)
        {
            let progress_map: std::collections::HashMap<String, (i64, i64)> = local_entries
                .iter()
                .filter_map(|entry| {
                    let chapter = entry.progress_chapter?;
                    let pct = entry.progress_pct.unwrap_or(0);
                    Some((entry.work_id.clone(), (chapter, pct)))
                })
                .collect();
            let title = if to_read {
                "Marked for Later — AO3Instance"
            } else {
                "History — AO3Instance"
            };
            let page = super::page_context(&state, title, NavSection::Dashboard).await;
            return Ok(Html(render::live_readings_page(
                &page,
                &parsed,
                page_num,
                to_read,
                &progress_map,
                Some((fetched.from_cache, fetched.fetched_at)),
            )));
        }
    }
    let page = super::page_context(&state, "Reading — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::reading_page(&page, &local_entries)))
}

pub(super) async fn about(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let page = super::page_context(&state, "About — AO3Instance", NavSection::About).await;
    Ok(Html(render::about_page(&page)))
}

#[derive(Deserialize)]
pub(super) struct TagSearchQuery {
    q: Option<String>,
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn tag_search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TagSearchQuery>,
) -> Handled<Html<String>> {
    let raw = query.q.unwrap_or_default();
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    let page_num = query.page.unwrap_or(1).max(1);
    if !local && !raw.trim().is_empty() {
        let encoded =
            crate::ao3::parse::percent_encode(raw.trim(), crate::ao3::parse::SpaceMode::Percent20);
        let path = if page_num > 1 {
            format!("/tags/search?tag_search%5Bname%5D={encoded}&page={page_num}")
        } else {
            format!("/tags/search?tag_search%5Bname%5D={encoded}")
        };
        if let Some(fetched) =
            super::cache::fetch_cached_optional(&state, "", &path, false, refresh).await
        {
            let results = crate::ao3::account_lists::parse_tag_search(&fetched.body);
            let page =
                super::page_context(&state, "Tag Search — AO3Instance", NavSection::Fandoms).await;
            return Ok(Html(render::live_tag_search_page(
                &page,
                &raw,
                &results,
                page_num,
                Some((fetched.from_cache, fetched.fetched_at)),
            )));
        }
    }
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
