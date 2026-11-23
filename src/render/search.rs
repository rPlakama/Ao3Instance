//! Offline search results over the local cache, with AO3-shaped filters.

use super::blurb::work_blurb;
use super::chrome::{NavSection, PageContext, layout};
use super::{escape, sanitize};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BrowseResult, SearchFilter, SearchPage};

/// Renders the AO3-style Work Search form and, once any criterion is set, the
/// matching cached works.
pub fn search_form_page(
    ctx: &PageContext,
    query: &str,
    facets: &BrowseResult,
    filter: &SearchFilter,
) -> String {
    let mut body = String::from("<h2 class=\"heading\">Work Search</h2>");
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

    layout(ctx, &body)
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
        body.push_str(&format!(
            "<h3 class=\"heading\">{} Found</h3>",
            crate::time::format_count(u64::try_from(result.total).unwrap_or(0))
        ));
        body.push_str("<ol class=\"work index group\">");
        for work in &result.works {
            body.push_str(&work_blurb(work, None));
        }
        body.push_str("</ol>");
        body.push_str(&crate::render::pagination(
            result.page,
            result.per_page,
            result.total,
            "/works/search",
            &filter_query(query, filter),
        ));
    }
    layout(ctx, &body)
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

// ---------------------------------------------------------------------------
// Live AO3 search (the default path)
// ---------------------------------------------------------------------------

use crate::ao3::search::{LiveSearchQuery, LiveSearchResults, LiveWork};

