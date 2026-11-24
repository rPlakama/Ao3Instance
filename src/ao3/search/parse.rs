//! Parsers for AO3 search result pages, work blurbs, and `#work-filters` facets.

use std::collections::BTreeMap;

use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::{
    collapse_text as text, extract_digits_after, parse_number, parse_selector as css,
};
use crate::error::{Error, Result};

use super::{LiveSearchResults, LiveSeries, LiveTagFacets, LiveWork, TagFacet};

impl LiveTagFacets {
    pub fn is_empty(&self) -> bool {
        self.ratings.is_empty()
            && self.warnings.is_empty()
            && self.categories.is_empty()
            && self.fandoms.is_empty()
            && self.characters.is_empty()
            && self.relationships.is_empty()
            && self.freeforms.is_empty()
    }

    /// Fallback popular-tag facets tallied from the returned works when AO3's
    /// `#work-filters` sidebar wasn't present (e.g., `/works/search` fallback).
    pub fn from_works(works: &[LiveWork]) -> Self {
        Self {
            ratings: tally_facets(works, |w| w.rating.iter().map(String::as_str)),
            warnings: tally_facets(works, |w| w.warnings.iter().map(String::as_str)),
            categories: tally_facets(works, |w| w.categories.iter().map(String::as_str)),
            fandoms: tally_facets(works, |w| w.fandoms.iter().map(String::as_str)),
            characters: tally_facets(works, |w| w.characters.iter().map(String::as_str)),
            relationships: tally_facets(works, |w| w.relationships.iter().map(String::as_str)),
            freeforms: tally_facets(works, |w| w.additional_tags.iter().map(String::as_str)),
        }
    }
}

