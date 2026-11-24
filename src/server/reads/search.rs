//! Live AO3 Work Search and local offline cache search handlers.

use std::sync::Arc;

use axum::{
    extract::{Form, Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::render::{self, NavSection};
use crate::server::error::{AppError, Handled};
use crate::server::{AppState, session_identity};

#[derive(Deserialize)]
pub(in crate::server) struct SearchQuery {
    q: Option<String>,
    page: Option<i64>,
    title: Option<String>,
    creator: Option<String>,
    fandom: Option<String>,
    characters: Option<String>,
    relationships: Option<String>,
    additional_tags: Option<String>,
    rating: Option<String>,
    warning: Option<String>,
    category: Option<String>,
    language: Option<String>,
    words_min: Option<String>,
    words_max: Option<String>,
    kudos_min: Option<String>,
    comments_min: Option<String>,
    single_chapter: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
    edit_search: Option<String>,
    date: Option<String>,
    complete: Option<String>,
    crossover: Option<String>,
    hits_min: Option<String>,
    bookmarks_min: Option<String>,
    include_tags: Option<String>,
    exclude_tags: Option<String>,
    local: Option<String>,
    solved: Option<String>,
    refresh: Option<String>,
}

pub(in crate::server) async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Handled<Html<String>> {
    perform_search(state.as_ref(), query).await
}

/// The AO3-shaped form posts here. AO3's own web search endpoint is GET-only,
/// so the server issues exactly one literal GET upstream for this POST.
pub(in crate::server) async fn search_post(
    State(state): State<Arc<AppState>>,
    Form(query): Form<SearchQuery>,
) -> Handled<Html<String>> {
    perform_search(state.as_ref(), query).await
}

async fn perform_search(app: &AppState, query: SearchQuery) -> Handled<Html<String>> {
    let raw = query.q.clone().unwrap_or_default();
    let local = query.local.as_deref() == Some("1");

    // Local-cache mode stays available as an explicit offline fallback.
    if local {
        let filter = render::search_filter_from(
            query.title,
            query.creator,
            query.fandom,
            query.characters,
            query.relationships,
            query.additional_tags,
            query.rating,
            query.warning,
            query.category,
            query.language,
            query.words_min,
            query.words_max,
            query.kudos_min,
            query.comments_min,
            query.single_chapter,
            query.sort,
            query.direction,
        );
        let submitted = !raw.trim().is_empty() || filter.has_filters();
        if submitted && query.edit_search.as_deref() != Some("true") {
            let result = app
                .store
                .search_works_filtered(&raw, &filter, query.page.unwrap_or(1), 20)
                .await?;
            let page =
                crate::server::page_context(app, "Local Search — AO3Instance", NavSection::Search)
                    .await;
            return Ok(Html(render::search_results_page(
                &page, &raw, &result, &filter,
            )));
        }
        let (account, _) = session_identity(app).await;
        let facets = app
            .store
            .browse_works(&account, &crate::store::WorkFilter::default())
            .await?;
        let page =
            crate::server::page_context(app, "Local Search — AO3Instance", NavSection::Search)
                .await;
        return Ok(Html(render::search_form_page(
            &page, &raw, &facets, &filter,
        )));
    }

    let live = live_query_from(&query, &raw);
    let edit = query.edit_search.as_deref() == Some("true");

    // A search solved by the local browser is rendered from the captured page
    // rather than immediately retried through the direct client.
    if query.solved.as_deref() == Some("1") {
        let preview = {
            let mut slot = app.solved_search.lock().await;
            slot.take()
        };
        if let Some(preview) =
            preview.filter(|preview| crate::time::now_secs().saturating_sub(preview.at) < 120)
        {
            let results =
                crate::ao3::search::parse_search_results(&preview.html).map_err(AppError::from)?;
            let page = crate::server::page_context(
                app,
                "Search Results — AO3Instance",
                NavSection::Search,
            )
            .await;
            return Ok(Html(render::live_search_results_page(
                &page, &live, &results, None,
            )));
        }
    }
    if live.has_criteria() && !edit {
        let path = live.path(live.page);
        let refresh = query.refresh.as_deref() == Some("1");
        let cache_account = crate::server::cache::cache_account(app).await;
        match crate::server::cache::fetch_cached_page(app, &cache_account, &path, true, refresh)
            .await
        {
            Ok(fetched) => {
                let results = crate::ao3::search::parse_search_results(&fetched.body)
                    .map_err(AppError::from)?;
                let page = crate::server::page_context(
                    app,
                    "Search Results — AO3Instance",
                    NavSection::Search,
                )
                .await;
                return Ok(Html(render::live_search_results_page(
                    &page,
                    &live,
                    &results,
                    Some((fetched.from_cache, fetched.fetched_at)),
                )));
            }
            Err(crate::server::cache::PageFetchError::Error(error)) => {
                return Err(AppError::from(error));
            }
            Err(crate::server::cache::PageFetchError::Status { status, location }) => {
                if location
                    .as_deref()
                    .is_some_and(|loc| loc.contains("/users/login"))
                {
                    return Err(AppError::from(crate::error::Error::LoggedOut));
                }
                match status {
                    403 | 503 | 429 => {
                        let solver_available = crate::server::solver::solver_available(app).await;
                        let send_cookies_allowed = {
                            let session = app.session.lock().await;
                            session.solver_may_receive_cookies()
                        };
                        let target = live.path(live.page);
                        let mut retry = format!("/works/search?{}", live.form_query());
                        if live.page > 1 {
                            retry.push_str(&format!("&page={}", live.page));
                        }
                        return Err(AppError::BlockedSearch {
                            status,
                            target,
                            retry,
                            solver_available,
                            send_cookies_allowed,
                        });
                    }
                    302 | 303 => {
                        return Err(AppError::Upstream(
                            "AO3 redirected the search; sign in on the Session page.".into(),
                        ));
                    }
                    other => {
                        return Err(AppError::Upstream(format!(
                            "Unexpected HTTP {other} from AO3 search."
                        )));
                    }
                }
            }
        }
    }

    let page =
        crate::server::page_context(app, "Work Search — AO3Instance", NavSection::Search).await;
    Ok(Html(render::live_search_form_page(&page, &live)))
}

fn live_query_from(query: &SearchQuery, raw: &str) -> crate::ao3::search::LiveSearchQuery {
    let clean = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let number = |value: &Option<String>| value.as_deref().and_then(|value| value.parse().ok());
    let split_tags = |value: &Option<String>| {
        value
            .as_deref()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let words = match (clean(&query.words_min), clean(&query.words_max)) {
        (Some(min), Some(max)) => Some(format!("{min}-{max}")),
        (Some(min), None) => Some(format!(">{min}")),
        (None, Some(max)) => Some(format!("<{max}")),
        (None, None) => None,
    };
    crate::ao3::search::LiveSearchQuery {
        query: raw.trim().to_string(),
        title: clean(&query.title),
        creator: clean(&query.creator),
        date: clean(&query.date),
        complete: clean(&query.complete),
        crossover: clean(&query.crossover),
        single_chapter: query
            .single_chapter
            .as_deref()
            .is_some_and(|value| matches!(value, "1" | "true" | "on")),
        word_count: words,
        language: clean(&query.language),
        fandom: clean(&query.fandom),
        characters: clean(&query.characters),
        relationships: clean(&query.relationships),
        additional_tags: clean(&query.additional_tags),
        include_tags: split_tags(&query.include_tags),
        exclude_tags: split_tags(&query.exclude_tags),
        rating: clean(&query.rating),
        warning: clean(&query.warning),
        category: clean(&query.category),
        hits_min: number(&query.hits_min),
        kudos_min: number(&query.kudos_min),
        comments_min: number(&query.comments_min),
        bookmarks_min: number(&query.bookmarks_min),
        sort: clean(&query.sort),
        direction: clean(&query.direction),
        page: query.page.unwrap_or(1),
    }
}
