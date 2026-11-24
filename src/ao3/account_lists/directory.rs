//! Parsers for AO3 directory pages: `/tags/search`, `/media`, and `/media/:slug/fandoms`.

use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::{collapse_text, parse_number};
use crate::ao3::parse::percent_decode;

use super::{parse_max_page, trailing_paren_number};

/// One tag result from `GET /tags/search`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTagHit {
    pub kind: String,
    pub name: String,
    pub canonical: bool,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveTagSearchResults {
    pub total: usize,
    pub total_pages: i64,
    pub hits: Vec<LiveTagHit>,
}

/// One fandom link inside `/media` or `/media/:slug/fandoms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFandom {
    pub name: String,
    pub count: Option<u64>,
}

/// One media category box on `GET /media`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveMedium {
    pub name: String,
    pub slug: String,
    pub fandoms: Vec<LiveFandom>,
}

/// Full A–Z fandom index from `GET /media/:slug/fandoms`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveMediumFandoms {
    pub title: String,
    pub groups: Vec<(String, Vec<LiveFandom>)>,
}

/// Parses `GET /tags/search?tag_search[name]=...`.
pub fn parse_tag_search(html: &str) -> LiveTagSearchResults {
    let doc = Html::parse_document(html);
    let total_pages = parse_max_page(&doc);
    let Ok(li_sel) = Selector::parse("ol.tag.index.group > li") else {
        return LiveTagSearchResults::default();
    };
    let Ok(tag_sel) = Selector::parse("a.tag") else {
        return LiveTagSearchResults::default();
    };
    let Ok(canon_sel) = Selector::parse("span.canonical") else {
        return LiveTagSearchResults::default();
    };

    let mut hits = Vec::new();
    for li in doc.select(&li_sel) {
        let Some(link) = li.select(&tag_sel).next() else {
            continue;
        };
        let name = collapse_text(&link);
        if name.is_empty() {
            continue;
        }
        let canonical = li.select(&canon_sel).next().is_some();
        let full = collapse_text(&li);
        let kind = full
            .split_once(':')
            .map(|(prefix, _)| prefix.trim().to_string())
            .filter(|prefix| !prefix.is_empty() && prefix.len() <= 32)
            .unwrap_or_else(|| "Tag".to_string());
        let count = trailing_paren_number(&full).unwrap_or(0);
        hits.push(LiveTagHit {
            kind,
            name,
            canonical,
            count,
        });
    }

    let total = Selector::parse("h3.heading")
        .ok()
        .and_then(|sel| {
            doc.select(&sel)
                .map(|node| collapse_text(&node))
                .find(|heading| heading.contains("Found"))
        })
        .and_then(|heading| parse_number(&heading.replace("Found", "")))
        .map(|value| value as usize)
        .unwrap_or(hits.len());

    LiveTagSearchResults {
        total,
        total_pages,
        hits,
    }
}

/// Parses `GET /media` (categories and top fandoms).
pub fn parse_media_index(html: &str) -> Vec<LiveMedium> {
    let doc = Html::parse_document(html);
    let Ok(med_sel) = Selector::parse("ul.media.fandom.index.group > li.medium.listbox.group")
    else {
        return Vec::new();
    };
    let Ok(heading_sel) = Selector::parse("h3.heading a") else {
        return Vec::new();
    };
    let Ok(item_sel) = Selector::parse("ol.index.group > li") else {
        return Vec::new();
    };
    let Ok(tag_sel) = Selector::parse("a.tag") else {
        return Vec::new();
    };

    let mut media = Vec::new();
    for li in doc.select(&med_sel) {
        let Some(heading_link) = li.select(&heading_sel).next() else {
            continue;
        };
        let name = collapse_text(&heading_link);
        let href = heading_link.value().attr("href").unwrap_or_default();
        let slug = extract_media_slug(href).unwrap_or_else(|| name.clone());
        let fandoms = parse_fandom_items(&li, &item_sel, &tag_sel);
        if !name.is_empty() {
            media.push(LiveMedium {
                name,
                slug,
                fandoms,
            });
        }
    }
    media
}

/// Parses `GET /media/:slug/fandoms` (full A–Z list for one media category).
pub fn parse_medium_fandoms(html: &str) -> LiveMediumFandoms {
    let doc = Html::parse_document(html);
    let title = Selector::parse("h2.heading")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|node| {
            let full = collapse_text(&node);
            full.rsplit_once('>')
                .map_or(full.as_str(), |(_, tail)| tail)
                .trim()
                .to_string()
        })
        .unwrap_or_default();

    let Ok(letter_sel) =
        Selector::parse("ol.alphabet.fandom.index.group > li.letter.listbox.group")
    else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };
    let Ok(heading_sel) = Selector::parse("h3.heading") else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };
    let Ok(item_sel) = Selector::parse("ul.tags.index.group > li") else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };
    let Ok(tag_sel) = Selector::parse("a.tag") else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };

    let mut groups = Vec::new();
    for li in doc.select(&letter_sel) {
        let letter = li
            .value()
            .attr("id")
            .and_then(|id| id.strip_prefix("letter-"))
            .map(str::to_string)
            .or_else(|| {
                li.select(&heading_sel).next().and_then(|node| {
                    collapse_text(&node)
                        .split_whitespace()
                        .next()
                        .map(str::to_string)
                })
            })
            .unwrap_or_else(|| "#".to_string());
        let fandoms = parse_fandom_items(&li, &item_sel, &tag_sel);
        if !fandoms.is_empty() {
            groups.push((letter, fandoms));
        }
    }

    LiveMediumFandoms { title, groups }
}

fn parse_fandom_items(
    container: &ElementRef<'_>,
    item_sel: &Selector,
    tag_sel: &Selector,
) -> Vec<LiveFandom> {
    container
        .select(item_sel)
        .filter_map(|item| {
            let link = item.select(tag_sel).next()?;
            let name = collapse_text(&link);
            if name.is_empty() {
                return None;
            }
            let count = trailing_paren_number(&collapse_text(&item));
            Some(LiveFandom { name, count })
        })
        .collect()
}

fn extract_media_slug(href: &str) -> Option<String> {
    let rest = href.strip_prefix("/media/")?;
    let seg = rest.split('/').next()?;
    if seg.is_empty() {
        return None;
    }
    Some(percent_decode(seg).unwrap_or_else(|| seg.to_string()))
}
