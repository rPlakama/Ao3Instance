//! Offline reads, plus the one user-initiated upstream fetch behind `/open`.

use std::sync::Arc;

use axum::{
    extract::{Form, Path, Query, State},
    http::{HeaderName, HeaderValue, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use super::session_identity;
use super::writes::notice_text;
use crate::ao3::actions;
use crate::ao3::work::parse_work_page;
use crate::error::Error;
use crate::render;
use crate::render::NavSection;
use crate::store::sha256_hex;

/// CSP for the chapter iframe: no scripts, no forms, inline styles only, images
/// from https only. Author content never runs in the app's origin.
const CHAPTER_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; \
     font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

#[derive(Deserialize)]
pub(super) struct HomeQuery {
    notice: Option<String>,
}

pub(super) async fn home(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HomeQuery>,
) -> Handled<Html<String>> {
    let works = state.store.list_works(50).await?;
    let detailed = state.store.list_works_detailed(50).await?;
    let fandoms = state.store.list_fandoms().await?;
    let flash = query.notice.as_deref().and_then(notice_text);
    let page = super::page_context(&state, "AO3Instance", NavSection::Home).await;
    Ok(Html(render::home_page(
        &page, &works, &detailed, &fandoms, flash,
    )))
}

#[derive(Deserialize)]
pub(super) struct OpenForm {
    id: String,
    /// `refresh=1` re-fetches even when the work is cached.
    refresh: Option<String>,
}

pub(super) async fn open_work(
    State(state): State<Arc<AppState>>,
    Form(form): Form<OpenForm>,
) -> Handled<Redirect> {
    let Some(id) = normalize_work_id(&form.id) else {
        return Err(AppError::BadRequest(
            "Expected an AO3 work id (e.g. 12345678) or a /works/<id> URL.".into(),
        ));
    };
    let existing = state.store.get_work(&id).await?;
    // Cache first: re-opening a cached work is an offline read, not a fetch.
    if form.refresh.is_none() && existing.is_some() {
        return Ok(Redirect::to(&format!("/read/{id}")));
    }
    let html = fetch_work_html(&state, &id).await?;
    cache_work_html(&state, &id, html, existing).await
}

/// Parses and stores one freshly fetched page, then redirects to the reader.
/// Shared by the direct `/open` path and the solver fallback so both cache the
/// same way.
pub(super) async fn cache_work_html(
    state: &AppState,
    id: &str,
    html: String,
    existing: Option<crate::store::StoredWork>,
) -> Handled<Redirect> {
    let mut page = match parse_work_page(&html) {
        Ok(page) => page,
        Err(Error::UpstreamChanged(_)) if looks_like_adult_gate(&html) => {
            return Err(AppError::Upstream(
                "This work is behind AO3's adult-content gate. Enable adult content on the \
                 Session page (or sign in), then reopen the work."
                    .into(),
            ));
        }
        Err(Error::UpstreamChanged(_)) if looks_like_login_gate(&html) => {
            return Err(AppError::Upstream(
                "AO3 says this work needs a signed-in account. Import your cookies on the \
                 Session page, then reopen the work."
                    .into(),
            ));
        }
        Err(error) => return Err(AppError::from(error)),
    };
    // The id in our URL is authoritative: the page's links may be relative or
    // (for synthetic fixtures) point elsewhere.
    page.ao3_id = id.to_string();
    let content_hash = sha256_hex(html.as_bytes());
    match existing {
        Some(work) if work.content_hash == content_hash => {
            // Same bytes upstream: record the check, do not rewrite the cache.
            state.store.touch_checked(id).await?;
            Ok(Redirect::to(&format!("/read/{id}?checked=1")))
        }
        Some(_) => {
            state.store.upsert_work(&page, &content_hash).await?;
            Ok(Redirect::to(&format!("/read/{id}?updated=1")))
        }
        None => {
            state.store.upsert_work(&page, &content_hash).await?;
            Ok(Redirect::to(&format!("/read/{id}")))
        }
    }
}

pub(super) async fn fetch_work_html(state: &AppState, id: &str) -> Handled<String> {
    // One request, user-initiated; `view_full_work` asks AO3 for all chapters in
    // a single page (ADR-0002: no crawling, no paging picks).
    let path = format!("/works/{id}?view_full_work=true");
    let (solver_reachable, send_cookies_allowed) = {
        let reachable = state.solver_status.read().await.reachable;
        let session = state.session.lock().await;
        (reachable, session.solver_may_receive_cookies())
    };
    let mut session = state.session.lock().await;
    let response = session
        .client_mut()
        .get(&path)
        .await
        .map_err(AppError::from)?;
    match response.status {
        200 => Ok(response.body),
        404 => Err(AppError::NotFound),
        429 => Err(AppError::Upstream(
            "AO3 is rate limiting this instance; wait a minute and retry.".into(),
        )),
        403 | 503 => Err(AppError::Blocked {
            status: response.status,
            work_id: id.to_string(),
            solver_reachable,
            send_cookies_allowed,
        }),
        302 | 303 => Err(AppError::Upstream(
            "AO3 redirected this work (restricted or login-gated). Sign in on the Session page."
                .into(),
        )),
        status => Err(AppError::Upstream(format!(
            "Unexpected HTTP {status} from AO3."
        ))),
    }
}

/// `GET /works/<id>` is the AO3-shaped alias: cached works read like `/read/<id>`,
/// uncached ones get an offline miss page with an explicit open button.
pub(super) async fn work_alias(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
) -> Handled<Response> {
    if state.store.get_work(&id).await?.is_none() {
        let page =
            super::page_context(&state, "Not cached — AO3Instance", NavSection::Browse).await;
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            Html(render::not_cached_page(&page, &id)),
        )
            .into_response());
    }
    let html = read_work(State(state), Path(id), Query(query)).await?;
    Ok(html.into_response())
}

