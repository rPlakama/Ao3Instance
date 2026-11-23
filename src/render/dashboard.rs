//! Dashboard modelled on `otwarchive/app/views/users/show.html.erb`,
//! `_header.html.erb`, and `_contents.html.erb`: `.user.home` header, live
//! AO3 account lists (cached 30 min) + local cache summary, and AO3
//! `.listbox.group` panels.

use std::collections::HashSet;

use super::chrome::{PageContext, layout_with_class};
use super::escape;
use crate::ao3::account_lists::{LiveBookmarksPage, LiveReadingsPage, LiveSubscriptionsPage};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BookmarkEntry, CacheStats, ReadingEntry, SubscriptionEntry};

/// Live (or 30-minute cached) AO3 lists merged onto the dashboard when signed in.
pub struct DashboardLive<'a> {
    pub subscriptions: Option<&'a LiveSubscriptionsPage>,
    pub bookmarks: Option<&'a LiveBookmarksPage>,
    pub readings: Option<&'a LiveReadingsPage>,
    pub cached_works: &'a HashSet<String>,
}

pub fn dashboard_page(
    ctx: &PageContext,
    stats: &CacheStats,
    subscriptions: &[SubscriptionEntry],
    bookmarks: &[BookmarkEntry],
    reading: &[ReadingEntry],
    live: &DashboardLive<'_>,
) -> String {
    let display_name = if ctx.account.is_empty() {
        "Dashboard".to_string()
    } else {
        escape(&ctx.account)
    };
    let mut body = String::from("<div class=\"user home\">");
    body.push_str(&format!(
        "<div class=\"primary header module\">\
         <h2 class=\"heading\">{display_name}</h2>\
         <ul class=\"navigation actions\">\
         <li><a href=\"/works\">Cached library</a></li>\
         <li><a href=\"/fandoms\">Fandoms</a></li>\
         <li><a href=\"/subscriptions\">Subscriptions</a></li>\
         <li><a href=\"/bookmarks\">Bookmarks</a></li>\
         <li><a href=\"/reading\">History &amp; Marked for Later</a></li>\
         <li><a href=\"/session\">Session &amp; preferences</a></li>\
         <li><a href=\"/audit\">Write audit</a></li>\
         <li><a href=\"/diagnostics\">HTTP diagnostics</a></li>\
         <li><a href=\"/dashboard?refresh=1\">Refresh from AO3</a></li></ul>\
         <p class=\"type\">Signed in as <strong>{account}</strong> · session \
         <span class=\"state {state_class}\">{state}</span> · writes {writes}</p></div>",
        account = if ctx.account.is_empty() {
            "nobody".to_string()
        } else {
            escape(&ctx.account)
        },
        state = escape(&ctx.session_state),
        state_class = if ctx.session_state == "active" {
            "active"
        } else {
            "imported"
        },
        writes = if ctx.write_enabled {
            "enabled"
        } else {
            "disabled"
        },
    ));

    let sub_count = live.subscriptions.map_or(stats.subscriptions, |page| {
        i64::try_from(page.items.len()).unwrap_or(i64::MAX)
    });
    let bm_count = live.bookmarks.map_or(stats.bookmarks, |page| {
        i64::try_from(page.total.max(page.items.len())).unwrap_or(i64::MAX)
    });

    body.push_str(&format!(
        "<div class=\"wrapper\"><dl class=\"work meta group\">\
         <dt>Cached works:</dt><dd><a href=\"/works\">{works}</a></dd>\
         <dt>Cached chapters:</dt><dd>{chapters}</dd>\
         <dt>Subscriptions:</dt><dd><a href=\"/subscriptions\">{sub_count}</a></dd>\
         <dt>Bookmarks:</dt><dd><a href=\"/bookmarks\">{bm_count}</a></dd>\
         <dt>Kudos given:</dt><dd>{kudos}</dd>\
         <dt>HTTP failures logged:</dt><dd><a href=\"/diagnostics\">{errors}</a></dd></dl></div>",
        works = stats.works,
        chapters = stats.chapters,
        kudos = stats.kudos,
        errors = stats.recent_http_errors,
    ));

    let sub_content = if let Some(live_subs) = live.subscriptions {
        recent_items(
            live_subs.items.iter().map(|item| {
                let href = match item.kind.as_str() {
                    "Work" if !item.target_id.is_empty() => {
                        if live.cached_works.contains(&item.target_id) {
                            format!("/read/{}", escape(&item.target_id))
                        } else {
                            format!("/open/{}", escape(&item.target_id))
                        }
                    }
                    "User" if !item.target_id.is_empty() => format!(
                        "/works/search?creator={}",
                        percent_encode(&item.target_id, SpaceMode::Percent20)
                    ),
                    _ => "/subscriptions".to_string(),
                };
                (escape(&item.title), href, escape(&item.kind))
            }),
            "No subscriptions on AO3.",
        )
    } else {
        recent_items(
            subscriptions.iter().map(|entry| {
                let (href, label) = match entry.target_kind.as_str() {
                    "Work" => (
                        format!("/read/{}", escape(&entry.target_id)),
                        entry
                            .title
                            .as_deref()
                            .unwrap_or(&entry.target_id)
                            .to_string(),
                    ),
                    "Tag" => (
                        crate::render::tag_href(
                            entry.title.as_deref().unwrap_or(&entry.target_id),
                        ),
                        entry
                            .title
                            .as_deref()
                            .unwrap_or(&entry.target_id)
                            .to_string(),
                    ),
                    _ => (
                        "/subscriptions".to_string(),
                        format!("{} {}", entry.target_kind, entry.target_id),
                    ),
                };
                (
                    escape(&label),
                    href,
                    format!("subscription {}", escape(&entry.state)),
                )
            }),
            "No subscriptions yet.",
        )
    };
    body.push_str(&listbox(
        "user-subscriptions",
        "Recent subscriptions",
        sub_content,
        Some(("/subscriptions", "All Subscriptions")),
    ));

    let bm_content = if let Some(live_bm) = live.bookmarks {
        recent_items(
            live_bm.items.iter().filter_map(|item| {
                if let Some(work) = &item.work {
                    let href = if live.cached_works.contains(&work.ao3_id) {
                        format!("/read/{}", escape(&work.ao3_id))
                    } else {
                        format!("/open/{}", escape(&work.ao3_id))
                    };
                    let meta = if item.bookmarked_at.is_empty() {
                        "bookmarked".to_string()
                    } else {
                        escape(&item.bookmarked_at)
                    };
                    Some((escape(&work.title), href, meta))
                } else {
                    item.fallback_title.as_deref().map(|title| {
                        (
                            escape(title),
                            "/bookmarks".to_string(),
                            escape(&item.bookmarked_at),
                        )
                    })
                }
            }),
            "No bookmarks on AO3 yet.",
        )
    } else {
        recent_items(
            bookmarks.iter().map(|entry| {
                (
                    escape(entry.title.as_deref().unwrap_or(&entry.work_id)),
                    format!("/read/{}", escape(&entry.work_id)),
                    "bookmarked".to_string(),
                )
            }),
            "No bookmarks yet.",
        )
    };
    body.push_str(&listbox(
        "user-bookmarks",
        "Recent bookmarks",
        bm_content,
        Some(("/bookmarks", "All Bookmarks")),
    ));

    let reading_content = if let Some(live_read) = live.readings {
        recent_items(
            live_read.items.iter().filter_map(|item| {
                let work = item.work.as_ref()?;
                let href = if live.cached_works.contains(&work.ao3_id) {
                    format!("/read/{}", escape(&work.ao3_id))
                } else {
                    format!("/open/{}", escape(&work.ao3_id))
                };
                let meta = if item.viewed_info.is_empty() {
                    "AO3 history".to_string()
                } else {
                    escape(&item.viewed_info)
                };
                Some((escape(&work.title), href, meta))
            }),
            "No AO3 reading history yet.",
        )
    } else {
        recent_items(
            reading.iter().filter_map(|entry| {
                entry.progress_chapter.map(|chapter| {
                    let pct = entry.progress_pct.map_or(0, |pct| {
                        super::overall_percent(chapter, pct, entry.chapter_count)
                    });
                    (
                        escape(entry.title.as_deref().unwrap_or(&entry.work_id)),
                        format!("/read/{}?chapter={chapter}", escape(&entry.work_id)),
                        format!("chapter {chapter} · {pct}%"),
                    )
                })
            }),
            "No saved reading position yet.",
        )
    };
    body.push_str(&listbox(
        "user-reading",
        "Continue reading & history",
        reading_content,
        Some(("/reading", "All Reading History")),
    ));

    body.push_str("</div>");
    layout_with_class(ctx, "dashboard users-show", &body)
}

fn listbox(id: &str, heading: &str, content: String, more: Option<(&str, &str)>) -> String {
    let footer = more.map_or_else(String::new, |(href, label)| {
        format!("<ul class=\"actions\" role=\"navigation\"><li><a href=\"{href}\">{label}</a></li></ul>")
    });
    format!(
        "<div class=\"listbox group\" id=\"{id}\">\
         <h3 class=\"heading\">{heading}</h3>\
         <div class=\"index group\">{content}</div>{footer}</div>"
    )
}

fn recent_items<I>(items: I, empty: &str) -> String
where
    I: Iterator<Item = (String, String, String)>,
{
    let items: Vec<(String, String, String)> = items.take(5).collect();
    if items.is_empty() {
        return format!("<p class=\"muted\">{empty}</p>");
    }
    let mut out = String::from("<ol class=\"works index group\">");
    for (title, href, meta) in items {
        out.push_str(&format!(
            "<li><a href=\"{href}\">{title}</a> <span class=\"meta\">· {meta}</span></li>"
        ));
    }
    out.push_str("</ol>");
    out
}
