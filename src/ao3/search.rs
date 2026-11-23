//! Live AO3 Work Search: build the query AO3's own form submits and parse the
//! rendered result page into typed entries.
//!
//! This is the one deliberately online search path: it is a single,
//! user-initiated request (ADR-0002 §3), never a paging crawl. Results are not
//! written to the cache; opening a result is a separate explicit action.

use scraper::{ElementRef, Html, Selector};

use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::error::{Error, Result};

/// What AO3's Work Search form sends. Flat local field names are translated
/// into AO3's `work_search[...]` parameters by [`Self::path`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSearchQuery {
    pub query: String,
    pub title: Option<String>,
    pub creator: Option<String>,
    pub date: Option<String>,
    /// `T` complete, `F` in progress, `None` all.
    pub complete: Option<String>,
    /// `F` exclude crossovers, `T` only crossovers, `None` include.
    pub crossover: Option<String>,
    pub single_chapter: bool,
    pub word_count: Option<String>,
    /// AO3 language short code (`en`, `es`, ...).
    pub language: Option<String>,
    pub fandom: Option<String>,
    pub characters: Option<String>,
    pub relationships: Option<String>,
    pub additional_tags: Option<String>,
    /// Free-text tags appended to `other_tag_names`.
    pub include_tags: Vec<String>,
    /// Free-text tags sent as `excluded_tag_names`.
    pub exclude_tags: Vec<String>,
    pub rating: Option<String>,
    pub warning: Option<String>,
    pub category: Option<String>,
    pub hits_min: Option<u64>,
    pub kudos_min: Option<u64>,
    pub comments_min: Option<u64>,
    pub bookmarks_min: Option<u64>,
    /// `best_match`, `date`, `words`, `kudos`, `title`.
    pub sort: Option<String>,
    /// `asc` or `desc`.
    pub direction: Option<String>,
    /// 1-based page requested from AO3.
    pub page: i64,
}

impl LiveSearchQuery {
    pub fn has_criteria(&self) -> bool {
        !self.query.trim().is_empty()
            || self.title.is_some()
            || self.creator.is_some()
            || self.fandom.is_some()
            || self.characters.is_some()
            || self.relationships.is_some()
            || self.additional_tags.is_some()
            || self.rating.is_some()
            || self.warning.is_some()
            || self.category.is_some()
            || !self.include_tags.is_empty()
            || !self.exclude_tags.is_empty()
            || self.language.is_some()
            || self.word_count.is_some()
            || self.hits_min.is_some()
            || self.kudos_min.is_some()
            || self.comments_min.is_some()
            || self.bookmarks_min.is_some()
            || self.complete.is_some()
            || self.crossover.is_some()
            || self.single_chapter
            || self.date.is_some()
    }

    /// AO3's `/works/search` query string for one page.
    pub fn path(&self, page: i64) -> String {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let push = |pairs: &mut Vec<(String, String)>, name: &str, value: &str| {
            if !value.trim().is_empty() {
                pairs.push((format!("work_search[{name}]"), value.trim().to_string()));
            }
        };
        push(&mut pairs, "query", &self.query);
        for (name, value) in [
            ("title", &self.title),
            ("creators", &self.creator),
            ("revised_at", &self.date),
            ("fandom_names", &self.fandom),
            ("character_names", &self.characters),
            ("relationship_names", &self.relationships),
            ("freeform_names", &self.additional_tags),
        ] {
            if let Some(value) = value {
                push(&mut pairs, name, value);
            }
        }
        if let Some(complete) = &self.complete {
            push(&mut pairs, "complete", complete);
        }
        if let Some(crossover) = &self.crossover {
            push(&mut pairs, "crossover", crossover);
        }
        if self.single_chapter {
            push(&mut pairs, "single_chapter", "1");
        }
        if let Some(words) = &self.word_count {
            push(&mut pairs, "word_count", words);
        }
        if let Some(language) = &self.language {
            push(&mut pairs, "language_id", language);
        }
        for (name, value) in [
            ("hits", self.hits_min),
            ("kudos_count", self.kudos_min),
            ("comments_count", self.comments_min),
            ("bookmarks_count", self.bookmarks_min),
        ] {
            if let Some(value) = value {
                push(&mut pairs, name, &format!(">{value}"));
            }
        }

        // Rating/warning/category names are canonical tags; AO3's free-text
        // "other tags" filter matches tag names across types.
        let mut tags = self.include_tags.clone();
        for value in [&self.rating, &self.warning, &self.category]
            .into_iter()
            .flatten()
        {
            tags.push(value.clone());
        }
        if !tags.is_empty() {
            push(&mut pairs, "other_tag_names", &tags.join(", "));
        }
        if !self.exclude_tags.is_empty() {
            push(
                &mut pairs,
                "excluded_tag_names",
                &self.exclude_tags.join(", "),
            );
        }

        let (sort, direction) = sort_columns(self.sort.as_deref(), self.direction.as_deref());
        // AO3's own default is best match descending; sending it explicitly
        // changes Elasticsearch's tie order, so omit it just like AO3 does.
        if !(sort == "_score" && direction == "desc") {
            push(&mut pairs, "sort_column", sort);
            push(&mut pairs, "sort_direction", direction);
        }
        let page = if page > 1 { page } else { self.page.max(1) };
        if page > 1 {
            pairs.push(("page".to_string(), page.to_string()));
        }
        let encoded = pairs
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    percent_encode(key, SpaceMode::Percent20),
                    percent_encode(value, SpaceMode::Percent20)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        format!("/works/search?{encoded}")
    }

