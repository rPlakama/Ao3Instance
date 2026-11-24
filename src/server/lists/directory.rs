//! Fandoms directory (`/fandoms`, `/media/:slug/fandoms`) and Tag Search (`/tags/search`).

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::render::{self, NavSection};
use crate::server::AppState;
use crate::server::error::Handled;

use super::RefreshQuery;

#[derive(Deserialize)]
pub(in crate::server) struct FandomsQuery {
    local: Option<String>,
    refresh: Option<String>,
}

pub(in crate::server) async fn fandoms(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FandomsQuery>,
) -> Handled<Html<String>> {
    let local = query.local.as_deref() == Some("1");
    let refresh = query.refresh.as_deref() == Some("1");
    if !local
        && let Some(fetched) =
            crate::server::cache::fetch_cached_optional(&state, "", "/media", false, refresh).await
    {
        let media = crate::ao3::account_lists::parse_media_index(&fetched.body);
        if !media.is_empty() {
            let page =
                crate::server::page_context(&state, "Fandoms — AO3Instance", NavSection::Fandoms)
                    .await;
            return Ok(Html(render::live_media_page(
                &page,
                &media,
                Some((fetched.from_cache, fetched.fetched_at)),
            )));
        }
    }
    let fandoms = state.store.list_fandoms().await?;
    let page =
        crate::server::page_context(&state, "Fandoms — AO3Instance", NavSection::Fandoms).await;
    Ok(Html(render::fandoms_page(&page, &fandoms)))
}

pub(in crate::server) async fn medium_fandoms(
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
        crate::server::cache::fetch_cached_optional(&state, "", &path, false, refresh).await
    {
        let data = crate::ao3::account_lists::parse_medium_fandoms(&fetched.body);
        let title = if data.title.is_empty() {
            format!("{medium} — AO3Instance")
        } else {
            format!("{} — AO3Instance", data.title)
        };
        let page = crate::server::page_context(&state, &title, NavSection::Fandoms).await;
        return Ok(Html(render::live_medium_fandoms_page(
            &page,
            &medium,
            &data,
            Some((fetched.from_cache, fetched.fetched_at)),
        )));
    }
    let fandoms = state.store.list_fandoms().await?;
    let page = crate::server::page_context(
        &state,
        &format!("{medium} — AO3Instance"),
        NavSection::Fandoms,
    )
    .await;
    Ok(Html(render::fandoms_page(&page, &fandoms)))
}

#[derive(Deserialize)]
pub(in crate::server) struct TagSearchQuery {
    q: Option<String>,
    page: Option<i64>,
    local: Option<String>,
    refresh: Option<String>,
}

pub(in crate::server) async fn tag_search(
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
            crate::server::cache::fetch_cached_optional(&state, "", &path, false, refresh).await
        {
            let results = crate::ao3::account_lists::parse_tag_search(&fetched.body);
            let page = crate::server::page_context(
                &state,
                "Tag Search — AO3Instance",
                NavSection::Fandoms,
            )
            .await;
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
    let page =
        crate::server::page_context(&state, "Tag Search — AO3Instance", NavSection::Fandoms).await;
    Ok(Html(render::tag_search_page(&page, &raw, &hits)))
}
