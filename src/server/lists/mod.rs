//! Offline & 30-minute cached list pages: browse, tag pages, fandoms,
//! dashboard, account ledgers, about, and local history.

mod account;
mod browse;
mod directory;

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::render::{self, NavSection};
use crate::server::AppState;
use crate::server::error::Handled;

pub(super) use account::{bookmarks, dashboard, reading, subscriptions};
pub(super) use browse::{autocomplete, browse, tag_browse};
pub(super) use directory::{fandoms, medium_fandoms, tag_search};

#[derive(Deserialize)]
pub(super) struct RefreshQuery {
    pub(super) refresh: Option<String>,
}

pub(super) async fn about(State(state): State<Arc<AppState>>) -> Handled<Html<String>> {
    let page = super::page_context(&state, "About — AO3Instance", NavSection::About).await;
    Ok(Html(render::about_page(&page)))
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

pub(super) fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

pub(super) fn tag_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn query_pairs(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |text: &str| crate::ao3::parse::percent_decode(&text.replace('+', " "));
            Some((decode(key)?, decode(value)?))
        })
        .collect()
}

pub(super) fn first(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

pub(super) fn all(pairs: &[(String, String)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
        .collect()
}
