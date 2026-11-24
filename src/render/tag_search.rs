//! Tag search modelled on `otwarchive/app/views/tags/search.html.erb`:
//! live AO3 tag search (cached 30 min) with an offline cached-tag fallback.

use super::chrome::{PageContext, layout_with_class};
use super::components::cache_banner;
use super::escape;
use crate::ao3::account_lists::LiveTagSearchResults;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::TagHit;
use crate::time::format_count;

/// Renders live (or 30-minute cached) AO3 Tag Search results.
pub fn live_tag_search_page(
    ctx: &PageContext,
    query: &str,
    results: &LiveTagSearchResults,
    page: i64,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let mut body = tag_search_header(query);
    let q_enc = percent_encode(query.trim(), SpaceMode::Percent20);
    let refresh_href = format!("/tags/search?q={q_enc}&page={page}&refresh=1");
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));

    if results.hits.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\">0 tags match “{}” on AO3.</p>",
            escape(query)
        ));
    } else {
        let total_items = usize::try_from(results.total_pages.saturating_mul(50))
            .unwrap_or(results.total)
            .max(results.total);
        let pager = if results.total_pages > 1 {
            super::pagination(
                page,
                50,
                i64::try_from(total_items).unwrap_or(i64::MAX),
                "/tags/search",
                &format!("q={q_enc}"),
            )
        } else {
            String::new()
        };
        body.push_str(&format!(
            "<h3 class=\"heading\">{} Found</h3>\
             {pager}\
             <h4 class=\"landmark heading\">Listing Tags</h4>\
             <ol class=\"tag index group\">",
            format_count(u64::try_from(results.total).unwrap_or(0))
        ));
        for hit in &results.hits {
            let span_class = if hit.canonical {
                " class=\"canonical\""
            } else {
                ""
            };
            body.push_str(&format!(
                "<li><span{span_class}><span class=\"meta\">{kind}:</span> \
                 <a class=\"tag\" href=\"{}\">{}</a> ({count})</span></li>",
                super::tag_href(&hit.name),
                escape(&hit.name),
                kind = escape(&hit.kind),
                count = format_count(hit.count),
            ));
        }
        body.push_str("</ol>");
        body.push_str(&pager);
    }
    layout_with_class(ctx, "tags-search", &body)
}

/// Offline / empty-query Tag Search view over cached tags.
pub fn tag_search_page(ctx: &PageContext, query: &str, hits: &[TagHit]) -> String {
    let mut body = tag_search_header(query);
    if query.trim().is_empty() {
        body.push_str("<p class=\"muted\">Type a tag name to search AO3 tags (cached 30 min).</p>");
    } else if hits.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\">0 cached tags match “{}”.</p>",
            escape(query)
        ));
    } else {
        body.push_str(&format!(
            "<p class=\"note\">AO3 unreachable; showing matching cached tags.</p>\
             <h3 class=\"heading\">{} Found</h3>\
             <h4 class=\"landmark heading\">Listing Tags</h4>\
             <ol class=\"tag index group\">",
            hits.len()
        ));
        for hit in hits {
            body.push_str(&format!(
                "<li><span class=\"meta\">{kind}:</span> \
                 <a class=\"tag\" href=\"{}\">{}</a> ({count})</li>",
                super::tag_href(&hit.name),
                escape(&hit.name),
                kind = escape(&hit.kind),
                count = hit.count,
            ));
        }
        body.push_str("</ol>");
    }
    layout_with_class(ctx, "tags-search", &body)
}

fn tag_search_header(query: &str) -> String {
    format!(
        "<h2 class=\"heading\">Tag Search</h2>\
         <ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/works/search\">Work Search</a></li>\
         <li><a href=\"/works?author=\">People Search</a></li>\
         <li><a href=\"/bookmarks\">Bookmarks</a></li>\
         <li class=\"active\"><span class=\"current\">Tag Search</span></li></ul>\
         <form class=\"search verbose\" method=\"get\" action=\"/tags/search\">\
         <fieldset><legend>Search Tags</legend><dl>\
         <dt><label for=\"tag_search_name\">Tag name</label></dt>\
         <dd><input id=\"tag_search_name\" type=\"search\" name=\"q\" value=\"{}\" \
         placeholder=\"tag name\"></dd></dl>\
         <p class=\"submit actions\"><button type=\"submit\">Search</button></p>\
         </fieldset></form>",
        escape(query)
    )
}
