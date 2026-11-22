//! Home / splash page.

use super::chrome::layout;
use super::escape;
use crate::store::WorkSummary;
pub fn home_page(works: &[WorkSummary], flash: Option<&str>) -> String {
    let mut body = String::from(
        "<form class=\"open\" method=\"get\" action=\"/search\">\
         <input name=\"q\" placeholder=\"Search cached works (title, author, tag)\">\
         <button type=\"submit\">Search</button></form>\
         <form class=\"open\" method=\"post\" action=\"/open\">\
         <input name=\"id\" placeholder=\"AO3 work id or URL (e.g. 12345678)\" autofocus required>\
         <button type=\"submit\">Open work</button></form>",
    );
    if let Some(flash) = flash {
        body.push_str(&format!("<div class=\"notice\">{}</div>", escape(flash)));
    }
    body.push_str("<div class=\"meta\">cached works stay readable offline</div>");
    if works.is_empty() {
        body.push_str(
            "<p class=\"muted\">Nothing cached yet. Paste a work id or URL above; \
             the work page is fetched once and kept for offline reading.</p>",
        );
    } else {
        body.push_str("<h2>Cached works</h2><ul class=\"works\">");
        for work in works {
            let authors = if work.authors.is_empty() {
                "Anonymous".to_string()
            } else {
                work.authors.join(", ")
            };
            let continue_at = match work.progress_chapter {
                Some(chapter) => format!(
                    " · <a href=\"/read/{}?chapter={chapter}\">continue ch. {chapter}</a>",
                    escape(&work.ao3_id)
                ),
                None => String::new(),
            };
            body.push_str(&format!(
                "<li><a href=\"/read/{id}\">{title}</a>\
                 <div class=\"meta\">{authors} · {chapters} chapter(s){continue_at}</div></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                authors = escape(&authors),
                chapters = work.chapter_count,
                continue_at = continue_at,
            ));
        }
        body.push_str("</ul>");
    }
    layout("AO3Instance", &body)
}
