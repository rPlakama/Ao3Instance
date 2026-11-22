//! Offline search results over the local cache.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::WorkSummary;

pub fn search_page(ctx: &PageContext, query: &str, works: &[WorkSummary]) -> String {
    let mut body = format!(
        "<h2>Search</h2><form class=\"open\" method=\"get\" action=\"/search\">\
         <input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"title, author or tag\">\
         <button type=\"submit\">Search</button></form>\
         <p class=\"meta\">Local cache only — this search never calls archiveofourown.org.</p>",
        escape(query)
    );
    if query.trim().is_empty() {
        body.push_str("<p class=\"muted\">Search the works cached on this instance.</p>");
    } else if works.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\">0 cached matches for “{}”. Open the work from the start \
             page first; offline search only covers what has been cached.</p>",
            escape(query)
        ));
    } else {
        body.push_str(&format!("<h3>{} cached match(es)</h3>", works.len()));
        for work in works {
            body.push_str(&render_blurb(work));
        }
    }
    layout(ctx, &body)
}

fn render_blurb(work: &WorkSummary) -> String {
    let authors = if work.authors.is_empty() {
        "Anonymous".to_string()
    } else {
        work.authors.join(", ")
    };
    let continue_at = work.progress_chapter.map_or_else(String::new, |chapter| {
        format!(
            " · <a href=\"/read/{}?chapter={chapter}\">continue ch. {chapter}</a>",
            escape(&work.ao3_id)
        )
    });
    format!(
        "<article class=\"work blurb group\"><div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/read/{id}\">{title}</a></h4>\
         <div class=\"byline\">by {authors}</div></div>\
         <dl class=\"stats\"><dt>chapters</dt><dd>{chapters}</dd></dl>\
         <p class=\"meta\">{continue_at}</p></article>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        authors = escape(&authors),
        chapters = work.chapter_count,
    )
}
