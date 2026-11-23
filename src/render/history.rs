//! Local cache history: recent reads/caches, with a range dropdown.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::AccessEntry;
use crate::time::format_utc;

pub fn history_page(ctx: &PageContext, entries: &[AccessEntry], range: &str) -> String {
    let mut body = String::from("<h2 class=\"heading\">History</h2>");
    body.push_str(
        "<form class=\"filters\" method=\"get\" action=\"/history\">\
         <label>Show<select name=\"range\">",
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
        "</select></label><button type=\"submit\">Apply</button></form>\
         <p class=\"meta\">Local cache history — the one thing AO3 itself does not keep here.</p>",
    );

    if entries.is_empty() {
        body.push_str("<p class=\"muted\">Nothing accessed in this range yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>when</th><th>work</th><th>action</th>\
             </tr></thead><tbody>",
        );
        for entry in entries {
            let title = entry.title.as_deref().unwrap_or(&entry.work_id);
            body.push_str(&format!(
                "<tr><td>{}</td><td><a href=\"/read/{id}\">{title}</a></td>\
                 <td>{kind}</td></tr>",
                escape(&format_utc(entry.accessed_at)),
                id = escape(&entry.work_id),
                title = escape(title),
                kind = escape(&entry.kind),
            ));
        }
        body.push_str("</tbody></table>");
    }
    body.push_str("<p><a href=\"/reading\">Reading positions &amp; flags</a></p>");
    layout(ctx, &body)
}
