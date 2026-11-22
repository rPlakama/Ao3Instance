//! Offline search results over the local cache, with AO3-shaped filters.

use super::blurb::work_blurb;
use super::chrome::{PageContext, layout};
use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BrowseResult, SearchFilter, SearchPage};

/// Renders the AO3-style Work Search form and, once any criterion is set, the
/// matching cached works.
pub fn search_page(
    ctx: &PageContext,
    query: &str,
    result: &SearchPage,
    facets: &BrowseResult,
    filter: &SearchFilter,
) -> String {
    let submitted = !query.trim().is_empty() || filter.has_filters();
    let mut body = String::from("<h2>Work Search</h2>");
    body.push_str(
        "<ul class=\"search-tabs actions\">\
         <li class=\"active\"><a href=\"/search\">Work Search</a></li>\
         <li><a href=\"/works?author=\">People Search</a></li>\
         <li><a href=\"/bookmarks\">Bookmark Search</a></li>\
         <li><a href=\"/fandoms\">Tag Search</a></li></ul>",
    );

    body.push_str("<form class=\"filters work-search\" method=\"get\" action=\"/search\">");
    body.push_str("<fieldset><legend>Work Info</legend>");
    body.push_str("<div class=\"search-actions\"><button type=\"submit\">Search</button></div>");
    body.push_str(&row(
        "Any Field",
        &format!(
            "<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"words in title, author, tags\">",
            escape(query)
        ),
    ));
    body.push_str(&row(
        "Title",
        &text_input("title", filter.title.as_deref(), "words in the title"),
    ));
    body.push_str(&row(
        "Creator",
        &text_input("creator", filter.creator.as_deref(), "author or pseud"),
    ));
    body.push_str(&row(
        "Rating",
        &select(
            "rating",
            "Any rating",
            &facets.ratings,
            filter.rating.as_deref(),
        ),
    ));
    body.push_str(&row(
        "Archive Warning",
        &text_input(
            "warning",
            filter.warning.as_deref(),
            "e.g. Major Character Death",
        ),
    ));
    body.push_str(&row(
        "Category",
        &text_input("category", filter.category.as_deref(), "e.g. F/M"),
    ));
    body.push_str(&row(
        "Fandom",
        &text_input("fandom", filter.fandom.as_deref(), "fandom name"),
    ));
    body.push_str(&row(
        "Characters",
        &text_input("characters", filter.characters.as_deref(), "character name"),
    ));
    body.push_str(&row(
        "Relationships",
        &text_input(
            "relationships",
            filter.relationships.as_deref(),
            "relationship name",
        ),
    ));
    body.push_str(&row(
        "Additional Tags",
        &text_input(
            "additional_tags",
            filter.additional_tags.as_deref(),
            "freeform tag",
        ),
    ));
    body.push_str(&row(
        "Language",
        &select(
            "language",
            "Any language",
            &facets.languages,
            filter.language.as_deref(),
        ),
    ));
    body.push_str(&row(
        "Word Count",
        &format!(
            "<span class=\"range\">from {}</span><span class=\"range\">to {}</span>",
            number_input(
                "words_min",
                filter.words_min.map(|value| value.to_string()).as_deref()
            ),
            number_input(
                "words_max",
                filter.words_max.map(|value| value.to_string()).as_deref()
            )
        ),
    ));
    body.push_str(&row(
        "Single Chapter",
        &format!(
            "<label class=\"check\"><input type=\"checkbox\" name=\"single_chapter\" value=\"1\"{}> Only single-chapter works</label>",
            if filter.single_chapter { " checked" } else { "" }
        ),
    ));
    body.push_str("</fieldset>");

    body.push_str("<fieldset><legend>Work Stats</legend>");
    body.push_str(&row(
        "Kudos",
        &format!(
            "at least {}",
            number_input(
                "kudos_min",
                filter.kudos_min.map(|value| value.to_string()).as_deref()
            )
        ),
    ));
    body.push_str(&row(
        "Comments",
        &format!(
            "at least {}",
            number_input(
                "comments_min",
                filter
                    .comments_min
                    .map(|value| value.to_string())
                    .as_deref()
            )
        ),
    ));
    body.push_str("</fieldset>");

    body.push_str("<fieldset><legend>Search</legend>");
    body.push_str(&row("Sort by", &sort_select(filter.sort.as_deref())));
    body.push_str(&row(
        "Sort direction",
        &direction_select(filter.direction.as_deref()),
    ));
    body.push_str("<div class=\"search-actions\"><button type=\"submit\">Search</button></div>");
    body.push_str("</fieldset></form>");

    body.push_str(
        "<p class=\"meta\">Local Cache — this search only covers works already cached on \
         this instance and never calls archiveofourown.org.</p>",
    );

    if submitted {
        if result.total == 0 {
            body.push_str(&format!(
                "<p class=\"muted\">0 cached matches{}. Open the work from the start page \
                 first; offline search only covers what has been cached.</p>",
                if query.trim().is_empty() {
                    " for these filters".to_string()
                } else {
                    format!(" for “{}”", escape(query))
                }
            ));
        } else {
            body.push_str(&format!("<h3>{} cached match(es)</h3>", result.total));
            for work in &result.works {
                body.push_str(&work_blurb(work, None));
            }
            body.push_str(&pagination(query, filter, result));
        }
    }
    layout(ctx, &body)
}

