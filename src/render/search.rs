//! Offline search results over the local cache, with filters and paging.

use super::blurb::work_blurb;
use super::chrome::{PageContext, layout};
use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BrowseResult, SearchFilter, SearchPage};

pub fn search_page(
    ctx: &PageContext,
    query: &str,
    result: &SearchPage,
    facets: &BrowseResult,
    filter: &SearchFilter,
) -> String {
    let mut body = format!(
        "<h2>Search</h2><form class=\"filters\" method=\"get\" action=\"/search\">\
         <label>Any field<input type=\"search\" name=\"q\" value=\"{}\" \
         placeholder=\"title, author or tag\"></label>\
         <label>Rating{}</label><label>Fandom{}</label>\
         <label>Language{}</label><label>Sort by{}</label>\
         <button type=\"submit\">Search</button></form>\
         <p class=\"meta\">Local Cache — offline search. This never calls \
         archiveofourown.org.</p>",
        escape(query),
        select(
            "rating",
            "",
            "Any rating",
            &facets.ratings,
            filter.rating.as_deref()
        ),
        select(
            "fandom",
            "",
            "Any fandom",
            &facets.fandoms,
            filter.fandom.as_deref()
        ),
        select(
            "language",
            "",
            "Any language",
            &facets.languages,
            filter.language.as_deref()
        ),
        sort_select(filter.sort.as_deref()),
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
        body.push_str(&pagination(query, filter, result));
    }
    layout(ctx, &body)
}

fn sort_select(current: Option<&str>) -> String {
    let options = [
        ("", "Relevance"),
        ("words_desc", "Word count"),
        ("kudos_desc", "Kudos"),
        ("title_asc", "Title A–Z"),
    ];
    let mut out = String::from("<select name=\"sort\">");
    for (value, label) in options {
        let selected = if current.unwrap_or("") == value {
            " selected"
        } else {
            ""
        };
        out.push_str(&format!(
            "<option value=\"{}\"{selected}>{}</option>",
            escape(value),
            escape(label)
        ));
    }
    out.push_str("</select>");
    out
}

fn select(
    name: &str,
    empty_value: &str,
    empty_label: &str,
    options: &[(String, usize)],
    current: Option<&str>,
) -> String {
    let mut out =
        format!("<select name=\"{name}\"><option value=\"{empty_value}\">{empty_label}</option>");
    for (value, count) in options {
        let selected = if current == Some(value.as_str()) {
            " selected"
        } else {
            ""
        };
        out.push_str(&format!(
            "<option value=\"{}\"{selected}>{} ({count})</option>",
            escape(value),
            escape(value)
        ));
    }
    out.push_str("</select>");
    out
}

fn pagination(query: &str, filter: &SearchFilter, result: &SearchPage) -> String {
    let pages = (result.total + result.per_page - 1) / result.per_page;
    if pages <= 1 {
        return String::new();
    }
    let mut out = String::from("<ol class=\"pagination actions\">");
    if result.page > 1 {
        out.push_str(&format!(
            "<li><a href=\"/search?{}\">← previous</a></li>",
            query_string(query, filter, result.page - 1)
        ));
    }
    out.push_str(&format!(
        "<li><span class=\"state\">page {}/{pages}</span></li>",
        result.page
    ));
    if result.page < pages {
        out.push_str(&format!(
            "<li><a href=\"/search?{}\">next →</a></li>",
            query_string(query, filter, result.page + 1)
        ));
    }
    out.push_str("</ol>");
    out
}

fn query_string(query: &str, filter: &SearchFilter, page: i64) -> String {
    let encode = |value: &str| percent_encode(value, SpaceMode::Percent20);
    let mut parts = vec![format!("q={}", encode(query))];
    if let Some(rating) = &filter.rating {
        parts.push(format!("rating={}", encode(rating)));
    }
    if let Some(fandom) = &filter.fandom {
        parts.push(format!("fandom={}", encode(fandom)));
    }
    if let Some(language) = &filter.language {
        parts.push(format!("language={}", encode(language)));
    }
    if let Some(sort) = &filter.sort {
        parts.push(format!("sort={}", encode(sort)));
    }
    parts.push(format!("page={}", page.max(1)));
    parts.join("&")
}

/// Build a search filter from raw query parameters, dropping blanks.
pub fn filter_from(
    rating: Option<String>,
    fandom: Option<String>,
    language: Option<String>,
    sort: Option<String>,
) -> SearchFilter {
    let clean = |value: Option<String>| {
        value.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        })
    };
    SearchFilter {
        rating: clean(rating),
        fandom: clean(fandom),
        language: clean(language),
        sort: clean(sort),
    }
}