fn tally_facets<'a, I>(works: &'a [LiveWork], extract: impl Fn(&'a LiveWork) -> I) -> Vec<TagFacet>
where
    I: Iterator<Item = &'a str>,
{
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for work in works {
        for raw in extract(work) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                *counts.entry(trimmed.to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut items: Vec<TagFacet> = counts
        .into_iter()
        .map(|(name, count)| TagFacet {
            name,
            count: Some(count),
        })
        .collect();
    items.sort_by(|a, b| {
        b.count
            .unwrap_or(0)
            .cmp(&a.count.unwrap_or(0))
            .then_with(|| a.name.cmp(&b.name))
    });
    items.truncate(10);
    items
}

/// Parses an AO3 `/works/search` result page.
pub fn parse_search_results(html: &str) -> Result<LiveSearchResults> {
    let document = Html::parse_document(html);
    let heading = css("h4.heading")?;
    let summary = document
        .select(&heading)
        .next()
        .map(|node| text(&node))
        .map(|text| {
            text.trim()
                .strip_prefix("You searched for:")
                .unwrap_or_else(|| text.trim())
                .trim()
                .to_string()
        })
        .unwrap_or_default();

    let work_selector = css("ol.work.index.group > li.work.blurb.group")?;
    let works: Vec<LiveWork> = document
        .select(&work_selector)
        .filter_map(|element| parse_work_element(&element).ok())
        .collect();

    let total = document
        .select(&css("h3.heading")?)
        .next()
        .and_then(|node| parse_number(&text(&node).replace("Found", "")))
        .or_else(|| {
            let selector = css("h2.heading").ok()?;
            document
                .select(&selector)
                .next()
                .and_then(|node| parse_total_from_heading(&text(&node)))
        })
        .map(|value| value as usize)
        .unwrap_or(works.len());

    if works.is_empty()
        && !document
            .root_element()
            .text()
            .collect::<String>()
            .contains("No results found")
        && total > 0
    {
        return Err(Error::UpstreamChanged(
            "search results had entries but none parsed".into(),
        ));
    }

    let facets = parse_tag_facets(&document);

    Ok(LiveSearchResults {
        summary,
        total,
        works,
        facets,
    })
}

fn parse_tag_facets(document: &Html) -> LiveTagFacets {
    LiveTagFacets {
        ratings: parse_facet_section(document, "rating"),
        warnings: parse_facet_section(document, "archive_warning"),
        categories: parse_facet_section(document, "category"),
        fandoms: parse_facet_section(document, "fandom"),
        characters: parse_facet_section(document, "character"),
        relationships: parse_facet_section(document, "relationship"),
        freeforms: parse_facet_section(document, "freeform"),
    }
}

fn parse_facet_section(document: &Html, tag_type: &str) -> Vec<TagFacet> {
    let mut out = parse_facet_dd(document, &format!("dd#include_{tag_type}_tags ul li label"));
    if out.is_empty() {
        out = parse_facet_dd(document, &format!("dd#exclude_{tag_type}_tags ul li label"));
    }
    out
}

fn parse_facet_dd(document: &Html, selector_str: &str) -> Vec<TagFacet> {
    let Ok(selector) = Selector::parse(selector_str) else {
        return Vec::new();
    };
    let span_selector = Selector::parse("span").ok();
    let mut out = Vec::new();
    for label in document.select(&selector) {
        let raw = span_selector
            .as_ref()
            .and_then(|sel| label.select(sel).last())
            .map(|span| text(&span))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| text(&label));
        if let Some(facet) = parse_facet_label(&raw) {
            out.push(facet);
        }
    }
    out
}

/// Parses an AO3 `#work-filters` label like `"Emilia (Re:Zero) (1,234)"` into
/// `TagFacet { name: "Emilia (Re:Zero)", count: Some(1234) }`.
fn parse_facet_label(raw: &str) -> Option<TagFacet> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(without_close) = trimmed.strip_suffix(')')
        && let Some((head, tail)) = without_close.rsplit_once('(')
    {
        let inner = tail.trim();
        let all_count_chars = !inner.is_empty()
            && inner
                .chars()
                .all(|ch| ch.is_ascii_digit() || ch == ',' || ch == ' ')
            && inner.chars().any(|ch| ch.is_ascii_digit());
        if all_count_chars {
            let name = head.trim();
            if !name.is_empty() {
                return Some(TagFacet {
                    name: name.to_string(),
                    count: parse_number(inner),
                });
            }
        }
    }
    Some(TagFacet {
        name: trimmed.to_string(),
        count: None,
    })
}

pub(crate) fn parse_work_element(element: &ElementRef<'_>) -> Result<LiveWork> {
    let id_attr = element.value().attr("id").unwrap_or_default();
    let ao3_id = id_attr
        .strip_prefix("work_")
        .map(str::to_string)
        .or_else(|| {
            let href = element
                .select(&css("h4.heading a").ok()?)
                .next()?
                .value()
                .attr("href")?;
            extract_digits_after(href, "/works/").map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::UpstreamChanged("search result without a work id".into()))?;

    let title = element
        .select(&css("h4.heading a")?)
        .next()
        .map(|node| text(&node))
        .unwrap_or_default();
    let restricted = element.select(&css("h4.heading img")?).any(|node| {
        node.value()
            .attr("alt")
            .is_some_and(|alt| alt.contains("Restricted"))
    });
    let authors: Vec<String> = element
        .select(&css("h4.heading a[rel=author]")?)
        .map(|node| text(&node))
        .filter(|value| !value.is_empty())
        .collect();
    let fandoms = element
        .select(&css("h5.fandoms a.tag")?)
        .map(|node| text(&node))
        .collect::<Vec<_>>();
    let rating = element
        .select(&css("ul.required-tags span.rating")?)
        .next()
        .map(|node| node.value().attr("title").unwrap_or_default().to_string())
        .filter(|value| !value.is_empty());
    let warning_tags: Vec<String> = element
        .select(&css("ul.tags.commas li.warnings a.tag")?)
        .map(|node| text(&node))
        .filter(|value| !value.is_empty())
        .collect();
    let warnings = if warning_tags.is_empty() {
        element
            .select(&css("ul.required-tags span.warnings")?)
            .next()
            .map(|node| vec![node.value().attr("title").unwrap_or_default().to_string()])
            .unwrap_or_default()
    } else {
        warning_tags
    };
    let categories = element
        .select(&css("ul.required-tags span.category")?)
        .next()
        .map(|node| vec![node.value().attr("title").unwrap_or_default().to_string()])
        .unwrap_or_default();
    let complete = element
        .select(&css("ul.required-tags span.iswip")?)
        .next()
        .map(|node| node.value().classes().any(|class| class == "complete-yes"));

    let group = |selector: &str| -> Result<Vec<String>> {
        Ok(element
            .select(&css(selector)?)
            .map(|node| text(&node))
            .filter(|value| !value.is_empty())
            .collect())
    };
    let relationships = group("ul.tags.commas li.relationships a.tag")?;
    let characters = group("ul.tags.commas li.characters a.tag")?;
    let additional_tags = group("ul.tags.commas li.freeforms a.tag")?;

    let datetime = element
        .select(&css("p.datetime")?)
        .next()
        .map(|node| text(&node))
        .unwrap_or_default();
    let summary_html = element
        .select(&css("blockquote.userstuff.summary")?)
        .next()
        .map(|node| node.inner_html())
        .filter(|html| !html.trim().is_empty());

    let series_strong = css("strong")?;
    let series_link = css("a")?;
    let series = element
        .select(&css("ul.series li")?)
        .map(|node| {
            let part = node
                .select(&series_strong)
                .next()
                .map(|strong| text(&strong))
                .filter(|value| !value.is_empty());
            let link = node.select(&series_link).next();
            let title = link
                .as_ref()
                .map(|link| text(link))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| text(&node));
            let href = link.and_then(|link| link.value().attr("href").map(str::to_string));
            LiveSeries { part, title, href }
        })
        .collect();

    let stat_text = |selector: &str| -> Option<String> {
        css(selector)
            .ok()
            .and_then(|selector| element.select(&selector).next())
            .map(|node| text(&node))
            .filter(|value| !value.is_empty())
    };
    let stat_number = |selector: &str| -> Option<u64> {
        stat_text(selector).and_then(|value| parse_number(&value))
    };

    Ok(LiveWork {
        ao3_id,
        title,
        restricted,
        authors,
        fandoms,
        rating,
        warnings,
        categories,
        complete,
        relationships,
        characters,
        additional_tags,
        datetime,
        summary_html,
        language: stat_text("dl.stats dd.language"),
        words: stat_number("dl.stats dd.words"),
        chapters: stat_text("dl.stats dd.chapters")
            .map(|value| value.replace(" /", "/").replace("/ ", "/")),
        comments: stat_number("dl.stats dd.comments"),
        kudos: stat_number("dl.stats dd.kudos"),
        bookmarks: stat_number("dl.stats dd.bookmarks"),
        hits: stat_number("dl.stats dd.hits"),
        collections: stat_number("dl.stats dd.collections"),
        series,
    })
}

/// Tag and bookmark pages say `1 - 20 of 9,432 Works in Tag`,
/// `1 - 20 of 42 Bookmarks by user`, or `1 Work in Tag`.
pub(crate) fn parse_total_from_heading(text: &str) -> Option<u64> {
    if let Some((_, tail)) = text.split_once(" of ") {
        return tail.split_whitespace().next().and_then(parse_number);
    }
    let mut words = text.split_whitespace();
    let count = words.next().and_then(parse_number)?;
    if words.any(|word| {
        word.eq_ignore_ascii_case("work")
            || word.eq_ignore_ascii_case("works")
            || word.eq_ignore_ascii_case("bookmark")
            || word.eq_ignore_ascii_case("bookmarks")
    }) {
        Some(count)
    } else {
        None
    }
}