/// AO3 Work Search form. It submits to our server, which performs one literal
/// AO3 search request (AO3's own web form uses GET, so this does too).
pub fn live_search_form_page(ctx: &PageContext, query: &LiveSearchQuery) -> String {
    let mut body = String::from("<h2 class=\"heading\">Work Search</h2>");
    body.push_str(
        "<ul class=\"search-tabs actions\">\
         <li class=\"active\"><a href=\"/works/search\">Work Search</a></li>\
         <li><a href=\"/works?author=\">People Search</a></li>\
         <li><a href=\"/bookmarks\">Bookmark Search</a></li>\
         <li><a href=\"/tags/search\">Tag Search</a></li></ul>",
    );
    body.push_str(
        "<form class=\"work-search\" method=\"get\" action=\"/works/search\">\
         <fieldset><legend>Work Info</legend>\
         <div class=\"search-actions\"><button type=\"submit\">Search</button></div>",
    );
    body.push_str(&row(
        "Any Field",
        &format!(
            "<input type=\"search\" name=\"q\" value=\"{}\" placeholder=\"words in title, author, tags\">",
            escape(&query.query)
        ),
    ));
    body.push_str(&row(
        "Title",
        &text_input("title", query.title.as_deref(), "words in the title"),
    ));
    body.push_str(&row(
        "Creator",
        &text_input("creator", query.creator.as_deref(), "author or pseud"),
    ));
    body.push_str(&row(
        "Date",
        &text_input("date", query.date.as_deref(), "YYYY-MM-DD"),
    ));
    body.push_str(&row(
        "Completion status",
        &radio_group(
            "complete",
            &[
                ("", "All works"),
                ("T", "Complete works only"),
                ("F", "Works in progress only"),
            ],
            query.complete.as_deref().unwrap_or(""),
        ),
    ));
    body.push_str(&row(
        "Crossovers",
        &radio_group(
            "crossover",
            &[
                ("", "Include crossovers"),
                ("F", "Exclude crossovers"),
                ("T", "Only crossovers"),
            ],
            query.crossover.as_deref().unwrap_or(""),
        ),
    ));
    body.push_str(&row(
        "Single Chapter",
        &format!(
            "<label class=\"check\"><input type=\"checkbox\" name=\"single_chapter\" value=\"1\"{}> Only single-chapter works</label>",
            if query.single_chapter { " checked" } else { "" }
        ),
    ));
    body.push_str(&row(
        "Word Count",
        &format!(
            "<span class=\"range\">from {}</span><span class=\"range\">to {}</span>",
            number_input(
                "words_min",
                query
                    .word_count
                    .as_deref()
                    .and_then(|value| value.split_once('-').map(|(min, _)| min))
            ),
            number_input(
                "words_max",
                query
                    .word_count
                    .as_deref()
                    .and_then(|value| value.split_once('-').map(|(_, max)| max))
            )
        ),
    ));
    body.push_str(&row(
        "Language",
        &live_select(
            "language",
            &[
                ("", "Any language"),
                ("en", "English"),
                ("es", "Español"),
                ("fr", "Français"),
                ("de", "Deutsch"),
                ("it", "Italiano"),
                ("nl", "Nederlands"),
                ("ptBR", "Português brasileiro"),
                ("ru", "Русский"),
                ("zh", "中文"),
                ("ja", "日本語"),
            ],
            query.language.as_deref().unwrap_or(""),
        ),
    ));
    body.push_str("</fieldset>");

    body.push_str("<fieldset><legend>Work Tags</legend>");
    body.push_str(&row(
        "Fandoms",
        &text_input("fandom", query.fandom.as_deref(), "fandom name"),
    ));
    body.push_str(&row(
        "Rating",
        &live_select(
            "rating",
            &[
                ("", "Any rating"),
                ("General Audiences", "General Audiences"),
                ("Teen And Up Audiences", "Teen And Up Audiences"),
                ("Mature", "Mature"),
                ("Explicit", "Explicit"),
                ("Not Rated", "Not Rated"),
            ],
            query.rating.as_deref().unwrap_or(""),
        ),
    ));
    body.push_str(&row(
        "Warnings",
        &text_input(
            "warning",
            query.warning.as_deref(),
            "e.g. Major Character Death",
        ),
    ));
    body.push_str(&row(
        "Categories",
        &text_input("category", query.category.as_deref(), "e.g. F/M"),
    ));
    body.push_str(&row(
        "Characters",
        &text_input("characters", query.characters.as_deref(), "character name"),
    ));
    body.push_str(&row(
        "Relationships",
        &text_input(
            "relationships",
            query.relationships.as_deref(),
            "relationship name",
        ),
    ));
    body.push_str(&row(
        "Additional Tags",
        &text_input(
            "additional_tags",
            query.additional_tags.as_deref(),
            "freeform tag",
        ),
    ));
    body.push_str(&row(
        "Other tags to include",
        &text_input(
            "include_tags",
            Some(&query.include_tags.join(", ")),
            "comma-separated tag names",
        ),
    ));
    body.push_str(&row(
        "Other tags to exclude",
        &text_input(
            "exclude_tags",
            Some(&query.exclude_tags.join(", ")),
            "comma-separated tag names",
        ),
    ));
    body.push_str("</fieldset>");

    body.push_str("<fieldset><legend>Work Stats</legend>");
    for (label, name, value) in [
        ("Hits", "hits_min", query.hits_min),
        ("Kudos", "kudos_min", query.kudos_min),
        ("Comments", "comments_min", query.comments_min),
        ("Bookmarks", "bookmarks_min", query.bookmarks_min),
    ] {
        body.push_str(&row(
            label,
            &format!(
                "at least {}",
                number_input(name, value.map(|value| value.to_string()).as_deref())
            ),
        ));
    }
    body.push_str("</fieldset>");

    body.push_str("<fieldset><legend>Search</legend>");
    body.push_str(&row(
        "Sort by",
        &live_select(
            "sort",
            &[
                ("best_match", "Best Match"),
                ("date", "Date Updated"),
                ("words", "Word Count"),
                ("kudos", "Kudos"),
                ("title", "Title"),
            ],
            query.sort.as_deref().unwrap_or("best_match"),
        ),
    ));
    body.push_str(&row(
        "Sort direction",
        &live_select(
            "direction",
            &[("desc", "Descending"), ("asc", "Ascending")],
            query.direction.as_deref().unwrap_or("desc"),
        ),
    ));
    body.push_str("<div class=\"search-actions\"><button type=\"submit\">Search</button></div>");
    body.push_str("</fieldset></form>");
    body.push_str(
        "<p class=\"meta\">This submits one search to archiveofourown.org; results are not \
         cached until you open them. <a href=\"/works\">Browse the local cache instead</a>.</p>",
    );
    layout(ctx, &body)
}

