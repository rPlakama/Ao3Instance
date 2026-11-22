//! Cached-fandoms directory with an A–Z jump bar.

use std::collections::BTreeMap;

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
        return layout(ctx, &body);
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

    body.push_str("<p class=\"actions\">");
    for letter in groups.keys() {
        body.push_str(&format!(
            "<a class=\"state\" href=\"#fandom-{letter}\">{letter}</a> "
        ));
    }
    body.push_str("</p>");

    for (letter, entries) in &groups {
        body.push_str(&format!("<h3 id=\"fandom-{letter}\">{letter}</h3>"));
        body.push_str("<ul class=\"tags commas\">");
        for (fandom, count) in entries {
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
