//! Local cache history: recent reads/caches, with a range dropdown.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::AccessEntry;
use crate::time::format_utc;

pub fn history_page(ctx: &PageContext, entries: &[AccessEntry], range: &str) -> String {
    let mut body = String::from("<h2 class=\"heading\">History</h2>");
    body.push_str(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">Local History</span></li>\
         <li><a href=\"/reading\">Reading positions &amp; flags</a></li></ul>",
    );
    body.push_str(
        "<form class=\"inline filter-bar\" method=\"get\" action=\"/history\">\
         <label for=\"history_range\">Show</label>\
         <select id=\"history_range\" name=\"range\">",
    );
    for (value, label) in [
        ("today", "Today"),
        ("week", "Last 7 days"),
        ("all", "Everything"),
    ] {
        body.push_str(&format!(
            "<option value=\"{value}\"{}>{label}</option>",
            if range == value { " selected" } else { "" }
        ));
    }
    body.push_str(
        "</select><button type=\"submit\">Apply</button></form>\
         <p class=\"note\">Local cache history — the one thing AO3 itself does not keep here.</p>",
    );

    if entries.is_empty() {
        body.push_str("<p class=\"muted\">Nothing accessed in this range yet.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>When</th><th>Work</th>\
             <th>Action</th></tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.work_id);
            body.push_str(&format!(
                "<tr><td class=\"datetime\">{}</td><td><a href=\"/read/{id}\">{title}</a></td>\
                 <td>{kind}</td></tr>",
                escape(&format_utc(entry.accessed_at)),
                id = escape(&entry.work_id),
                title = escape(title),
                kind = escape(&entry.kind),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    layout(ctx, &body)
}
