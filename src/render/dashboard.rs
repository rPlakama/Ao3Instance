//! Dashboard: cache statistics, session status and shortcuts into the lists.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BookmarkEntry, CacheStats, ReadingEntry, SubscriptionEntry};

pub fn dashboard_page(
    ctx: &PageContext,
    stats: &CacheStats,
    subscriptions: &[SubscriptionEntry],
    bookmarks: &[BookmarkEntry],
    reading: &[ReadingEntry],
) -> String {
    let mut body = String::from("<h2>Dashboard</h2>");
    body.push_str(&format!(
        "<p class=\"meta\">Signed in as <strong>{account}</strong> · session \
         <span class=\"state\">{state}</span> · writes {writes}</p>",
        account = if ctx.account.is_empty() {
            "nobody".to_string()
        } else {
            escape(&ctx.account)
        },
        state = escape(&ctx.session_state),
        writes = if ctx.write_enabled {
            "enabled"
        } else {
            "disabled"
        },
    ));

    body.push_str(&format!(
        "<dl class=\"work meta group\">\
         <dt>Cached works:</dt><dd><a href=\"/works\">{works}</a></dd>\
         <dt>Cached chapters:</dt><dd>{chapters}</dd>\
         <dt>Subscriptions:</dt><dd><a href=\"/subscriptions\">{subscriptions}</a></dd>\
         <dt>Bookmarks:</dt><dd><a href=\"/bookmarks\">{bookmarks}</a></dd>\
         <dt>Kudos given:</dt><dd>{kudos}</dd>\
         <dt>HTTP failures logged:</dt><dd><a href=\"/diagnostics\">{errors}</a></dd></dl>",
        works = stats.works,
        chapters = stats.chapters,
        subscriptions = stats.subscriptions,
        bookmarks = stats.bookmarks,
        kudos = stats.kudos,
        errors = stats.recent_http_errors,
    ));

    body.push_str(
        "<ul class=\"actions\">\
         <li><a href=\"/works\">Cached library</a></li>\
         <li><a href=\"/fandoms\">Fandoms</a></li>\
         <li><a href=\"/subscriptions\">Subscriptions</a></li>\
         <li><a href=\"/bookmarks\">Bookmarks</a></li>\
         <li><a href=\"/reading\">Marked for later &amp; history</a></li>\
         <li><a href=\"/session\">Session &amp; preferences</a></li>\
         <li><a href=\"/audit\">Write audit</a></li>\
         <li><a href=\"/diagnostics\">HTTP diagnostics</a></li></ul>",
    );

    body.push_str("<h3>Recent subscriptions</h3>");
    body.push_str(&recent_items(
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
                    format!(
                        "/works?tag={}",
                        percent_encode(
                            entry.title.as_deref().unwrap_or(&entry.target_id),
                            SpaceMode::Percent20
                        )
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
    ));

    body.push_str("<h3>Recent bookmarks</h3>");
    body.push_str(&recent_items(
        bookmarks.iter().map(|entry| {
            (
                escape(entry.title.as_deref().unwrap_or(&entry.work_id)),
                format!("/read/{}", escape(&entry.work_id)),
                "bookmarked".to_string(),
            )
        }),
        "No bookmark yet.",
    ));

    body.push_str("<h3>Continue reading</h3>");
    body.push_str(&recent_items(
        reading.iter().filter_map(|entry| {
            entry.progress_chapter.map(|chapter| {
                (
                    escape(entry.title.as_deref().unwrap_or(&entry.work_id)),
                    format!("/read/{}?chapter={chapter}", escape(&entry.work_id)),
                    format!("chapter {chapter}"),
                )
            })
        }),
        "No saved reading position yet.",
    ));

    layout(ctx, &body)
}

fn recent_items<I>(items: I, empty: &str) -> String
where
    I: Iterator<Item = (String, String, String)>,
{
    let items: Vec<(String, String, String)> = items.take(5).collect();
    if items.is_empty() {
        return format!("<p class=\"muted\">{empty}</p>");
    }
    let mut out = String::from("<ul class=\"works\">");
    for (title, href, meta) in items {
        out.push_str(&format!(
            "<li><a href=\"{href}\">{title}</a> <span class=\"meta\">· {meta}</span></li>"
        ));
    }
    out.push_str("</ul>");
    out
}