pub fn row(label: &str, control: &str) -> String {
    format!("<div class=\"row\"><label>{label}</label><div>{control}</div></div>")
}

fn text_input(name: &str, value: Option<&str>, placeholder: &str) -> String {
    format!(
        "<input type=\"text\" name=\"{name}\" value=\"{}\" placeholder=\"{}\">",
        escape(value.unwrap_or("")),
        escape(placeholder)
    )
}

fn number_input(name: &str, value: Option<&str>) -> String {
    format!(
        "<input type=\"number\" min=\"0\" name=\"{name}\" value=\"{}\" class=\"number\">",
        escape(value.unwrap_or(""))
    )
}

fn select(
    name: &str,
    empty_label: &str,
    options: &[(String, usize)],
    current: Option<&str>,
) -> String {
    let mut out = format!("<select name=\"{name}\"><option value=\"\">{empty_label}</option>");
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

fn sort_select(current: Option<&str>) -> String {
    let options = [
        ("best_match", "Best Match"),
        ("words", "Word Count"),
        ("kudos", "Kudos"),
        ("title", "Title"),
        ("date", "Date Cached"),
    ];
    let mut out = String::from("<select name=\"sort\">");
    for (value, label) in options {
        let selected = if current.unwrap_or("best_match") == value {
            " selected"
        } else {
            ""
        };
        out.push_str(&format!(
            "<option value=\"{value}\"{selected}>{label}</option>"
        ));
    }
    out.push_str("</select>");
    out
}

fn direction_select(current: Option<&str>) -> String {
    let current = current.unwrap_or("desc");
    format!(
        "<select name=\"direction\">\
         <option value=\"desc\"{}>Descending</option>\
         <option value=\"asc\"{}>Ascending</option></select>",
        if current == "desc" { " selected" } else { "" },
        if current == "asc" { " selected" } else { "" },
    )
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
    let pairs = [
        ("title", &filter.title),
        ("creator", &filter.creator),
        ("fandom", &filter.fandom),
        ("characters", &filter.characters),
        ("relationships", &filter.relationships),
        ("additional_tags", &filter.additional_tags),
        ("rating", &filter.rating),
        ("warning", &filter.warning),
        ("category", &filter.category),
        ("language", &filter.language),
        ("sort", &filter.sort),
        ("direction", &filter.direction),
    ];
    for (name, value) in pairs {
        if let Some(value) = value {
            parts.push(format!("{name}={}", encode(value)));
        }
    }
    for (name, value) in [
        ("words_min", filter.words_min),
        ("words_max", filter.words_max),
        ("kudos_min", filter.kudos_min),
        ("comments_min", filter.comments_min),
    ] {
        if let Some(value) = value {
            parts.push(format!("{name}={value}"));
        }
    }
    if filter.single_chapter {
        parts.push("single_chapter=1".to_string());
    }
    parts.push(format!("page={}", page.max(1)));
    parts.join("&")
}

/// Build a search filter from raw query parameters, dropping blanks.
#[allow(
    clippy::too_many_arguments,
    reason = "each argument is one independent AO3 search field"
)]
pub fn filter_from(
    title: Option<String>,
    creator: Option<String>,
    fandom: Option<String>,
    characters: Option<String>,
    relationships: Option<String>,
    additional_tags: Option<String>,
    rating: Option<String>,
    warning: Option<String>,
    category: Option<String>,
    language: Option<String>,
    words_min: Option<String>,
    words_max: Option<String>,
    kudos_min: Option<String>,
    comments_min: Option<String>,
    single_chapter: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
) -> SearchFilter {
    let clean = |value: Option<String>| {
        value.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        })
    };
    let number = |value: Option<String>| value.and_then(|value| value.trim().parse::<u64>().ok());
    SearchFilter {
        title: clean(title),
        creator: clean(creator),
        fandom: clean(fandom),
        characters: clean(characters),
        relationships: clean(relationships),
        additional_tags: clean(additional_tags),
        rating: clean(rating),
        warning: clean(warning),
        category: clean(category),
        language: clean(language),
        words_min: number(words_min),
        words_max: number(words_max),
        kudos_min: number(kudos_min),
        comments_min: number(comments_min),
        single_chapter: single_chapter.is_some_and(|value| value == "1" || value == "on"),
        sort: clean(sort).filter(|value| value != "best_match"),
        direction: clean(direction),
    }
}
