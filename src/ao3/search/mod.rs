//! Live AO3 Work Search: build the query AO3's own form submits and parse the
//! rendered result page into typed entries.
//!
//! This is the one deliberately online search path: it is a single,
//! user-initiated request (ADR-0002 §3), never a paging crawl. Results are not
//! written to the cache; opening a result is a separate explicit action.

mod parse;
mod query;
#[cfg(test)]
mod tests;

pub use parse::parse_search_results;
pub(crate) use parse::{parse_total_from_heading, parse_work_element};
pub use query::ao3_tag_param;

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

/// One popular tag facet from AO3's `#work-filters` sidebar (or tallied from
/// the returned works when `#work-filters` is absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFacet {
    pub name: String,
    pub count: Option<u64>,
}

/// Popular tag facets rendered in the `#work-filters` sidebar on tag pages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveTagFacets {
    pub ratings: Vec<TagFacet>,
    pub warnings: Vec<TagFacet>,
    pub categories: Vec<TagFacet>,
    pub fandoms: Vec<TagFacet>,
    pub characters: Vec<TagFacet>,
    pub relationships: Vec<TagFacet>,
    pub freeforms: Vec<TagFacet>,
}

/// Parsed AO3 search page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSearchResults {
    /// AO3's "You searched for:" text, already plain text.
    pub summary: String,
    /// Total matches AO3 reports (may exceed the 20 shown).
    pub total: usize,
    pub works: Vec<LiveWork>,
    /// Popular tag facets parsed from AO3's `#work-filters` sidebar, if present.
    pub facets: LiveTagFacets,
}

/// One AO3 series entry attached to a work blurb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSeries {
    pub part: Option<String>,
    pub title: String,
    pub href: Option<String>,
}

/// One AO3 work result entry.
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