    /// Query string for the form/results links (without AO3's bracket syntax).
    pub fn form_query(&self) -> String {
        let encode = |value: &str| percent_encode(value, SpaceMode::Percent20);
        let mut parts: Vec<String> = Vec::new();
        let mut push = |name: &str, value: &str| {
            if !value.trim().is_empty() {
                parts.push(format!("{name}={}", encode(value.trim())));
            }
        };
        push("q", &self.query);
        for (name, value) in [
            ("title", &self.title),
            ("creator", &self.creator),
            ("date", &self.date),
            ("fandom", &self.fandom),
            ("characters", &self.characters),
            ("relationships", &self.relationships),
            ("additional_tags", &self.additional_tags),
            ("rating", &self.rating),
            ("warning", &self.warning),
            ("category", &self.category),
            ("language", &self.language),
            ("words", &self.word_count),
            ("complete", &self.complete),
            ("crossover", &self.crossover),
            ("sort", &self.sort),
            ("direction", &self.direction),
        ] {
            if let Some(value) = value {
                push(name, value);
            }
        }
        if self.single_chapter {
            parts.push("single_chapter=1".to_string());
        }
        for (name, value) in [
            ("hits_min", self.hits_min),
            ("kudos_min", self.kudos_min),
            ("comments_min", self.comments_min),
            ("bookmarks_min", self.bookmarks_min),
        ] {
            if let Some(value) = value {
                parts.push(format!("{name}={value}"));
            }
        }
        if !self.include_tags.is_empty() {
            parts.push(format!(
                "include_tags={}",
                encode(&self.include_tags.join(","))
            ));
        }
        if !self.exclude_tags.is_empty() {
            parts.push(format!(
                "exclude_tags={}",
                encode(&self.exclude_tags.join(","))
            ));
        }
        parts.join("&")
    }
}

fn sort_columns(sort: Option<&str>, direction: Option<&str>) -> (&'static str, &'static str) {
    let column = match sort.unwrap_or("best_match") {
        "date" => "revised_at",
        "created_at" => "created_at",
        "words" => "word_count",
        "kudos" => "kudos_count",
        "hits" => "hits",
        "comments" => "comments_count",
        "bookmarks" => "bookmarks_count",
        "title" => "title_to_sort_on",
        _ => "_score",
    };
    let direction = match direction {
        Some("asc") => "asc",
        Some("desc") => "desc",
        _ if column == "title_to_sort_on" => "asc",
        _ => "desc",
    };
    (column, direction)
}

/// Parsed AO3 search page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSearchResults {
    /// AO3's "You searched for:" text, already plain text.
    pub summary: String,
    /// Total matches AO3 reports (may exceed the 20 shown).
    pub total: usize,
    pub works: Vec<LiveWork>,
}

