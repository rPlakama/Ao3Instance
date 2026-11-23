//! Cached tag search: find a tag, then continue into an AO3 tag search.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::TagHit;

pub fn tag_search_page(ctx: &PageContext, query: &str, hits: &[TagHit]) -> String {
    let mut body = format!(
        "<h2 class=\"heading\">Tag Search</h2>\
         <form class=\"open\" method=\"get\" action=\"/tags/search\">\
         <input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"tag name\">\
         <button type=\"submit\">Search</button></form>\
         <p class=\"meta\">Searches the tags present in the local cache; selecting one \
         continues with a live AO3 search.</p>",
        escape(query)
    );
    if query.trim().is_empty() {
        body.push_str("<p class=\"muted\">Type a tag name to find cached tags.</p>");
    } else if hits.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\">0 cached tags match “{}”.</p>",
            escape(query)
        ));
    } else {
        body.push_str("<ul class=\"tags commas\">");
        for hit in hits {
            body.push_str(&format!(
                "<li><a href=\"{}\">{}</a> \
                 <span class=\"meta\">{kind} · {count} work(s)</span></li>",
                super::tag_href(&hit.name),
                escape(&hit.name),
                kind = escape(&hit.kind),
                count = hit.count,
            ));
        }
        body.push_str("</ul>");
    }
    layout(ctx, &body)
}
