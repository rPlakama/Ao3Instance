//! Cached tag search modelled on `otwarchive/app/views/tags/search.html.erb`.

use super::chrome::{PageContext, layout_with_class};
use super::escape;
use crate::store::TagHit;

pub fn tag_search_page(ctx: &PageContext, query: &str, hits: &[TagHit]) -> String {
    let mut body = format!(
        "<h2 class=\"heading\">Tag Search</h2>\
         <ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/works/search\">Work Search</a></li>\
         <li><a href=\"/works?author=\">People Search</a></li>\
         <li><a href=\"/bookmarks\">Bookmark Search</a></li>\
         <li class=\"active\"><span class=\"current\">Tag Search</span></li></ul>\
         <form class=\"search verbose\" method=\"get\" action=\"/tags/search\">\
         <fieldset><legend>Search Tags</legend><dl>\
         <dt><label for=\"tag_search_name\">Tag name</label></dt>\
         <dd><input id=\"tag_search_name\" type=\"search\" name=\"q\" value=\"{}\" \
         placeholder=\"tag name\"></dd></dl>\
         <p class=\"submit actions\"><button type=\"submit\">Search</button></p>\
         </fieldset></form>\
         <p class=\"note\">Searches the tags present in the local cache; selecting one \
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
        body.push_str(&format!(
            "<h3 class=\"heading\">{} Found</h3>\
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
