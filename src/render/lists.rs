//! Account lists: live AO3 subscriptions, bookmarks, and reading history /
//! Marked for Later (with 30-minute SQLite cache), plus local offline fallback.

use std::collections::{HashMap, HashSet};

use super::blurb::live_blurb;
use super::chrome::{PageContext, layout};
use super::components::{author_links_live, author_links_local, cache_banner};
use super::{escape, sanitize};
use crate::ao3::account_lists::{LiveBookmarksPage, LiveReadingsPage, LiveSubscriptionsPage};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BookmarkEntry, ReadingEntry, SubscriptionEntry};
use crate::time::{format_count, format_utc};

/// Renders live (or 30-minute cached) AO3 subscriptions from
/// `GET /users/:login/subscriptions`.
pub fn live_subscriptions_page(
    ctx: &PageContext,
    page: &LiveSubscriptionsPage,
    current_page: i64,
    filter_type: Option<&str>,
    cached_works: &HashSet<String>,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let mut body = String::from("<h2 class=\"heading\">Subscriptions</h2>");
    body.push_str("<ul class=\"navigation actions\" role=\"navigation\">");
    for (label, value) in [
        ("All Subscriptions", None),
        ("Series", Some("series")),
        ("Users", Some("users")),
        ("Works", Some("works")),
    ] {
        let href = value.map_or_else(
            || "/subscriptions".to_string(),
            |kind| format!("/subscriptions?type={kind}"),
        );
        if filter_type == value {
            body.push_str(&format!(
                "<li class=\"active\"><span class=\"current\">{label}</span></li>"
            ));
        } else {
            body.push_str(&format!("<li><a href=\"{href}\">{label}</a></li>"));
        }
    }
    let refresh_href = filter_type.map_or_else(
        || format!("/subscriptions?page={current_page}&refresh=1"),
        |kind| format!("/subscriptions?type={kind}&page={current_page}&refresh=1"),
    );
    body.push_str(&format!(
        "<li><a href=\"{refresh_href}\">Refresh from AO3</a></li></ul>"
    ));
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));

    let extra_query = filter_type.map_or_else(String::new, |kind| format!("type={kind}"));
    let pager = page_pager(
        current_page,
        page.total_pages,
        "/subscriptions",
        &extra_query,
    );
    if page.items.is_empty() {
        body.push_str("<p class=\"muted\">No subscriptions on AO3 for this filter.</p>");
    } else {
        body.push_str(&pager);
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>Subscription</th>\
             <th>Kind</th><th>Action</th></tr></thead><tbody>",
        );
        for item in &page.items {
            let title_html = match item.kind.as_str() {
                "Work" if !item.target_id.is_empty() => {
                    let href = if cached_works.contains(&item.target_id) {
                        format!("/read/{}", escape(&item.target_id))
                    } else {
                        format!("/open/{}", escape(&item.target_id))
                    };
                    format!("<a href=\"{href}\">{}</a>", escape(&item.title))
                }
                "Series" if !item.target_id.is_empty() => format!(
                    "<a href=\"https://archiveofourown.org/series/{}\" rel=\"noopener noreferrer\">{}</a>",
                    escape(&item.target_id),
                    escape(&item.title)
                ),
                "User" if !item.target_id.is_empty() => format!(
                    "<a href=\"/works/search?creator={}\">{}</a>",
                    percent_encode(&item.target_id, SpaceMode::Percent20),
                    escape(&item.title)
                ),
                _ => escape(&item.title),
            };
            let byline = if item.authors.is_empty() {
                String::new()
            } else {
                format!(
                    "<div class=\"byline\">by {}</div>",
                    author_links_live(&item.authors)
                )
            };
            let action = if ctx.write_enabled
                && let Some(sub_id) = item.subscription_id
            {
                format!(
                    "<form class=\"inline\" method=\"post\" action=\"/write/unsubscribe\">\
                     <input type=\"hidden\" name=\"kind\" value=\"{kind}\">\
                     <input type=\"hidden\" name=\"target_id\" value=\"{target}\">\
                     <input type=\"hidden\" name=\"subscription_id\" value=\"{sub_id}\">\
                     <button type=\"submit\">Unsubscribe</button></form>",
                    kind = escape(&item.kind),
                    target = escape(&item.target_id),
                )
            } else {
                "—".to_string()
            };
            body.push_str(&format!(
                "<tr><td>{title_html}{byline}</td><td>{}</td><td>{action}</td></tr>",
                escape(&item.kind),
            ));
        }
        body.push_str("</tbody></table></div>");
        body.push_str(&pager);
    }
    layout(ctx, &body)
}

