//! Local lists: subscriptions, bookmarks and reading history.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::{BookmarkEntry, ReadingEntry, SubscriptionEntry};
use crate::time::format_utc;

pub fn subscriptions_page(ctx: &PageContext, entries: &[SubscriptionEntry]) -> String {
    let mut body = String::from("<h2>Subscriptions</h2>");
    body.push_str(
        "<p class=\"meta\">The local ledger of subscriptions this instance created. \
         AO3 allows duplicates, so this list is the client's own pre-check.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No subscription recorded yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>work</th><th>kind</th><th>state</th>\
             <th>updated</th><th></th></tr></thead><tbody>",
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
            body.push_str(&format!(
                "<tr><td><a href=\"/read/{id}\">{title}</a></td><td>{kind}</td>\
                 <td>{state}</td><td>{updated}</td><td>{action}</td></tr>",
                id = escape(&entry.target_id),
                title = escape(title),
                kind = escape(&entry.target_kind),
                state = escape(&entry.state),
                updated = escape(&format_utc(entry.updated_at)),
            ));
        }
        body.push_str("</tbody></table>");
    }
    layout(ctx, &body)
}

pub fn bookmarks_page(ctx: &PageContext, entries: &[BookmarkEntry]) -> String {
    let mut body = String::from("<h2>Bookmarks</h2>");
    body.push_str(
        "<p class=\"meta\">Works bookmarked from this instance. The AO3 bookmark itself \
         lives upstream; this is the local receipt.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No bookmark yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>work</th><th>by</th><th>saved</th>\
             </tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.work_id);
            body.push_str(&format!(
                "<tr><td><a href=\"/read/{id}\">{title}</a></td><td>{authors}</td>\
                 <td>{saved}</td></tr>",
                id = escape(&entry.work_id),
                title = escape(title),
                authors = escape(&entry.authors.join(", ")),
                saved = escape(&format_utc(entry.created_at)),
            ));
        }
        body.push_str("</tbody></table>");
    }
    layout(ctx, &body)
}

pub fn reading_page(ctx: &PageContext, entries: &[ReadingEntry]) -> String {
    let mut body = String::from("<h2>Marked for later &amp; history</h2>");
    body.push_str(
        "<p class=\"meta\">Reading positions and AO3 reading flags saved by this instance.</p>",
    );
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">Nothing marked yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>work</th><th>progress</th><th>flag</th>\
             </tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.work_id);
            let progress = entry.progress_chapter.map_or_else(
                || "—".to_string(),
                |chapter| {
                    format!(
                        "<a href=\"/read/{id}?chapter={chapter}\">chapter {chapter}</a>",
                        id = escape(&entry.work_id)
                    )
                },
            );
            body.push_str(&format!(
                "<tr><td><a href=\"/read/{id}\">{title}</a></td><td>{progress}</td>\
                 <td>{flag}</td></tr>",
                id = escape(&entry.work_id),
                title = escape(title),
                flag = escape(entry.reading_state.as_deref().unwrap_or("—")),
            ));
        }
        body.push_str("</tbody></table>");
    }
    layout(ctx, &body)
}
