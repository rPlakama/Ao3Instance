//! Cached-fandoms directory modelled on `otwarchive/app/views/fandoms/index.html.erb`,
//! with an A–Z jump bar and AO3 `.letter.listbox.group` panels.

use std::collections::BTreeMap;

use super::chrome::{PageContext, layout_with_class};
use super::escape;

pub fn fandoms_page(ctx: &PageContext, fandoms: &[(String, usize)]) -> String {
    let mut body = String::from("<h2 class=\"heading\">Cached fandoms</h2>");
    body.push_str(
        "<p class=\"note\">Only fandoms present in the local cache are listed; \
         this page never calls archiveofourown.org. You can search this page by pressing \
         <kbd>ctrl F</kbd> / <kbd>cmd F</kbd>.</p>",
    );
    if fandoms.is_empty() {
        body.push_str(
            "<p class=\"muted\">No fandom yet. Open a work from the start page to populate \
             the cache.</p>",
        );
        return layout_with_class(ctx, "fandoms-index", &body);
    }

    let mut groups: BTreeMap<char, Vec<&(String, usize)>> = BTreeMap::new();
    for fandom in fandoms {
        let letter = fandom
            .0
            .chars()
            .next()
            .map(|ch| ch.to_ascii_uppercase())
            .filter(|ch| ch.is_ascii_alphabetic())
            .unwrap_or('#');
        groups.entry(letter).or_default().push(fandom);
    }

    body.push_str(
        "<h3 class=\"landmark heading\">Alphabet Navigation</h3>\
         <ul class=\"alphabet actions\" id=\"alphabet\" role=\"navigation\">",
    );
    for letter in groups.keys() {
        body.push_str(&format!(
            "<li><a href=\"#letter-{letter}\">{letter}</a></li>"
        ));
    }
    body.push_str("</ul>");

    body.push_str(
        "<h3 class=\"landmark heading\">Alphabetised List of Fandoms</h3>\
         <ol class=\"alphabet fandom index group\">",
    );
    for (letter, entries) in &groups {
        body.push_str(&format!(
            "<li id=\"letter-{letter}\" class=\"letter listbox group\">\
             <h3 class=\"heading\">{letter}\
             <span class=\"action top\" title=\"top\"><a href=\"#alphabet\">↑</a></span></h3>\
             <ul class=\"tags index group\">"
        ));
        for (fandom, count) in entries {
            body.push_str(&format!(
                "<li><a class=\"tag\" href=\"/works?fandom={}\">{}</a> ({count})</li>",
                escape(&crate::ao3::parse::percent_encode(
                    fandom,
                    crate::ao3::parse::SpaceMode::Percent20
                )),
                escape(fandom),
            ));
        }
        body.push_str("</ul></li>");
    }
    body.push_str("</ol>");
    layout_with_class(ctx, "fandoms-index", &body)
}