/// Renders live (or 30-minute cached) AO3 bookmarks from
/// `GET /users/:login/bookmarks`.
pub fn live_bookmarks_page(
    ctx: &PageContext,
    page: &LiveBookmarksPage,
    current_page: i64,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let heading = if page.total > 0 {
        format!(
            "{} Bookmark{}",
            format_count(u64::try_from(page.total).unwrap_or(0)),
            if page.total == 1 { "" } else { "s" }
        )
    } else {
        "Bookmarks".to_string()
    };
    let refresh_href = format!("/bookmarks?page={current_page}&refresh=1");
    let mut body = format!(
        "<h2 class=\"heading\">{heading}</h2>\
         <ul class=\"navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">AO3 Bookmarks</span></li>\
         <li><a href=\"/bookmarks?local=1\">Local Cache</a></li>\
         <li><a href=\"{refresh_href}\">Refresh from AO3</a></li></ul>"
    );
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));

    let total_pages = page
        .total_pages
        .max(i64::try_from(page.total.div_ceil(20)).unwrap_or(1));
    let pager = page_pager(current_page, total_pages, "/bookmarks", "");
    if page.items.is_empty() {
        body.push_str("<p class=\"muted\">No bookmarks on AO3 yet.</p>");
    } else {
        body.push_str(&pager);
        body.push_str("<ol class=\"bookmark index group\">");
        for item in &page.items {
            if let Some(work) = &item.work {
                body.push_str(&live_blurb(work));
            } else {
                let title = item
                    .fallback_title
                    .as_deref()
                    .unwrap_or("Unavailable bookmark");
                let heading_html = item.fallback_href.as_deref().map_or_else(
                    || escape(title),
                    |href| {
                        format!(
                            "<a href=\"https://archiveofourown.org{}\" rel=\"noopener noreferrer\">{}</a>",
                            escape(href),
                            escape(title)
                        )
                    },
                );
                body.push_str(&format!(
                    "<li class=\"bookmark blurb group\"><h4 class=\"heading\">{heading_html}</h4></li>"
                ));
            }

            let mut meta_parts = Vec::new();
            if item.is_private {
                meta_parts.push("Private".to_string());
            }
            if item.is_rec {
                meta_parts.push("Rec".to_string());
            }
            if !item.bookmarked_at.is_empty() {
                meta_parts.push(format!("Bookmarked {}", escape(&item.bookmarked_at)));
            }
            let has_user_info =
                !meta_parts.is_empty() || !item.tags.is_empty() || item.notes_html.is_some();
            if has_user_info {
                body.push_str("<div class=\"own user module group\">");
                if !meta_parts.is_empty() {
                    body.push_str(&format!("<p class=\"meta\">{}</p>", meta_parts.join(" · ")));
                }
                if !item.tags.is_empty() {
                    body.push_str("<ul class=\"meta tags commas\">");
                    for tag in &item.tags {
                        body.push_str(&format!(
                            "<li><a class=\"tag\" href=\"{}\">{}</a></li>",
                            super::tag_href(tag),
                            escape(tag)
                        ));
                    }
                    body.push_str("</ul>");
                }
                if let Some(notes) = &item.notes_html {
                    body.push_str(&format!(
                        "<blockquote class=\"userstuff notes\">{}</blockquote>",
                        sanitize(notes)
                    ));
                }
                body.push_str("</div>");
            }
        }
        body.push_str("</ol>");
        body.push_str(&pager);
    }
    layout(ctx, &body)
}

