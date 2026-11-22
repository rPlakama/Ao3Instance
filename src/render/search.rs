//! Offline search results over the local cache, with paging.

use super::blurb::work_blurb;
use super::chrome::{PageContext, layout};
use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::SearchPage;

pub fn search_page(ctx: &PageContext, query: &str, result: &SearchPage) -> String {
    let mut body = format!(
        "<h2>Search</h2><form class=\"open\" method=\"get\" action=\"/search\">\
         <input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"title, author or tag\">\
         <button type=\"submit\">Search</button></form>\
         <p class=\"meta\">Local Cache — offline search. This never calls archiveofourown.org.</p>",
        escape(query)
    );
    if query.trim().is_empty() {
        body.push_str("<p class=\"muted\">Search the works cached on this instance.</p>");
    } else if result.total == 0 {
        body.push_str(&format!(
            "<p class=\"muted\">0 cached matches for “{}”. Open the work from the start \
             page first; offline search only covers what has been cached.</p>",
            escape(query)
        ));
    } else {
        body.push_str(&format!("<h3>{} cached match(es)</h3>", result.total));
        for work in &result.works {
            body.push_str(&work_blurb(work, None));
        }
        body.push_str(&pagination(query, result));
    }
    layout(ctx, &body)
}

fn pagination(query: &str, result: &SearchPage) -> String {
    let pages = (result.total + result.per_page - 1) / result.per_page;
    if pages <= 1 {
        return String::new();
    }
    let encoded = percent_encode(query, SpaceMode::Percent20);
    let mut out = String::from("<ol class=\"pagination actions\">");
    if result.page > 1 {
        out.push_str(&format!(
            "<li><a href=\"/search?q={encoded}&page={}\">← previous</a></li>",
            result.page - 1
        ));
    }
    out.push_str(&format!(
        "<li><span class=\"state\">page {}/{pages}</span></li>",
        result.page
    ));
    if result.page < pages {
        out.push_str(&format!(
            "<li><a href=\"/search?q={encoded}&page={}\">next →</a></li>",
            result.page + 1
        ));
    }
    out.push_str("</ol>");
    out
}
