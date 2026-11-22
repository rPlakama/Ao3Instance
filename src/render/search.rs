//! Offline search results over the local cache.

use super::blurb::work_blurb;
use super::chrome::{PageContext, layout};
use super::escape;
use crate::store::StoredWork;

pub fn search_page(ctx: &PageContext, query: &str, works: &[StoredWork]) -> String {
    let mut body = format!(
        "<h2>Search</h2><form class=\"open\" method=\"get\" action=\"/search\">\
         <input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"title, author or tag\">\
         <button type=\"submit\">Search</button></form>\
         <p class=\"meta\">Local Cache — offline search. This never calls archiveofourown.org.</p>",
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
            body.push_str(&work_blurb(work, None));
        }
    }
    layout(ctx, &body)
}