/// `/works/<id>/chapters/<idx>` → the reader's chapter query form.
pub(super) async fn chapter_alias(Path((id, idx)): Path<(String, i64)>) -> Redirect {
    Redirect::to(&format!("/read/{id}?chapter={idx}"))
}

#[derive(Deserialize)]
pub(super) struct ReadQuery {
    chapter: Option<i64>,
    /// AO3-compatible "Entire Work" mode; renders every cached chapter.
    view_full_work: Option<String>,
    /// Set by a refresh that found no upstream change.
    checked: Option<String>,
    /// Set by a refresh that stored a new version.
    updated: Option<String>,
    /// Fixed-key notice from a write action (never raw user input).
    notice: Option<String>,
}

pub(super) async fn read_work(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
) -> Handled<Html<String>> {
    let work = state.store.get_work(&id).await?.ok_or(AppError::NotFound)?;
    let chapters = state.store.get_chapters(&id).await?;
    let progress = state.store.get_progress(&id).await?;
    let current = query
        .chapter
        .or(progress)
        .unwrap_or_else(|| chapters.first().map(|chapter| chapter.idx).unwrap_or(1));
    let full_work = query
        .view_full_work
        .as_deref()
        .is_some_and(|value| matches!(value, "true" | "1" | "yes" | "on"));
    let notice = if query.checked.is_some() {
        Some("Checked AO3 — no changes since the cached copy.")
    } else if query.updated.is_some() {
        Some("Updated from AO3.")
    } else {
        query.notice.as_deref().and_then(notice_text)
    };
    let (account, usable) = session_identity(&state).await;
    let subscribed = if usable && !account.is_empty() {
        state
            .store
            .get_subscription(&account, actions::WORK, &id)
            .await?
            .is_some_and(|row| row.state == "active")
    } else {
        false
    };
    let can_write = usable && !account.is_empty() && state.write_enabled;
    let kudos_sent = can_write && state.store.has_kudos(&account, &id).await?;
    let bookmarked = can_write && state.store.has_bookmark(&account, &id).await?;
    let reading_state = if can_write {
        state.store.get_reading_state(&account, &id).await?
    } else {
        None
    };
    let comment_as = if can_write {
        let session = state.session.lock().await;
        session.pseud().map(|pseud| pseud.name.clone())
    } else {
        None
    };
    let page = super::page_context(
        &state,
        &format!("{} — AO3Instance", work.title),
        NavSection::Home,
    )
    .await;
    Ok(Html(render::read_page(
        &page,
        &work,
        &chapters,
        &render::ReadContext {
            current,
            progress,
            notice,
            subscribed,
            can_write,
            kudos_sent,
            bookmarked,
            reading_state: reading_state.as_deref(),
            full_work,
            comment_as: comment_as.as_deref(),
        },
    )))
}

#[derive(Deserialize)]
pub(super) struct ChapterQuery {
    skin: Option<String>,
}

