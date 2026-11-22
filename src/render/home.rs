//! Home / splash page.

use super::blurb::work_blurb;
use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::{StoredWork, WorkSummary};

pub fn home_page(
    ctx: &PageContext,
    works: &[WorkSummary],
    detailed: &[StoredWork],
    fandoms: &[(String, usize)],
    flash: Option<&str>,
) -> String {
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

    if !fandoms.is_empty() {
        body.push_str("<h3>Cached Fandoms</h3><ul class=\"tags commas\">");
        for (fandom, count) in fandoms.iter().take(10) {
            body.push_str(&format!(
                "<li><a href=\"/works?fandom={}\">{} <span class=\"meta\">({count})</span></a></li>",
                escape(&crate::ao3::parse::percent_encode(
                    fandom,
                    crate::ao3::parse::SpaceMode::Percent20
                )),
                escape(fandom),
            ));
        }
        body.push_str("</ul><p class=\"meta\"><a href=\"/fandoms\">All cached fandoms →</a></p>");
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
            body.push_str(&format!(
                "<li><a href=\"/read/{id}?chapter={chapter}\">{title}</a> \
                 <span class=\"meta\">· continue ch. {chapter}</span></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
            ));
        }
        body.push_str("</ul>");
    }

    if detailed.is_empty() {
        body.push_str(
            "<p class=\"muted\">Nothing cached yet. Paste a work id or URL above; \
             the work page is fetched once and kept for offline reading.</p>",
        );
    } else {
        body.push_str("<h2>Cached works</h2>");
        for work in detailed {
            body.push_str(&work_blurb(work, None));
        }
    }
    layout(ctx, &body)
}
