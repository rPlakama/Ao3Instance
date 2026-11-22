//! Cached-fandoms directory.

use super::chrome::{PageContext, layout};
use super::escape;

pub fn fandoms_page(ctx: &PageContext, fandoms: &[(String, usize)]) -> String {
    let mut body = String::from("<h2>Cached fandoms</h2>");
    body.push_str(
        "<p class=\"meta\">Only fandoms present in the local cache are listed; \
         this page never calls archiveofourown.org.</p>",
    );
    if fandoms.is_empty() {
        body.push_str(
            "<p class=\"muted\">No fandom yet. Open a work from the start page to populate \
             the cache.</p>",
        );
    } else {
        body.push_str("<ul class=\"tags commas\">");
        for (fandom, count) in fandoms {
            body.push_str(&format!(
                "<li><a href=\"/works?fandom={}\">{} <span class=\"meta\">({count})</span></a></li>",
                escape(&crate::ao3::parse::percent_encode(
                    fandom,
                    crate::ao3::parse::SpaceMode::Percent20
                )),
                escape(fandom),
            ));
        }
        body.push_str("</ul>");
    }
    layout(ctx, &body)
}
