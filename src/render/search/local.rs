//! Offline search form and results over the local SQLite work cache.

use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::render::blurb::work_blurb;
use crate::render::chrome::{PageContext, layout_with_class};
use crate::render::components::select_options;
use crate::render::escape;
use crate::store::{BrowseResult, SearchFilter, SearchPage};

use super::{number_input, row, text_input};

/// Renders the AO3-style Work Search form over the local cache.
pub fn search_form_page(
    ctx: &PageContext,
    query: &str,
    facets: &BrowseResult,
    filter: &SearchFilter,
) -> String {
    let mut body = String::from("<h2 class=\"heading\">Work Search</h2>");
    body.push_str(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">Work Search</span></li>\
         <li><a href=\"/works?author=\">People Search</a></li>\
         <li><a href=\"/bookmarks\">Bookmark Search</a></li>\
         <li><a href=\"/fandoms\">Tag Search</a></li></ul>",
    );

    body.push_str("<form class=\"verbose search work-search\" method=\"get\" action=\"/search\">");
    body.push_str(
        "<fieldset><legend>Work Info</legend><h3 class=\"landmark heading\">Work Info</h3>\
         <p class=\"submit actions\"><button type=\"submit\">Search</button></p><dl>",
    );
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
        &facet_select(
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
        &facet_select(
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
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset><legend>Work Stats</legend><h3 class=\"landmark heading\">Work Stats</h3><dl>",
    );
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
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset><legend>Search</legend><h3 class=\"landmark heading\">Search</h3><dl>",
    );
    body.push_str(&row("Sort by", &sort_select(filter.sort.as_deref())));
    body.push_str(&row(
        "Sort direction",
        &direction_select(filter.direction.as_deref()),
    ));
    body.push_str("</dl><p class=\"submit actions\"><button type=\"submit\">Search</button></p>");
    body.push_str("</fieldset></form>");

    body.push_str(
        "<p class=\"meta\">Local Cache — this search only covers works already cached on \
         this instance and never calls archiveofourown.org.</p>",
    );

    layout_with_class(ctx, "works-search", &body)
}

pub fn search_results_page(
    ctx: &PageContext,
    query: &str,
    result: &SearchPage,
    filter: &SearchFilter,
) -> String {
    let mut body = String::from("<h2 class=\"heading\">Search Results</h2>");
    body.push_str(&format!(
        "<h4 class=\"heading\">You searched for: {}</h4>",
        summary(query, filter)
    ));
    body.push_str(&format!(
        "<ul class=\"navigation actions\" role=\"navigation\"><li>\
         <a href=\"/works/search?{}&edit_search=true\">Edit Your Search</a></li></ul>",
        filter_query(query, filter)
    ));

    if result.total == 0 {
        body.push_str(
            "<p>No results found. You may want to edit your search to make it less \
             specific.</p>",
        );
    } else {
        let pager = crate::render::pagination(
            result.page,
            result.per_page,
            result.total,
            "/works/search",
            &filter_query(query, filter),
        );
        body.push_str(&format!(
            "<h3 class=\"heading\">{} Found</h3>",
            crate::time::format_count(u64::try_from(result.total).unwrap_or(0))
        ));
        body.push_str(&pager);
        body.push_str("<h3 class=\"landmark heading\">Works List</h3>");
        body.push_str("<ol class=\"work index group\">");
        for work in &result.works {
            body.push_str(&work_blurb(work, None));
        }
        body.push_str("</ol>");
        body.push_str(&pager);
    }
    layout_with_class(ctx, "works-search", &body)
}

/// AO3-shaped "You searched for:" summary.
fn summary(query: &str, filter: &SearchFilter) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !query.trim().is_empty() {
        parts.push(escape(query.trim()));
    }
    for (label, value) in [
        ("Title:", filter.title.as_deref()),
        ("Creator:", filter.creator.as_deref()),
    ] {
        if let Some(value) = value {
            parts.push(format!("{label} {}", escape(value)));
        }
    }
    let mut tags: Vec<String> = Vec::new();
    for value in [
        filter.rating.as_deref(),
        filter.warning.as_deref(),
        filter.category.as_deref(),
        filter.fandom.as_deref(),
        filter.characters.as_deref(),
        filter.relationships.as_deref(),
        filter.additional_tags.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        tags.push(escape(value));
    }
    if !tags.is_empty() {
        parts.push(format!("Tags: {}", tags.join(", ")));
    }
    if let Some(language) = &filter.language {
        parts.push(format!("Language: {}", escape(language)));
    }
    if filter.words_min.is_some() || filter.words_max.is_some() {
        parts.push(format!(
            "words: {}-{}",
            filter
                .words_min
                .map_or_else(String::new, |value| value.to_string()),
            filter
                .words_max
                .map_or_else(String::new, |value| value.to_string())
        ));
    }
    if filter.kudos_min.is_some() {
        parts.push(format!(
            "kudos: >= {}",
            filter.kudos_min.unwrap_or_default()
        ));
    }
    if filter.comments_min.is_some() {
        parts.push(format!(
            "comments: >= {}",
            filter.comments_min.unwrap_or_default()
        ));
    }
    if filter.single_chapter {
        parts.push("Single Chapter".to_string());
    }
    let sort = match filter.sort.as_deref().unwrap_or("best_match") {
        "words" => "word count",
        "kudos" => "kudos",
        "title" => "title",
        "date" => "date",
        _ => "best match",
    };
    let direction = if filter.direction.as_deref() == Some("asc") {
        "ascending"
    } else {
        "descending"
    };
    parts.push(format!("sort by: {sort} {direction}"));
    parts.join(" ")
}

fn facet_select(
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
    const OPTIONS: &[(&str, &str)] = &[
        ("best_match", "Best Match"),
        ("words", "Word Count"),
        ("kudos", "Kudos"),
        ("title", "Title"),
        ("date", "Date Cached"),
    ];
    format!(
        "<select name=\"sort\">{}</select>",
        select_options(OPTIONS, current.unwrap_or("best_match"))
    )
}

fn direction_select(current: Option<&str>) -> String {
    const OPTIONS: &[(&str, &str)] = &[("desc", "Descending"), ("asc", "Ascending")];
    format!(
        "<select name=\"direction\">{}</select>",
        select_options(OPTIONS, current.unwrap_or("desc"))
    )
}

fn filter_query(query: &str, filter: &SearchFilter) -> String {
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