pub(super) async fn read_chapter(
    State(state): State<Arc<AppState>>,
    Path((id, idx)): Path<(String, i64)>,
    Query(query): Query<ChapterQuery>,
) -> Handled<Response> {
    let work = state.store.get_work(&id).await?.ok_or(AppError::NotFound)?;
    let chapters = state.store.get_chapters(&id).await?;
    let chapter = chapters
        .iter()
        .find(|chapter| chapter.idx == idx)
        .ok_or(AppError::NotFound)?;
    let skin = match query.skin.as_deref() {
        Some("reversi") => "reversi",
        _ => "default",
    };
    let mut response = Html(render::chapter_document(&work.title, chapter, skin)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CHAPTER_CSP),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[derive(Deserialize)]
pub(super) struct SearchQuery {
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
}

pub(super) async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Handled<Html<String>> {
    perform_search(state.as_ref(), query).await
}

/// The AO3-shaped form posts here. AO3's own web search endpoint is GET-only,
/// so the server issues exactly one literal GET upstream for this POST.
pub(super) async fn search_post(
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
                super::page_context(app, "Local Search — AO3Instance", NavSection::Search).await;
            return Ok(Html(render::search_results_page(
                &page, &raw, &result, &filter,
            )));
        }
        let (account, _) = session_identity(app).await;
        let facets = app
            .store
            .browse_works(&account, &crate::store::WorkFilter::default())
            .await?;
        let page = super::page_context(app, "Local Search — AO3Instance", NavSection::Search).await;
        return Ok(Html(render::search_form_page(
            &page, &raw, &facets, &filter,
        )));
    }

    let live = live_query_from(&query, &raw);
    let edit = query.edit_search.as_deref() == Some("true");
    if live.has_criteria() && !edit {
        let path = live.path(live.page);
        let response = {
            let mut session = app.session.lock().await;
            session
                .client_mut()
                .get(&path)
                .await
                .map_err(AppError::from)?
        };
        if response.location_contains("/users/login") {
            return Err(AppError::from(crate::error::Error::LoggedOut));
        }
        match response.status {
            200 => {}
            403 | 503 => {
                return Err(AppError::Upstream(format!(
                    "AO3 refused the search (HTTP {}); it may be a bot challenge.",
                    response.status
                )));
            }
            429 => {
                return Err(AppError::Upstream(
                    "AO3 is rate limiting this instance; wait a minute and retry.".into(),
                ));
            }
            302 | 303 => {
                return Err(AppError::Upstream(
                    "AO3 redirected the search; sign in on the Session page.".into(),
                ));
            }
            status => {
                return Err(AppError::Upstream(format!(
                    "Unexpected HTTP {status} from AO3 search."
                )));
            }
        }
        let results =
            crate::ao3::search::parse_search_results(&response.body).map_err(AppError::from)?;
        let page =
            super::page_context(app, "Search Results — AO3Instance", NavSection::Search).await;
        return Ok(Html(render::live_search_results_page(
            &page, &live, &results,
        )));
    }

    let page = super::page_context(app, "Work Search — AO3Instance", NavSection::Search).await;
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

#[derive(Deserialize)]
pub(super) struct ProgressForm {
    work_id: String,
    chapter: i64,
}

pub(super) async fn save_progress(
    State(state): State<Arc<AppState>>,
    Form(form): Form<ProgressForm>,
) -> Handled<Redirect> {
    if state.store.get_work(&form.work_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    state
        .store
        .save_progress(&form.work_id, form.chapter)
        .await?;
    Ok(Redirect::to(&format!(
        "/read/{}?chapter={}",
        form.work_id, form.chapter
    )))
}

/// AO3's adult interstitial has no work title, so the parser reports drift; the
/// gate markers let us say "enable adult content" instead of "AO3 changed".
fn looks_like_adult_gate(html: &str) -> bool {
    html.contains("This work could have adult content")
        || html.contains("id=\"adult\"")
        || html.contains("Proceed")
}

/// Restricted works redirect to the login page; the marker check covers the
/// rendered login form some paths serve with 200.
fn looks_like_login_gate(html: &str) -> bool {
    html.contains("This work is only available to registered users")
        || (html.contains("id=\"loginform\"") && !html.contains("class=\"title heading\""))
}

/// Accepts a bare id or any URL containing `/works/<id>`.
pub(super) fn normalize_work_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }
    let marker = "/works/";
    let start = trimmed.find(marker)? + marker.len();
    let digits: String = trimmed[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_ids_are_normalised_from_ids_and_urls() {
        assert_eq!(normalize_work_id("12345").as_deref(), Some("12345"));
        assert_eq!(
            normalize_work_id("https://archiveofourown.org/works/12345").as_deref(),
            Some("12345")
        );
        assert_eq!(
            normalize_work_id(" https://archiveofourown.org/works/12345?view_full_work=true ")
                .as_deref(),
            Some("12345")
        );
        assert_eq!(
            normalize_work_id("https://archiveofourown.org/works/12345/chapters/1").as_deref(),
            Some("12345")
        );
        assert_eq!(normalize_work_id(""), None);
        assert_eq!(normalize_work_id("   "), None);
        assert_eq!(normalize_work_id("not a work"), None);
        assert_eq!(
            normalize_work_id("https://archiveofourown.org/tags/x/works"),
            None
        );
    }

    #[test]
    fn gate_markers_are_recognised() {
        let adult = "<div id=\"adult\">This work could have adult content. Proceed</div>";
        assert!(looks_like_adult_gate(adult));
        assert!(!looks_like_login_gate(adult));
        let login = "<div id=\"loginform\">This work is only available to registered users</div>";
        assert!(looks_like_login_gate(login));
    }
}