/// AO3-style live search result page.
pub fn live_search_results_page(
    ctx: &PageContext,
    query: &LiveSearchQuery,
    results: &LiveSearchResults,
) -> String {
    let summary = if results.summary.is_empty() {
        query.query.clone()
    } else {
        results.summary.clone()
    };
    let mut body = String::from("<h2 class=\"heading\">Search Results</h2>");
    body.push_str(&format!(
        "<h4 class=\"heading\">You searched for: {}</h4>",
        escape(&summary)
    ));
    body.push_str(&format!(
        "<ul class=\"navigation actions\" role=\"navigation\"><li>\
         <a href=\"/works/search?{}&edit_search=true\">Edit Your Search</a></li></ul>",
        query.form_query()
    ));
    if results.total == 0 {
        body.push_str(
            "<p>No results found. You may want to edit your search to make it less \
             specific.</p>",
        );
    } else {
        body.push_str(&format!(
            "<h3 class=\"heading\">{} Found</h3>",
            crate::time::format_count(u64::try_from(results.total).unwrap_or(0))
        ));
        body.push_str("<ol class=\"work index group\">");
        for work in &results.works {
            body.push_str(&live_blurb(work));
        }
        body.push_str("</ol>");
        body.push_str(&crate::render::pagination(
            query_page(query),
            20,
            i64::try_from(results.total).unwrap_or(i64::MAX),
            "/works/search",
            &query.form_query(),
        ));
        if ctx.account.is_empty() {
            body.push_str(
                "<p class=\"meta\">Signed out: this is AO3's signed-out result set. \
                 <a href=\"/session#cookie-import\">Import your AO3 session</a> to see the \
                 results your account sees (and to use cookies with the solver).</p>",
            );
        }
        body.push_str(
            "<p class=\"meta\">Live results from archiveofourown.org. Opening a work here \
             caches it for offline reading.</p>",
        );
    }
    layout(ctx, &body)
}

fn query_page(query: &LiveSearchQuery) -> i64 {
    query.page.max(1)
}

