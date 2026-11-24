//! Offline library browse, live AO3 tag works pages, and tag autocomplete.

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderValue, header},
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::render::{self, NavSection};
use crate::server::AppState;
use crate::server::error::{AppError, Handled};

use super::{all, first, nonempty, query_pairs, tag_list};

#[derive(Deserialize)]
pub(in crate::server) struct BrowseQuery {
    rating: Option<String>,
    author: Option<String>,
    fandom: Option<String>,
    tag: Option<String>,
    language: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
}

pub(in crate::server) async fn browse(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BrowseQuery>,
) -> Handled<Html<String>> {
    render_browse(&state, query, None).await
}

pub(in crate::server) async fn tag_browse(
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
    let cache_account = crate::server::cache::cache_account(&state).await;

    let fetched = match fetch_tag_page(&state, &cache_account, &tag_path, refresh).await {
        Some(page_html) => Some(page_html),
        None => {
            // Non-canonical / unwrangled tags may 404 on `/tags/<tag>/works`;
            // fall back to AO3's `/works/search` endpoint.
            let search_path = live_query.path(page);
            crate::server::cache::fetch_cached_optional(
                &state,
                &cache_account,
                &search_path,
                true,
                refresh,
            )
            .await
        }
    };

    if let Some(fetched) = fetched
        && let Ok(mut results) = crate::ao3::search::parse_search_results(&fetched.body)
    {
        if results.facets.is_empty() && !results.works.is_empty() {
            results.facets = crate::ao3::search::LiveTagFacets::from_works(&results.works);
        }
        let page_ctx = crate::server::page_context(
            &state,
            &format!("{tag} — AO3Instance"),
            NavSection::Fandoms,
        )
        .await;
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
) -> Option<crate::server::cache::CachedHtml> {
    match crate::server::cache::fetch_cached_page(state, cache_account, path, true, refresh).await {
        Ok(fetched) => Some(fetched),
        Err(crate::server::cache::PageFetchError::Status {
            status: 301 | 302 | 303 | 307 | 308,
            location: Some(location),
        }) => {
            // Follow one canonical-tag redirect (e.g. synonym -> canonical tag works page).
            let next_path = extract_tag_redirect_path(&location)?;
            crate::server::cache::fetch_cached_optional(
                state,
                cache_account,
                &next_path,
                true,
                refresh,
            )
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
pub(in crate::server) struct AutocompleteQuery {
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
pub(in crate::server) async fn autocomplete(
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
        crate::server::cache::fetch_cached_optional(&state, "", &upstream_path, false, false).await
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

async fn render_browse(
    state: &AppState,
    query: BrowseQuery,
    tag_override: Option<String>,
) -> Handled<Html<String>> {
    if tag_override.as_deref().is_some_and(str::is_empty) {
        return Err(AppError::BadRequest("tag is empty".into()));
    }
    let (account, _) = crate::server::session_identity(state).await;
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
    let page = crate::server::page_context(state, &title, NavSection::Browse).await;
    Ok(Html(render::browse_page(&page, &result, &filter)))
}
