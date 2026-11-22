//! Offline search results over the local cache.

use super::chrome::layout;
use super::escape;
use crate::store::WorkSummary;
/// Search results over the local cache.
pub fn search_page(query: &str, works: &[WorkSummary]) -> String {
    let mut body = format!(
        "<h2>Search</h2><form class=\"open\" method=\"get\" action=\"/search\">\
         <input name=\"q\" value=\"{}\" placeholder=\"title, author or tag\">\
         <button type=\"submit\">Search</button></form>",
        escape(query)
    );
    if query.trim().is_empty() {
        body.push_str("<p class=\"muted\">Search the works cached on this instance.</p>");
    } else if works.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\">Nothing cached matches “{}”. Open the work from the start \
             page first; offline search only covers what has been cached.</p>",
            escape(query)
        ));
    } else {
        body.push_str(&format!(
            "<p class=\"meta\">{} match(es)</p><ul class=\"works\">",
            works.len()
        ));
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
    layout("Search — AO3Instance", &body)
}