fn live_blurb(work: &LiveWork) -> String {
    let authors = if work.authors.is_empty() {
        "Anonymous".to_string()
    } else {
        work.authors
            .iter()
            .map(|author| {
                format!(
                    "<a href=\"/works/search?creator={}\">{}</a>",
                    percent_encode(author, SpaceMode::Percent20),
                    escape(author)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let fandoms = work
        .fandoms
        .iter()
        .map(|fandom| {
            format!(
                "<a class=\"tag\" href=\"/works/search?fandom={}\">{}</a>",
                percent_encode(fandom, SpaceMode::Percent20),
                escape(fandom)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let tag_links = |tags: &[String], class: &str, parameter: &str| -> String {
        tags.iter()
            .map(|tag| {
                format!(
                    "<li class=\"{class}\"><a class=\"tag\" href=\"/works/search?{parameter}={}\">{}</a></li>",
                    percent_encode(tag, SpaceMode::Percent20),
                    escape(tag)
                )
            })
            .collect()
    };
    let tags = format!(
        "{}{}{}{}",
        tag_links(&work.warnings, "warnings", "include_tags"),
        tag_links(&work.relationships, "relationships", "include_tags"),
        tag_links(&work.characters, "characters", "include_tags"),
        tag_links(&work.additional_tags, "freeforms", "include_tags"),
    );
    let summary = work
        .summary_html
        .as_deref()
        .map_or_else(String::new, |html| {
            format!(
                "<h6 class=\"landmark heading\">Summary</h6>\
                 <blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(html)
            )
        });
    let series = if work.series.is_empty() {
        String::new()
    } else {
        let mut items = String::new();
        for series in &work.series {
            let part = series.part.as_deref().map_or_else(String::new, |part| {
                format!("Part <strong>{}</strong> of ", escape(part))
            });
            let title = series.href.as_deref().map_or_else(
                || escape(&series.title),
                |href| {
                    format!(
                        "<a href=\"https://archiveofourown.org{}\" rel=\"noopener noreferrer\">{}</a>",
                        escape(href),
                        escape(&series.title)
                    )
                },
            );
            items.push_str(&format!("<li>{part}{title}</li>"));
        }
        format!("<h6 class=\"landmark heading\">Series</h6><ul class=\"series\">{items}</ul>")
    };
    let restricted = if work.restricted {
        " <span class=\"restricted\" title=\"Restricted\">(Restricted)</span>"
    } else {
        ""
    };
    let rating = required_badge(
        rating_class(work.rating.as_deref()),
        rating_short(work.rating.as_deref()),
        work.rating.as_deref().unwrap_or("Not rated"),
    );
    let warnings = required_badge(
        if work
            .warnings
            .iter()
            .any(|warning| warning.contains("No Archive Warnings"))
        {
            "warning-no"
        } else {
            "warning-yes"
        },
        if work.warnings.is_empty() { "–" } else { "!" },
        work.warnings
            .first()
            .map_or("Archive warnings", String::as_str),
    );
    let category_label = category_short(work.categories.first().map_or("?", String::as_str));
    let category = required_badge(
        "category",
        &category_label,
        work.categories
            .first()
            .map_or("No category", String::as_str),
    );
    let status = match work.complete {
        Some(true) => required_badge("status-complete", "✓", "Complete"),
        Some(false) => required_badge("status-wip", "WIP", "Work in progress"),
        None => required_badge("status-unknown", "?", "Status unknown"),
    };
    let mut stats = String::new();
    if let Some(language) = &work.language {
        stats.push_str(&format!(
            "<dt class=\"language\">Language:</dt><dd class=\"language\">{}</dd>",
            escape(language)
        ));
    }
    if let Some(words) = work.words {
        stats.push_str(&format!(
            "<dt class=\"words\">Words:</dt><dd class=\"words\">{}</dd>",
            crate::time::format_count(words)
        ));
    }
    if let Some(chapters) = &work.chapters {
        stats.push_str(&format!(
            "<dt class=\"chapters\">Chapters:</dt><dd class=\"chapters\">{}</dd>",
            escape(chapters)
        ));
    }
    for (label, value) in [
        ("Collections", work.collections),
        ("Comments", work.comments),
        ("Kudos", work.kudos),
        ("Bookmarks", work.bookmarks),
        ("Hits", work.hits),
    ] {
        if let Some(value) = value.filter(|value| *value > 0) {
            stats.push_str(&format!(
                "<dt>{label}:</dt><dd>{}</dd>",
                crate::time::format_count(value)
            ));
        }
    }
    format!(
        "<li class=\"work blurb group\" role=\"article\"><div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/works/{id}\">{title}</a> by {authors}{restricted}</h4>\
         <h5 class=\"fandoms heading\"><span class=\"landmark\">Fandoms:</span> {fandoms}</h5>\
         <ul class=\"required-tags\">{rating}{warnings}{category}{status}</ul>\
         <p class=\"datetime\">{datetime}</p></div>\
         <h6 class=\"landmark heading\">Tags</h6><ul class=\"tags commas\">{tags}</ul>{summary}{series}\
         <dl class=\"stats\">{stats}</dl>\
         <p class=\"actions\"><a href=\"https://archiveofourown.org/works/{id}\" \
         rel=\"noopener noreferrer\">Open on AO3</a> \
         <form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{id}\">\
         <button type=\"submit\">Cache this work</button></form></p></li>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        datetime = escape(&work.datetime),
        restricted = restricted,
        series = series,
    )
}

fn required_badge(class: &str, label: &str, title: &str) -> String {
    format!(
        "<li class=\"{class}\" title=\"{}\">{}</li>",
        escape(title),
        escape(label)
    )
}

fn rating_class(rating: Option<&str>) -> &'static str {
    match rating.unwrap_or_default().to_ascii_lowercase().as_str() {
        value if value.contains("general") => "rating-g",
        value if value.contains("teen") => "rating-t",
        value if value.contains("mature") => "rating-m",
        value if value.contains("explicit") => "rating-e",
        _ => "rating-nr",
    }
}

fn category_short(category: &str) -> String {
    match category.to_ascii_lowercase().as_str() {
        "f/m" => "F/M".to_string(),
        "m/m" => "M/M".to_string(),
        "f/f" => "F/F".to_string(),
        "gen" => "Gen".to_string(),
        "multi" => "Multi".to_string(),
        "other" => "Other".to_string(),
        "no category" | "?" => "—".to_string(),
        other => other.chars().take(4).collect(),
    }
}

fn rating_short(rating: Option<&str>) -> &'static str {
    match rating_class(rating) {
        "rating-g" => "G",
        "rating-t" => "T",
        "rating-m" => "M",
        "rating-e" => "E",
        _ => "NR",
    }
}

fn radio_group(name: &str, options: &[(&str, &str)], current: &str) -> String {
    options
        .iter()
        .map(|(value, label)| {
            format!(
                "<label class=\"check\"><input type=\"radio\" name=\"{name}\" value=\"{}\"{}> {label}</label>",
                escape(value),
                if current == *value { " checked" } else { "" }
            )
        })
        .collect()
}

fn live_select(name: &str, options: &[(&str, &str)], current: &str) -> String {
    let mut out = format!("<select name=\"{name}\">");
    for (value, label) in options {
        out.push_str(&format!(
            "<option value=\"{}\"{}>{label}</option>",
            escape(value),
            if current == *value { " selected" } else { "" }
        ));
    }
    out.push_str("</select>");
    out
}

/// Edge block on a live search: one explicit solve-and-retry, never automatic.
pub fn search_challenge_page(
    status: u16,
    target: &str,
    retry: &str,
    solver_available: bool,
    send_cookies_allowed: bool,
) -> String {
    let mut body = format!(
        "<h2>Search blocked</h2><div class=\"error\">AO3 answered HTTP {status}.          Search is one literal AO3 request; it has not been retried automatically.</div>"
    );
    if solver_available {
        body.push_str(
            "<div class=\"continue\"><strong>A local FlareSolverr is available.</strong>              It loads this exact result page once in a real browser. Only press this if you              want to spend that solve.</div>",
        );
        body.push_str(&format!(
            "<form class=\"stack\" method=\"post\" action=\"/solve\">             <input type=\"hidden\" name=\"target\" value=\"{}\">             <input type=\"hidden\" name=\"next\" value=\"{}\">",
            escape(target),
            escape(retry)
        ));
        if send_cookies_allowed {
            body.push_str(
                "<label><input type=\"checkbox\" name=\"send_cookies\" value=\"1\">                  Send my AO3 session cookies to the local solver (needed for restricted or                  adult results)</label>",
            );
        }
        body.push_str("<button type=\"submit\">Solve &amp; show this search</button></form>");
    } else {
        body.push_str(
            "<p class=\"meta\">The solver is off or unreachable. A host-provided              FlareSolverr can help; check <a href=\"/diagnostics\">Diagnostics</a>.</p>",
        );
    }
    body.push_str(
        "<p><a href=\"/session#cookie-import\">Import fresh session cookies</a> ·          <a href=\"/diagnostics\">Diagnostics</a> · <a href=\"/works\">Browse the cache</a></p>",
    );
    layout(
        &PageContext::new("AO3Instance — search blocked", NavSection::Search),
        &body,
    )
}