/// Renders live (or 30-minute cached) AO3 History / Marked for Later from
/// `GET /users/:login/readings` (and `?show=to-read`).
pub fn live_readings_page(
    ctx: &PageContext,
    page: &LiveReadingsPage,
    current_page: i64,
    marked_for_later_only: bool,
    local_progress: &HashMap<String, (i64, i64)>,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let heading = if marked_for_later_only {
        "Marked for Later"
    } else {
        "History"
    };
    let refresh_href = if marked_for_later_only {
        format!("/reading?show=to-read&page={current_page}&refresh=1")
    } else {
        format!("/reading?page={current_page}&refresh=1")
    };
    let mut body = format!(
        "<h2 class=\"heading\">{heading}</h2>\
         <ul class=\"navigation actions\" role=\"navigation\">"
    );
    if marked_for_later_only {
        body.push_str(
            "<li><a href=\"/reading\">Full History</a></li>\
             <li class=\"active\"><span class=\"current\">Marked for Later</span></li>",
        );
    } else {
        body.push_str(
            "<li class=\"active\"><span class=\"current\">Full History</span></li>\
             <li><a href=\"/reading?show=to-read\">Marked for Later</a></li>",
        );
    }
    body.push_str(&format!(
        "<li><a href=\"/reading?local=1\">Local Progress</a></li>\
         <li><a href=\"{refresh_href}\">Refresh from AO3</a></li></ul>"
    ));
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));

    let extra_query = if marked_for_later_only {
        "show=to-read"
    } else {
        ""
    };
    let pager = page_pager(current_page, page.total_pages, "/reading", extra_query);
    if page.items.is_empty() {
        body.push_str("<p class=\"muted\">No works in this AO3 reading list yet.</p>");
    } else {
        body.push_str(&pager);
        body.push_str("<ol class=\"reading work index group\">");
        for item in &page.items {
            if let Some(work) = &item.work {
                body.push_str(&live_blurb(work));
                let progress_badge = local_progress.get(&work.ao3_id).map(|(chapter, pct)| {
                    format!(
                        " · <a href=\"/read/{}?chapter={chapter}\">Local progress: chapter {chapter} ({pct}%)</a>",
                        escape(&work.ao3_id)
                    )
                });
                if !item.viewed_info.is_empty() || progress_badge.is_some() {
                    body.push_str(&format!(
                        "<div class=\"user module group\"><h4 class=\"viewed heading\">{}{}</h4></div>",
                        escape(&item.viewed_info),
                        progress_badge.unwrap_or_default()
                    ));
                }
            } else if !item.viewed_info.is_empty() {
                body.push_str(&format!(
                    "<li class=\"deleted reading work blurb group\">\
                     <div class=\"user module group\"><h4 class=\"viewed heading\">{}</h4></div></li>",
                    escape(&item.viewed_info)
                ));
            }
        }
        body.push_str("</ol>");
        body.push_str(&pager);
    }
    layout(ctx, &body)
}

