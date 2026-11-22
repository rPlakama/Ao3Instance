//! Home / splash page.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::WorkSummary;

pub fn home_page(ctx: &PageContext, works: &[WorkSummary], flash: Option<&str>) -> String {
    let mut body = String::from(
        "<section class=\"splash\">\
         <div><h2>Your offline AO3</h2>\
         <p class=\"meta\">Cached works stay readable when the archive is down. \
         The cache is yours; content belongs to its authors.</p></div>\
         <div id=\"open-work\"><form class=\"open\" method=\"post\" action=\"/open\">\
         <input name=\"id\" placeholder=\"AO3 work id or URL (e.g. 12345678)\" autofocus required>\
         <button type=\"submit\">Open work</button></form></div></section>\
         <form class=\"open\" method=\"get\" action=\"/search\">\
         <input type=\"search\" name=\"q\" placeholder=\"Search cached works (title, author, tag)\">\
         <button type=\"submit\">Search</button></form>",
    );
    if let Some(flash) = flash {
        body.push_str(&format!(
            "<div class=\"flash notice\">{}</div>",
            escape(flash)
        ));
    }
    let continuing: Vec<&WorkSummary> = works
        .iter()
        .filter(|work| work.progress_chapter.is_some())
        .take(6)
        .collect();
    if !continuing.is_empty() {
        body.push_str("<h2>Continue Reading</h2><ul class=\"works\">");
        for work in continuing {
            let chapter = work.progress_chapter.unwrap_or(1);
            let authors = if work.authors.is_empty() {
                "Anonymous".to_string()
            } else {
                work.authors.join(", ")
            };
            body.push_str(&format!(
                "<li><a href=\"/read/{id}?chapter={chapter}\">{title}</a> \
                 <span class=\"meta\">{authors} · chapter {chapter}</span></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                authors = escape(&authors),
            ));
        }
        body.push_str("</ul>");
    }
    if works.is_empty() {
        body.push_str(
            "<p class=\"muted\">Nothing cached yet. Paste a work id or URL above; \
             the work page is fetched once and kept for offline reading.</p>",
        );
    } else {
        body.push_str("<h2>Cached works</h2>");
        for work in works {
            body.push_str(&render_blurb(work));
        }
    }
    layout(ctx, &body)
}

/// A summary-only blurb. Richer tags/stats arrive with the store enrichment.
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
