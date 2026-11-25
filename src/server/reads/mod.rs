//! Offline reads, home splash, work reader, plus the one user-initiated
//! upstream fetch behind `/open`.

mod comments;
mod progress;
mod search;

use std::sync::Arc;

use axum::{
    extract::{Form, Path, Query, State},
    http::{HeaderName, HeaderValue, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use super::AppState;
use super::error::{AppError, Handled};
use super::writes::notice_text;
use crate::ao3::actions;
use crate::ao3::html::extract_digits_after;
use crate::ao3::work::parse_work_page;
use crate::error::Error;
use crate::render;
use crate::render::NavSection;
use crate::store::sha256_hex;

pub(super) use comments::load_comments;
pub(super) use progress::{auto_progress, save_progress};
pub(super) use search::{search, search_post};

/// CSP for the chapter iframe: no scripts, no forms, inline styles only, images
/// from https only. Author content never runs in the app's origin.
const CHAPTER_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; \
     font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

#[derive(Deserialize)]
pub(super) struct HomeQuery {
    notice: Option<String>,
    refresh: Option<String>,
}

pub(super) async fn home(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HomeQuery>,
) -> Handled<Html<String>> {
    let works = state.store.list_works(50).await?;
    let detailed = state.store.list_works_detailed(50).await?;
    let fandoms = state.store.list_fandoms().await?;
    let flash = query.notice.as_deref().and_then(notice_text);
    let cache_account = super::cache::cache_account(&state).await;
    let refresh = query.refresh.as_deref() == Some("1");
    let modules = match live_home_html(&state, &cache_account, refresh).await {
        Some(html) => crate::ao3::home::parse_home(&html),
        None => crate::ao3::home::HomeModules::default(),
    };
    let page = super::page_context(&state, "AO3Instance", NavSection::Home).await;
    Ok(Html(render::home_page(
        &page, &works, &detailed, &fandoms, flash, &modules,
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
    if let Err(error) = state.store.record_access(&id, "cached").await {
        eprintln!("access history: {error}");
    }
    cache_work_html(&state, &id, html, existing).await
}

/// One-click result title: cache if needed, then open the reader.
pub(super) async fn open_and_read(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Handled<Redirect> {
    let Some(id) = normalize_work_id(&id) else {
        return Err(AppError::BadRequest("Expected an AO3 work id.".into()));
    };
    let existing = state.store.get_work(&id).await?;
    if existing.is_some() {
        return Ok(Redirect::to(&format!("/read/{id}")));
    }
    let html = fetch_work_html(&state, &id).await?;
    if let Err(error) = state.store.record_access(&id, "cached").await {
        eprintln!("access history: {error}");
    }
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
    fetch_page_html(state, &path, id).await
}

/// One user-initiated HTML fetch: the host-provided solver first (when present),
/// then the direct session client. The caller owns the path and the action that
/// triggered it; nothing here runs in the background or retries.
pub(super) async fn fetch_page_html(
    state: &AppState,
    path: &str,
    work_id: &str,
) -> Handled<String> {
    if super::solver::solver_available(state).await {
        match super::solver::fetch_via_solver(state, path, true).await {
            Ok(response) => match response.status {
                200 => return Ok(response.body),
                404 => return Err(AppError::NotFound),
                status => {
                    return Err(AppError::Upstream(format!(
                        "Unexpected HTTP {status} from AO3 through the local solver."
                    )));
                }
            },
            Err(Error::SolverUnavailable(_)) => {}
            Err(error) => return Err(AppError::from(error)),
        }
    }
    let (solver_reachable, send_cookies_allowed) = {
        let reachable = state.solver_status.read().await.reachable;
        let session = state.session.lock().await;
        (reachable, session.solver_may_receive_cookies())
    };
    let response = {
        let mut session = state.session.lock().await;
        let response = session
            .client_mut()
            .get(path)
            .await
            .map_err(AppError::from)?;
        session.sync_csrf_from_response(&response);
        super::sync_session_snapshot(state, &session).await;
        response
    };
    if response.jar_changed {
        super::session::persist_if_complete(state).await;
    }
    match response.status {
        200 => Ok(response.body),
        404 => Err(AppError::NotFound),
        429 => Err(AppError::Upstream(
            "AO3 is rate limiting this instance; wait a minute and retry.".into(),
        )),
        403 | 503 => Err(AppError::Blocked {
            status: response.status,
            work_id: work_id.to_string(),
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
    /// Render the cached AO3 comment thread (`?show_comments=1`).
    show_comments: Option<String>,
}

/// AO3's boolean query params accept `true` (and the usual spellings).
pub(super) fn is_truthy(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| matches!(value, "true" | "1" | "yes" | "on"))
}

/// The commentable AO3 renders for this reader position, plus the page that
/// carries its comment thread.
///
/// AO3 comments attach to the work itself in Entire Work view (and for
/// one-chapter works, whose `/works/<id>` URL is the work page), and to the
/// chapter in single-chapter view of a multi-chapter work. The cached chapter
/// AO3 id is what makes the chapter page reachable.
pub(super) fn comment_scope(
    work_id: &str,
    chapters: &[crate::store::StoredChapter],
    current: i64,
    full_work: bool,
) -> (String, String) {
    if !full_work
        && chapters.len() > 1
        && let Some(chapter) = chapters.iter().find(|chapter| chapter.idx == current)
        && let Some(ao3_chapter_id) = &chapter.ao3_chapter_id
    {
        return (
            crate::store::chapter_scope(ao3_chapter_id),
            format!("/works/{work_id}/chapters/{ao3_chapter_id}?show_comments=true"),
        );
    }
    (
        crate::store::WORK_SCOPE.to_string(),
        format!("/works/{work_id}?view_full_work=true&show_comments=true"),
    )
}

pub(super) async fn read_work(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ReadQuery>,
) -> Handled<Html<String>> {
    let work = state.store.get_work(&id).await?.ok_or(AppError::NotFound)?;
    if let Err(error) = state.store.record_access(&id, "read").await {
        eprintln!("access history: {error}");
    }
    let chapters = state.store.get_chapters(&id).await?;
    let progress_exact = state.store.get_progress_exact(&id).await?;
    let progress = progress_exact.map(|(chapter_idx, pct)| crate::store::Progress {
        chapter_idx,
        pct: pct.round().clamp(0.0, 100.0) as i64,
    });
    let current = query
        .chapter
        .or_else(|| progress.map(|saved| saved.chapter_idx))
        .unwrap_or_else(|| chapters.first().map(|chapter| chapter.idx).unwrap_or(1));
    let resume_pct = progress_exact
        .filter(|(chapter_idx, _)| *chapter_idx == current)
        .map(|(_, pct)| pct);
    let full_work = is_truthy(query.view_full_work.as_deref());
    let show_comments = is_truthy(query.show_comments.as_deref());
    // Comment notices belong next to the thread (the redirect lands on
    // `#comments`), not in the page-top flash.
    let comment_notice = query
        .notice
        .as_deref()
        .filter(|key| key.starts_with("comments_"))
        .and_then(notice_text);
    let notice = if query.checked.is_some() {
        Some("Checked AO3 — no changes since the cached copy.")
    } else if query.updated.is_some() {
        Some("Updated from AO3.")
    } else if comment_notice.is_some() {
        None
    } else {
        query.notice.as_deref().and_then(notice_text)
    };
    let (comment_scope_key, comment_page_path) = comment_scope(&id, &chapters, current, full_work);
    let comment_set = state.store.get_comment_set(&id, &comment_scope_key).await?;
    let comments = if show_comments {
        state.store.get_comments(&id, &comment_scope_key).await?
    } else {
        Vec::new()
    };
    let snap = state.session_snapshot.read().await.clone();
    let signed_in = !snap.account.is_empty() && (snap.usable || snap.jar_complete);
    let subscribed = if signed_in {
        state
            .store
            .get_subscription(&snap.account, actions::WORK, &id)
            .await?
            .is_some_and(|row| row.state == "active")
    } else {
        false
    };
    // Distinguish "signed out" from "signed in but writes off" and "the saved
    // jar cannot authenticate": each needs a different notice and recovery path.
    let session_ready = snap.usable || snap.jar_complete;
    let write_gate = if snap.account.is_empty() {
        crate::render::WriteGate::SignedOut
    } else if !session_ready {
        crate::render::WriteGate::SessionUnusable
    } else if !state.write_enabled {
        crate::render::WriteGate::WritesDisabled
    } else {
        crate::render::WriteGate::Ready
    };
    let can_write = write_gate.can_write();
    let kudos_sent = can_write && state.store.has_kudos(&snap.account, &id).await?;
    let bookmarked = can_write && state.store.has_bookmark(&snap.account, &id).await?;
    let reading_state = if can_write {
        state.store.get_reading_state(&snap.account, &id).await?
    } else {
        None
    };
    let comment_as = if can_write { snap.pseud_name } else { None };
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
            resume_pct,
            notice,
            subscribed,
            write_gate,
            kudos_sent,
            bookmarked,
            reading_state: reading_state.as_deref(),
            full_work,
            comment_as: comment_as.as_deref(),
            show_comments,
            comments: &comments,
            comment_set: comment_set.as_ref(),
            comment_notice,
            comment_page_path: Some(comment_page_path.as_str()),
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
    let skin = query.skin.as_deref().unwrap_or("default");
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

/// One AO3 home fetch, cached in SQLite for 30 minutes (with stale-on-error
/// fallback when AO3 is unreachable).
async fn live_home_html(state: &AppState, account: &str, refresh: bool) -> Option<String> {
    super::cache::fetch_cached_optional(state, account, "/", !account.is_empty(), refresh)
        .await
        .map(|fetched| fetched.body)
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
    extract_digits_after(trimmed, "/works/").map(str::to_string)
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
