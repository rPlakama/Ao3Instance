//! Dashboard modelled on `otwarchive/app/views/users/show.html.erb`,
//! `_header.html.erb`, and `_contents.html.erb`: `.user.home` header, cache
//! metadata summary, and AO3 `.listbox.group` panels.

use super::chrome::{PageContext, layout_with_class};
use super::escape;
use crate::store::{BookmarkEntry, CacheStats, ReadingEntry, SubscriptionEntry};

pub fn dashboard_page(
    ctx: &PageContext,
    stats: &CacheStats,
    subscriptions: &[SubscriptionEntry],
    bookmarks: &[BookmarkEntry],
    reading: &[ReadingEntry],
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
         <li><a href=\"/reading\">Marked for later &amp; history</a></li>\
         <li><a href=\"/session\">Session &amp; preferences</a></li>\
         <li><a href=\"/audit\">Write audit</a></li>\
         <li><a href=\"/diagnostics\">HTTP diagnostics</a></li></ul>\
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

    body.push_str(&format!(
        "<div class=\"wrapper\"><dl class=\"work meta group\">\
         <dt>Cached works:</dt><dd><a href=\"/works\">{works}</a></dd>\
         <dt>Cached chapters:</dt><dd>{chapters}</dd>\
         <dt>Subscriptions:</dt><dd><a href=\"/subscriptions\">{subscriptions}</a></dd>\
         <dt>Bookmarks:</dt><dd><a href=\"/bookmarks\">{bookmarks}</a></dd>\
         <dt>Kudos given:</dt><dd>{kudos}</dd>\
         <dt>HTTP failures logged:</dt><dd><a href=\"/diagnostics\">{errors}</a></dd></dl></div>",
        works = stats.works,
        chapters = stats.chapters,
        subscriptions = stats.subscriptions,
        bookmarks = stats.bookmarks,
        kudos = stats.kudos,
        errors = stats.recent_http_errors,
    ));

    body.push_str(&listbox(
        "user-subscriptions",
        "Recent subscriptions",
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
            "No subscriptions recorded locally.",
        ),
        Some(("/subscriptions", "All Subscriptions")),
    ));

    body.push_str(&listbox(
        "user-bookmarks",
        "Recent bookmarks",
        recent_items(
            bookmarks.iter().map(|entry| {
                (
                    escape(entry.title.as_deref().unwrap_or(&entry.work_id)),
                    format!("/read/{}", escape(&entry.work_id)),
                    "bookmarked".to_string(),
                )
            }),
            "No bookmark yet.",
        ),
        Some(("/bookmarks", "All Bookmarks")),
    ));

    body.push_str(&listbox(
        "user-reading",
        "Continue reading",
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
        ),
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
