//! Account list routes: dashboard, subscriptions, bookmarks, and reading history.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::render::{self, NavSection};
use crate::server::AppState;
use crate::server::error::Handled;

use super::RefreshQuery;

pub(in crate::server) async fn dashboard(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RefreshQuery>,
) -> Handled<Html<String>> {
    let refresh = query.refresh.as_deref() == Some("1");
    let (account, _) = crate::server::session_identity(&state).await;
    let cache_acct = crate::server::cache::cache_account(&state).await;
    let stats = state.store.cache_stats().await?;
    let subscriptions = state.store.list_subscriptions(&account).await?;
    let bookmarks = state.store.list_bookmarks(&account).await?;
    let reading = state.store.list_reading_history(&account).await?;
    let cached_works = state.store.cached_work_ids().await?;

    let (live_subs, live_bm, live_read) = if !cache_acct.is_empty() {
        let user_seg = user_path_segment(&cache_acct);
        let subs_path = format!("/users/{user_seg}/subscriptions");
        let bm_path = format!("/users/{user_seg}/bookmarks");
        let read_path = format!("/users/{user_seg}/readings");
        let live_subs = crate::server::cache::fetch_cached_optional(
            &state,
            &cache_acct,
            &subs_path,
            true,
            refresh,
        )
        .await
        .and_then(|fetched| crate::ao3::account_lists::parse_subscriptions(&fetched.body));
        let live_bm = crate::server::cache::fetch_cached_optional(
            &state,
            &cache_acct,
            &bm_path,
            true,
            refresh,
        )
        .await
        .and_then(|fetched| crate::ao3::account_lists::parse_bookmarks(&fetched.body));
        let live_read = crate::server::cache::fetch_cached_optional(
            &state,
            &cache_acct,
            &read_path,
            true,
            refresh,
        )
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
    let page =
        crate::server::page_context(&state, "Dashboard — AO3Instance", NavSection::Dashboard).await;
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
pub(in crate::server) struct SubscriptionsQuery {
    #[serde(rename = "type")]
    kind: Option<String>,
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(in crate::server) async fn subscriptions(
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
    let cache_acct = crate::server::cache::cache_account(&state).await;
    if !local && !cache_acct.is_empty() {
        let user_seg = user_path_segment(&cache_acct);
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
            crate::server::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh)
                .await
            && let Some(parsed) = crate::ao3::account_lists::parse_subscriptions(&fetched.body)
        {
            let cached_works = state.store.cached_work_ids().await?;
            let page = crate::server::page_context(
                &state,
                "Subscriptions — AO3Instance",
                NavSection::Dashboard,
            )
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
    let (account, _) = crate::server::session_identity(&state).await;
    let entries = state.store.list_subscriptions(&account).await?;
    let page =
        crate::server::page_context(&state, "Subscriptions — AO3Instance", NavSection::Dashboard)
            .await;
    Ok(Html(render::subscriptions_page(&page, &entries)))
}

/// `GET /my/works/<work_id>/chapters` — AO3's chapter index for one owned work.
pub(in crate::server) async fn my_work_chapters(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(work_id): axum::extract::Path<String>,
    Query(query): Query<MyWorksQuery>,
) -> Handled<Html<String>> {
    let refresh = query.refresh.as_deref() == Some("1");
    let notice = query
        .notice
        .as_deref()
        .and_then(crate::server::writes::notice_text);
    let cache_acct = crate::server::cache::cache_account(&state).await;
    let page =
        crate::server::page_context(&state, "Chapters — AO3Instance", NavSection::Dashboard).await;
    let cached_title = state
        .store
        .get_work(&work_id)
        .await?
        .map(|work| work.title)
        .unwrap_or_else(|| format!("work {work_id}"));
    if cache_acct.is_empty() {
        return Ok(Html(render::chapter_manager_page(
            &page,
            &work_id,
            &crate::ao3::chapters::ChapterIndex {
                work_title: cached_title,
                chapters: Vec::new(),
            },
            None,
            None,
            Some("Sign in to AO3 on the Session page to manage chapters."),
        )));
    }
    let path = format!("/works/{work_id}/navigate");
    let fetched =
        crate::server::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh)
            .await;
    let parsed = fetched.as_ref().and_then(|entry| {
        crate::ao3::chapters::parse_chapter_index(&entry.body)
            .ok()
            .map(|mut index| {
                if index.work_title.is_empty() {
                    index.work_title = cached_title.clone();
                }
                index
            })
    });
    let cache_meta = fetched
        .as_ref()
        .map(|entry| (entry.from_cache, entry.fetched_at));
    let error = match (&fetched, &parsed) {
        (None, _) => Some(
            "Could not reach archiveofourown.org for the chapter list; the cached work is unaffected.",
        ),
        (Some(_), None) => {
            Some("AO3 returned an unexpected chapter page (session or layout problem).")
        }
        _ => None,
    };
    let index = parsed.unwrap_or(crate::ao3::chapters::ChapterIndex {
        work_title: cached_title,
        chapters: Vec::new(),
    });
    Ok(Html(render::chapter_manager_page(
        &page, &work_id, &index, cache_meta, notice, error,
    )))
}

#[derive(Deserialize)]
pub(in crate::server) struct MyWorksQuery {
    page: Option<i64>,
    refresh: Option<String>,
    notice: Option<String>,
}

pub(in crate::server) async fn my_works(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MyWorksQuery>,
) -> Handled<Html<String>> {
    my_works_impl(state, query, false).await
}

pub(in crate::server) async fn my_drafts(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MyWorksQuery>,
) -> Handled<Html<String>> {
    my_works_impl(state, query, true).await
}

/// The signed-in author's works manager: AO3's own works / drafts index, cached
/// for 30 minutes like the other account lists, with the local cache used only
/// for links (the lists themselves are AO3 facts).
async fn my_works_impl(
    state: Arc<AppState>,
    query: MyWorksQuery,
    drafts: bool,
) -> Handled<Html<String>> {
    let refresh = query.refresh.as_deref() == Some("1");
    let page_num = query.page.unwrap_or(1).max(1);
    let notice = query
        .notice
        .as_deref()
        .and_then(crate::server::writes::notice_text);
    let cached_works = state.store.cached_work_ids().await?;
    let cache_acct = crate::server::cache::cache_account(&state).await;
    let title = if drafts {
        "My Drafts — AO3Instance"
    } else {
        "My Works — AO3Instance"
    };
    let page = crate::server::page_context(&state, title, NavSection::Dashboard).await;

    if cache_acct.is_empty() {
        return Ok(Html(render::my_works_page(
            &page,
            None,
            drafts,
            page_num,
            &cached_works,
            None,
            None,
            Some("Sign in to AO3 on the Session page to manage your works."),
        )));
    }

    let user_seg = user_path_segment(&cache_acct);
    let mut path = if drafts {
        format!("/users/{user_seg}/works/drafts")
    } else {
        format!("/users/{user_seg}/works")
    };
    if page_num > 1 {
        path.push_str(&format!("?page={page_num}"));
    }
    let fetched =
        crate::server::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh)
            .await;
    let parsed = fetched
        .as_ref()
        .and_then(|entry| crate::ao3::account_lists::parse_my_works(&entry.body));
    let cache_meta = fetched
        .as_ref()
        .map(|entry| (entry.from_cache, entry.fetched_at));
    let error = match (&fetched, &parsed) {
        (None, _) => Some(
            "Could not reach archiveofourown.org for this list. The cached library is unaffected.",
        ),
        (Some(_), None) => {
            Some("AO3 returned an unexpected works page (session expired, or the layout changed).")
        }
        _ => None,
    };
    Ok(Html(render::my_works_page(
        &page,
        parsed.as_ref(),
        drafts,
        page_num,
        &cached_works,
        cache_meta,
        notice,
        error,
    )))
}

#[derive(Deserialize)]
pub(in crate::server) struct BookmarksQuery {
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
    notice: Option<String>,
}

pub(in crate::server) async fn bookmarks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BookmarksQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    let page_num = query.page.unwrap_or(1).max(1);
    let notice = query
        .notice
        .as_deref()
        .and_then(crate::server::writes::notice_text);
    let cache_acct = crate::server::cache::cache_account(&state).await;
    if !local && !cache_acct.is_empty() {
        let user_seg = user_path_segment(&cache_acct);
        let path = if page_num > 1 {
            format!("/users/{user_seg}/bookmarks?page={page_num}")
        } else {
            format!("/users/{user_seg}/bookmarks")
        };
        if let Some(fetched) =
            crate::server::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh)
                .await
            && let Some(parsed) = crate::ao3::account_lists::parse_bookmarks(&fetched.body)
        {
            let page = crate::server::page_context(
                &state,
                "Bookmarks — AO3Instance",
                NavSection::Dashboard,
            )
            .await;
            return Ok(Html(render::live_bookmarks_page(
                &page,
                &parsed,
                page_num,
                Some((fetched.from_cache, fetched.fetched_at)),
                notice,
            )));
        }
    }
    let (account, _) = crate::server::session_identity(&state).await;
    let entries = state.store.list_bookmarks(&account).await?;
    let page =
        crate::server::page_context(&state, "Bookmarks — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::bookmarks_page(&page, &entries)))
}

#[derive(Deserialize)]
pub(in crate::server) struct ReadingQuery {
    show: Option<String>,
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(in crate::server) async fn reading(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReadingQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    let to_read = query.show.as_deref() == Some("to-read");
    let page_num = query.page.unwrap_or(1).max(1);
    let (account, _) = crate::server::session_identity(&state).await;
    let local_entries = state.store.list_reading_history(&account).await?;
    let cache_acct = crate::server::cache::cache_account(&state).await;
    if !local && !cache_acct.is_empty() {
        let user_seg = user_path_segment(&cache_acct);
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
            crate::server::cache::fetch_cached_optional(&state, &cache_acct, &path, true, refresh)
                .await
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
            let page = crate::server::page_context(&state, title, NavSection::Dashboard).await;
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
    let page =
        crate::server::page_context(&state, "Reading — AO3Instance", NavSection::Dashboard).await;
    Ok(Html(render::reading_page(&page, &local_entries)))
}

fn user_path_segment(account: &str) -> String {
    crate::ao3::parse::percent_encode(account, crate::ao3::parse::SpaceMode::Percent20)
}
