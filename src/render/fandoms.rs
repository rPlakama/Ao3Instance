//! Fandoms directory: live AO3 `/media` and `/media/:slug/fandoms` (cached 30
//! min), with an offline cached-fandoms fallback (`?local=1`).

use std::collections::BTreeMap;

use super::chrome::{PageContext, layout_with_class};
use super::escape;
use crate::ao3::account_lists::{LiveMedium, LiveMediumFandoms};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::time::{format_count, format_utc};

/// Renders AO3's live (or 30-minute cached) `/media` directory.
pub fn live_media_page(
    ctx: &PageContext,
    media: &[LiveMedium],
    cache_meta: Option<(bool, i64)>,
) -> String {
    let mut body = String::from(
        "<h2 class=\"heading\">Fandoms</h2>\
         <ul class=\"navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">AO3 Fandoms</span></li>\
         <li><a href=\"/fandoms?local=1\">Cached Fandoms</a></li>\
         <li><a href=\"/fandoms?refresh=1\">Refresh from AO3</a></li></ul>",
    );
    body.push_str(&cache_note(cache_meta, "/fandoms?refresh=1"));
    if media.is_empty() {
        body.push_str("<p class=\"muted\">No media categories returned by AO3.</p>");
        return layout_with_class(ctx, "media-index", &body);
    }

    body.push_str("<ul class=\"media fandom index group\">");
    for medium in media {
        let slug_enc = percent_encode(&medium.slug, SpaceMode::Percent20).replace("%2A", "*");
        let medium_href = format!("/media/{slug_enc}/fandoms");
        body.push_str(&format!(
            "<li class=\"medium listbox group\">\
             <h3 class=\"heading\"><a href=\"{medium_href}\">{}</a></h3>\
             <ol class=\"index group\">",
            escape(&medium.name)
        ));
        for fandom in &medium.fandoms {
            let count = fandom
                .count
                .map_or_else(String::new, |n| format!(" ({})", format_count(n)));
            body.push_str(&format!(
                "<li><a class=\"tag\" href=\"{}\">{}</a>{count}</li>",
                super::tag_href(&fandom.name),
                escape(&fandom.name),
            ));
        }
        body.push_str(&format!(
            "</ol><p class=\"actions\" role=\"navigation\">\
             <a href=\"{medium_href}\">All {}</a></p></li>",
            escape(&medium.name)
        ));
    }
    body.push_str("</ul>");
    layout_with_class(ctx, "media-index", &body)
}

/// Renders AO3's full A–Z fandom index for one media category
/// (`/media/:slug/fandoms`).
pub fn live_medium_fandoms_page(
    ctx: &PageContext,
    medium_slug: &str,
    data: &LiveMediumFandoms,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let title = if data.title.is_empty() {
        medium_slug
    } else {
        data.title.as_str()
    };
    let slug_enc = percent_encode(medium_slug, SpaceMode::Percent20).replace("%2A", "*");
    let refresh_href = format!("/media/{slug_enc}/fandoms?refresh=1");
    let mut body = format!(
        "<h2 class=\"heading\"><a href=\"/fandoms\">Fandoms</a> &gt; {}</h2>\
         <p class=\"note\">You can search this page by pressing \
         <kbd>ctrl F</kbd> / <kbd>cmd F</kbd>.</p>",
        escape(title)
    );
    body.push_str(&cache_note(cache_meta, &refresh_href));
    if data.groups.is_empty() {
        body.push_str("<p class=\"muted\">No fandoms found for this category.</p>");
        return layout_with_class(ctx, "fandoms-index", &body);
    }

    body.push_str(
        "<h3 class=\"landmark heading\">Alphabet Navigation</h3>\
         <ul class=\"alphabet actions\" id=\"alphabet\" role=\"navigation\">",
    );
    for (letter, _) in &data.groups {
        body.push_str(&format!(
            "<li><a href=\"#letter-{l}\">{l}</a></li>",
            l = escape(letter)
        ));
    }
    body.push_str("</ul><ol class=\"alphabet fandom index group\">");
    for (letter, entries) in &data.groups {
        body.push_str(&format!(
            "<li id=\"letter-{l}\" class=\"letter listbox group\">\
             <h3 class=\"heading\">{l}\
             <span class=\"action top\" title=\"top\"><a href=\"#alphabet\">↑</a></span></h3>\
             <ul class=\"tags index group\">",
            l = escape(letter)
        ));
        for fandom in entries {
            let count = fandom
                .count
                .map_or_else(String::new, |n| format!(" ({})", format_count(n)));
            body.push_str(&format!(
                "<li><a class=\"tag\" href=\"{}\">{}</a>{count}</li>",
                super::tag_href(&fandom.name),
                escape(&fandom.name),
            ));
        }
        body.push_str("</ul></li>");
    }
    body.push_str("</ol>");
    layout_with_class(ctx, "fandoms-index", &body)
}

/// Offline / `?local=1` cached-fandoms view.
pub fn fandoms_page(ctx: &PageContext, fandoms: &[(String, usize)]) -> String {
    let mut body = String::from(
        "<h2 class=\"heading\">Cached fandoms</h2>\
         <ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/fandoms\">AO3 Fandoms</a></li>\
         <li class=\"active\"><span class=\"current\">Cached Fandoms</span></li></ul>\
         <p class=\"note\">Showing fandoms present in the local cache. \
         You can search this page by pressing <kbd>ctrl F</kbd> / <kbd>cmd F</kbd>.</p>",
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
                escape(&percent_encode(fandom, SpaceMode::Percent20)),
                escape(fandom),
            ));
        }
        body.push_str("</ul></li>");
    }
    body.push_str("</ol>");
    layout_with_class(ctx, "fandoms-index", &body)
}

fn cache_note(cache_meta: Option<(bool, i64)>, refresh_href: &str) -> String {
    let Some((from_cache, fetched_at)) = cache_meta else {
        return String::new();
    };
    let status = if from_cache {
        format!(
            "Cached from archiveofourown.org at {} (30-min cache)",
            escape(&format_utc(fetched_at))
        )
    } else {
        "Live from archiveofourown.org (cached for 30 min)".to_string()
    };
    format!("<p class=\"meta\">{status} · <a href=\"{refresh_href}\">Refresh now</a></p>")
}
