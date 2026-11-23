//! Local lists: subscriptions, bookmarks and reading history.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BookmarkEntry, ReadingEntry, SubscriptionEntry};
use crate::time::format_utc;

pub fn subscriptions_page(ctx: &PageContext, entries: &[SubscriptionEntry]) -> String {
    let mut body = String::from("<h2 class=\"heading\">Subscriptions</h2>");
    body.push_str(
        "<p class=\"note\">The local ledger of subscriptions this instance created. \
         AO3 allows duplicates, so this list is the client's own pre-check.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No subscription recorded yet.</p>");
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
            let authors = author_cell(&entry.authors);
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
        "<p class=\"note\">Works bookmarked from this instance. The AO3 bookmark itself \
         lives upstream; this is the local receipt.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No bookmark yet.</p>");
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
                authors = author_cell(&entry.authors),
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
        "<p class=\"note\">Reading positions and AO3 reading flags saved by this instance.</p>",
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
                authors = author_cell(&entry.authors),
                flag = escape(entry.reading_state.as_deref().unwrap_or("—")),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    layout(ctx, &body)
}

/// Author names as local-filter links (entries, not decorations).
fn author_cell(authors: &[String]) -> String {
    if authors.is_empty() {
        return "Anonymous".to_string();
    }
    authors
        .iter()
        .map(|author| {
            format!(
                "<a href=\"/works?author={}\">{}</a>",
                percent_encode(author, SpaceMode::Percent20),
                escape(author)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