/// One AO3 result entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSeries {
    pub part: Option<String>,
    pub title: String,
    pub href: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveWork {
    pub ao3_id: String,
    pub title: String,
    pub restricted: bool,
    pub authors: Vec<String>,
    pub fandoms: Vec<String>,
    pub rating: Option<String>,
    pub warnings: Vec<String>,
    pub categories: Vec<String>,
    pub complete: Option<bool>,
    pub relationships: Vec<String>,
    pub characters: Vec<String>,
    pub additional_tags: Vec<String>,
    pub datetime: String,
    pub summary_html: Option<String>,
    pub language: Option<String>,
    pub words: Option<u64>,
    pub chapters: Option<String>,
    pub comments: Option<u64>,
    pub kudos: Option<u64>,
    pub bookmarks: Option<u64>,
    pub hits: Option<u64>,
    pub collections: Option<u64>,
    pub series: Vec<LiveSeries>,
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

    Ok(LiveSearchResults {
        summary,
        total,
        works,
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
            let start = href.find("/works/")? + "/works/".len();
            Some(
                href[start..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect(),
            )
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

fn css(selector: &str) -> Result<Selector> {
    Selector::parse(selector)
        .map_err(|_| Error::UpstreamChanged(format!("search selector is invalid: {selector}")))
}

fn text(element: &ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_number(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
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

#[cfg(test)]
mod tests {
    use super::*;

    const RESULTS: &str = include_str!("../../tests/fixtures/search_results.html");

    #[test]
    fn parses_ao3_search_results() {
        let results = parse_search_results(RESULTS).expect("parse");
        assert_eq!(results.total, 2);
        assert_eq!(results.summary, "mirror sort by: best match descending");
        assert_eq!(results.works.len(), 2);

        let first = &results.works[0];
        assert_eq!(first.ao3_id, "111");
        assert_eq!(first.title, "A Mirror of Small Things");
        assert!(first.restricted);
        assert_eq!(first.collections, Some(2));
        assert_eq!(first.series.len(), 1);
        assert_eq!(first.series[0].part.as_deref(), Some("2"));
        assert_eq!(first.series[0].title, "purge_king basement");
        assert_eq!(first.series[0].href.as_deref(), Some("/series/5383226"));
        assert_eq!(first.authors, ["tester"]);
        assert_eq!(first.fandoms, ["Test Fandom"]);
        assert_eq!(first.rating.as_deref(), Some("Explicit"));
        assert_eq!(first.warnings, ["Graphic Depictions Of Violence"]);
        assert_eq!(first.categories, ["F/M"]);
        assert_eq!(first.complete, Some(true));
        assert_eq!(first.relationships, ["A/B"]);
        assert_eq!(first.characters, ["Character A"]);
        assert_eq!(first.additional_tags, ["Fluff"]);
        assert_eq!(first.datetime, "02 Jan 2025");
        assert_eq!(first.words, Some(4_321));
        assert_eq!(first.chapters.as_deref(), Some("2/2"));
        assert_eq!(first.comments, Some(57));
        assert_eq!(first.kudos, Some(1_024));
        assert_eq!(first.bookmarks, Some(12));
        assert_eq!(first.hits, Some(9_876));
        assert!(
            first
                .summary_html
                .as_deref()
                .is_some_and(|html| html.contains("cartographer"))
        );

        let second = &results.works[1];
        assert_eq!(second.complete, Some(false));
        assert_eq!(second.chapters.as_deref(), Some("1/?"));
        assert_eq!(second.comments, None);
        assert!(!second.restricted);
        assert!(second.series.is_empty());
    }

    #[test]
    fn builds_ao3_query_parameters() {
        let query = LiveSearchQuery {
            query: "mirror".into(),
            creator: Some("tester".into()),
            fandom: Some("Test Fandom".into()),
            rating: Some("Explicit".into()),
            include_tags: vec!["Fluff".into(), "Angst".into()],
            exclude_tags: vec!["Major Character Death".into()],
            complete: Some("T".into()),
            single_chapter: true,
            sort: Some("kudos".into()),
            direction: Some("asc".into()),
            ..LiveSearchQuery::default()
        };
        let path = query.path(2);
        assert!(path.starts_with("/works/search?"), "{path}");
        assert!(path.contains("work_search%5Bquery%5D=mirror"), "{path}");
        assert!(path.contains("work_search%5Bcreators%5D=tester"), "{path}");
        assert!(
            path.contains("work_search%5Bfandom_names%5D=Test%20Fandom"),
            "{path}"
        );
        assert!(
            path.contains("work_search%5Bother_tag_names%5D=Fluff%2C%20Angst%2C%20Explicit"),
            "{path}"
        );
        assert!(
            path.contains("work_search%5Bexcluded_tag_names%5D=Major%20Character%20Death"),
            "{path}"
        );
        assert!(path.contains("work_search%5Bcomplete%5D=T"), "{path}");
        assert!(path.contains("work_search%5Bsingle_chapter%5D=1"), "{path}");
        assert!(
            path.contains("work_search%5Bsort_column%5D=kudos_count"),
            "{path}"
        );
        assert!(
            path.contains("work_search%5Bsort_direction%5D=asc"),
            "{path}"
        );
        assert!(path.contains("page=2"), "{path}");
        assert!(query.has_criteria());
    }

    #[test]
    fn empty_query_has_no_criteria() {
        assert!(!LiveSearchQuery::default().has_criteria());
    }

    #[test]
    fn tag_page_totals_parse_from_range_headings() {
        assert_eq!(
            parse_total_from_heading("1 - 20 of 9,432 Works in Re:Zero"),
            Some(9_432)
        );
        assert_eq!(parse_total_from_heading("1 Work in Fluff"), Some(1));
        assert_eq!(parse_total_from_heading("Cached works"), None);
    }

    #[test]
    fn default_best_match_omits_sort_and_commit() {
        let query = LiveSearchQuery {
            query: "re zero".into(),
            ..LiveSearchQuery::default()
        };
        let path = query.path(1);
        assert!(!path.contains("sort_column"), "{path}");
        assert!(!path.contains("commit"), "{path}");
        assert!(path.contains("work_search%5Bquery%5D=re%20zero"), "{path}");
    }
}