pub fn subscriptions_page(ctx: &PageContext, entries: &[SubscriptionEntry]) -> String {
    let mut body = String::from("<h2 class=\"heading\">Subscriptions</h2>");
    body.push_str(
        "<p class=\"note\">Showing cached subscriptions. Sign in on \
         <a href=\"/session\">Session</a> to load your full AO3 subscriptions list.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No subscriptions found.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>Work</th><th>Kind</th>\
             <th>State</th><th>Updated</th><th>Action</th></tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.target_id);
            let action = if entry.state == "active" && ctx.write_enabled {
                format!(
                    "<form class=\"inline\" method=\"post\" action=\"/write/unsubscribe\">\
                     <input type=\"hidden\" name=\"kind\" value=\"{kind}\">\
                     <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                     <button type=\"submit\">Unsubscribe</button></form>",
                    kind = escape(&entry.target_kind),
                    id = escape(&entry.target_id),
                )
            } else {
                "—".to_string()
            };
            let authors = author_links_local(&entry.authors);
            let title_html = match entry.target_kind.as_str() {
                "Work" => format!(
                    "<a href=\"/read/{}\">{}</a>",
                    escape(&entry.target_id),
                    escape(title)
                ),
                "Tag" => format!(
                    "<a class=\"tag\" href=\"{}\">{}</a>",
                    super::tag_href(title),
                    escape(title)
                ),
                _ => escape(title),
            };
            body.push_str(&format!(
                "<tr><td>{title_html}\
                 <div class=\"byline\">by {authors}</div></td><td>{kind}</td>\
                 <td>{state}</td><td class=\"datetime\">{updated}</td><td>{action}</td></tr>",
                kind = escape(&entry.target_kind),
                state = escape(&entry.state),
                updated = escape(&format_utc(entry.updated_at)),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    layout(ctx, &body)
}

pub fn bookmarks_page(ctx: &PageContext, entries: &[BookmarkEntry]) -> String {
    let mut body = String::from("<h2 class=\"heading\">Bookmarks</h2>");
    body.push_str(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/bookmarks\">AO3 Bookmarks</a></li>\
         <li class=\"active\"><span class=\"current\">Local Cache</span></li></ul>\
         <p class=\"note\">Showing locally cached bookmarks. Sign in on \
         <a href=\"/session\">Session</a> to load your full AO3 bookmarks.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No bookmarks found.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>Work</th><th>By</th>\
             <th>Saved</th></tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.work_id);
            body.push_str(&format!(
                "<tr><td><a href=\"/read/{id}\">{title}</a></td><td>{authors}</td>\
                 <td class=\"datetime\">{saved}</td></tr>",
                id = escape(&entry.work_id),
                title = escape(title),
                authors = author_links_local(&entry.authors),
                saved = escape(&format_utc(entry.created_at)),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    layout(ctx, &body)
}

pub fn reading_page(ctx: &PageContext, entries: &[ReadingEntry]) -> String {
    let mut body = String::from("<h2 class=\"heading\">Marked for later &amp; history</h2>");
    body.push_str(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/reading\">Full History</a></li>\
         <li><a href=\"/reading?show=to-read\">Marked for Later</a></li>\
         <li class=\"active\"><span class=\"current\">Local Progress</span></li></ul>\
         <p class=\"note\">Saved local reading positions. Sign in on \
         <a href=\"/session\">Session</a> to load your live AO3 History and Marked for Later list.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">Nothing marked yet.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>Work</th>\
             <th>Progress</th><th>Flag</th></tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.work_id);
            let progress = entry.progress_chapter.map_or_else(
                || "—".to_string(),
                |chapter| {
                    let pct = entry.progress_pct.map_or(0, |pct| {
                        super::overall_percent(chapter, pct, entry.chapter_count)
                    });
                    format!(
                        "<a href=\"/read/{id}?chapter={chapter}\">chapter {chapter} · {pct}%</a>",
                        id = escape(&entry.work_id)
                    )
                },
            );
            body.push_str(&format!(
                "<tr><td><a href=\"/read/{id}\">{title}</a>\
                 <div class=\"byline\">by {authors}</div></td><td>{progress}</td>\
                 <td>{flag}</td></tr>",
                id = escape(&entry.work_id),
                title = escape(title),
                authors = author_links_local(&entry.authors),
                flag = escape(entry.reading_state.as_deref().unwrap_or("—")),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    layout(ctx, &body)
}

fn page_pager(current: i64, total_pages: i64, base: &str, extra_query: &str) -> String {
    if total_pages <= 1 {
        return String::new();
    }
    let total_items = total_pages.saturating_mul(20);
    super::pagination(current, 20, total_items, base, extra_query)
}
