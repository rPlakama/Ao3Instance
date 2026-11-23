//! Offline search results over the local cache, with AO3-shaped filters.

use super::blurb::work_blurb;
use super::chrome::{NavSection, PageContext, layout, layout_with_class};
use super::icons::required_tags_from;
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

pub fn row(label: &str, control: &str) -> String {
    format!("<dt><label>{label}</label></dt><dd>{control}</dd>")
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
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">Work Search</span></li>\
         <li><a href=\"/works?author=\">People Search</a></li>\
         <li><a href=\"/bookmarks\">Bookmark Search</a></li>\
         <li><a href=\"/tags/search\">Tag Search</a></li></ul>",
    );
    body.push_str(
        "<form class=\"verbose search work-search\" method=\"get\" action=\"/works/search\">\
         <fieldset><legend>Work Info</legend><h3 class=\"landmark heading\">Work Info</h3>\
         <p class=\"submit actions\"><button type=\"submit\">Search</button></p><dl>",
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
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset><legend>Work Tags</legend><h3 class=\"landmark heading\">Work Tags</h3><dl>",
    );
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
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset><legend>Work Stats</legend><h3 class=\"landmark heading\">Work Stats</h3><dl>",
    );
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
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset><legend>Search</legend><h3 class=\"landmark heading\">Search</h3><dl>",
    );
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
    body.push_str("</dl><p class=\"submit actions\"><button type=\"submit\">Search</button></p>");
    body.push_str("</fieldset></form>");
    body.push_str(
        "<p class=\"meta\">This submits one search to archiveofourown.org; results are not \
         cached until you open them. <a href=\"/works\">Browse the local cache instead</a>.</p>",
    );
    layout_with_class(ctx, "works-search", &body)
}

/// AO3-style live search result page.
pub fn live_search_results_page(
    ctx: &PageContext,
    query: &LiveSearchQuery,
    results: &LiveSearchResults,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let summary = if results.summary.is_empty() {
        query.query.clone()
    } else {
        results.summary.clone()
    };
    let form_query = query.form_query();
    let page_suffix = if query.page > 1 {
        format!("&page={}", query.page)
    } else {
        String::new()
    };
    let refresh_href = format!("/works/search?{form_query}{page_suffix}&refresh=1");
    let mut body = String::from("<h2 class=\"heading\">Search Results</h2>");
    body.push_str(&format!(
        "<h4 class=\"heading\">You searched for: {}</h4>",
        escape(&summary)
    ));
    body.push_str(&format!(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/works/search?{form_query}&edit_search=true\">Edit Your Search</a></li>\
         <li><a href=\"{refresh_href}\">Refresh results</a></li></ul>"
    ));
    if let Some((from_cache, fetched_at)) = cache_meta {
        let status = if from_cache {
            format!(
                "Cached from archiveofourown.org at {} (30-min cache)",
                escape(&crate::time::format_utc(fetched_at))
            )
        } else {
            "Live results from archiveofourown.org (cached for 30 min)".to_string()
        };
        body.push_str(&format!(
            "<p class=\"meta\">{status} · <a href=\"{refresh_href}\">Refresh now</a></p>"
        ));
    }
    if results.total == 0 {
        body.push_str(
            "<p>No results found. You may want to edit your search to make it less \
             specific.</p>",
        );
    } else {
        let pager = crate::render::pagination(
            query_page(query),
            20,
            i64::try_from(results.total).unwrap_or(i64::MAX),
            "/works/search",
            &query.form_query(),
        );
        body.push_str(&format!(
            "<h3 class=\"heading\">{} Found</h3>",
            crate::time::format_count(u64::try_from(results.total).unwrap_or(0))
        ));
        body.push_str(&pager);
        body.push_str("<h3 class=\"landmark heading\">Works List</h3>");
        body.push_str("<ol class=\"work index group\">");
        for work in &results.works {
            body.push_str(&live_blurb(work));
        }
        body.push_str("</ol>");
        body.push_str(&pager);
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
    layout_with_class(ctx, "works-search", &body)
}

/// Selected filters for a live tag page. Kept separate from the upstream query
/// so the sidebar can be re-rendered with the exact submitted values.
#[derive(Debug, Clone)]
pub struct TagFilter {
    pub sort: String,
    pub direction: String,
    pub query: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_fandoms: String,
    pub include_characters: String,
    pub include_relationships: String,
    pub include_additional: String,
    pub other_include: String,
    pub exclude_fandoms: String,
    pub exclude_characters: String,
    pub exclude_relationships: String,
    pub exclude_additional: String,
    pub other_exclude: String,
    pub complete: String,
    pub crossover: String,
    pub words_from: String,
    pub words_to: String,
    pub language: String,
}

impl Default for TagFilter {
    fn default() -> Self {
        Self {
            sort: "date".to_string(),
            direction: "desc".to_string(),
            query: String::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            include_fandoms: String::new(),
            include_characters: String::new(),
            include_relationships: String::new(),
            include_additional: String::new(),
            other_include: String::new(),
            exclude_fandoms: String::new(),
            exclude_characters: String::new(),
            exclude_relationships: String::new(),
            exclude_additional: String::new(),
            other_exclude: String::new(),
            complete: String::new(),
            crossover: String::new(),
            words_from: String::new(),
            words_to: String::new(),
            language: String::new(),
        }
    }
}

impl TagFilter {
    fn to_query(&self) -> String {
        let encode = |value: &str| percent_encode(value, SpaceMode::Percent20);
        let mut parts: Vec<String> = Vec::new();
        if !self.sort.is_empty() && self.sort != "date" {
            parts.push(format!("sort={}", encode(&self.sort)));
        }
        if !self.query.is_empty() {
            parts.push(format!("q={}", encode(&self.query)));
        }
        for (name, value) in [
            ("include_fandoms", &self.include_fandoms),
            ("include_characters", &self.include_characters),
            ("include_relationships", &self.include_relationships),
            ("include_additional", &self.include_additional),
            ("other_tags_include", &self.other_include),
            ("exclude_fandoms", &self.exclude_fandoms),
            ("exclude_characters", &self.exclude_characters),
            ("exclude_relationships", &self.exclude_relationships),
            ("exclude_additional", &self.exclude_additional),
            ("other_tags_exclude", &self.other_exclude),
            ("words_from", &self.words_from),
            ("words_to", &self.words_to),
            ("language", &self.language),
        ] {
            if !value.is_empty() {
                parts.push(format!("{name}={}", encode(value)));
            }
        }
        if !self.complete.is_empty() {
            parts.push(format!("complete={}", encode(&self.complete)));
        }
        if !self.crossover.is_empty() {
            parts.push(format!("crossover={}", encode(&self.crossover)));
        }
        for tag in &self.include {
            parts.push(format!("include_tag={}", encode(tag)));
        }
        for tag in &self.exclude {
            parts.push(format!("exclude_tag={}", encode(tag)));
        }
        parts.join("&")
    }
}

fn facet_group(action: &str, label: &str, values: &[&str], selected: &[String]) -> String {
    let mut out = format!(
        "<details class=\"filter-group {action}\"><summary>{label}</summary><ul class=\"facet-list\">"
    );
    for value in values {
        let checked = if selected.iter().any(|item| item == value) {
            " checked"
        } else {
            ""
        };
        out.push_str(&format!(
            "<li><label><input type=\"checkbox\" name=\"{action}_tag\" value=\"{value}\"{checked}>\
             <span class=\"indicator\" aria-hidden=\"true\"></span><span>{value}</span></label></li>",
            value = escape(value),
        ));
    }
    out.push_str("</ul></details>");
    out
}

fn text_group(name: &str, label: &str, value: &str, placeholder: &str) -> String {
    format!(
        "<details class=\"filter-group\"><summary>{label}</summary>\
         <input type=\"text\" name=\"{name}\" value=\"{}\" placeholder=\"{}\"></details>",
        escape(value),
        escape(placeholder)
    )
}

fn tag_filter_sidebar(tag: &str, filter: &TagFilter) -> String {
    let path = format!("/tags/{}/works", percent_encode(tag, SpaceMode::Percent20));
    let mut out = format!(
        "<form id=\"work-filters\" class=\"filters\" method=\"get\" action=\"{path}\">\
         <h3 class=\"landmark heading\">Filters</h3><fieldset>\
         <legend>Filter results:</legend>\
         <div class=\"filter-submit\"><button type=\"submit\">Sort and Filter</button></div>\
         <label class=\"filter-sort\">Sort by<select name=\"sort\">"
    );
    for (value, label) in [
        ("date", "Date Updated"),
        ("created_at", "Date Posted"),
        ("words", "Word Count"),
        ("kudos", "Kudos"),
        ("hits", "Hits"),
        ("comments", "Comments"),
        ("bookmarks", "Bookmarks"),
        ("best_match", "Best Match"),
    ] {
        out.push_str(&format!(
            "<option value=\"{value}\"{}>{label}</option>",
            if filter.sort == value {
                " selected"
            } else {
                ""
            }
        ));
    }
    out.push_str("</select></label>");

    for (action, label, include) in [("include", "Include", true), ("exclude", "Exclude", false)] {
        let selected = if include {
            &filter.include
        } else {
            &filter.exclude
        };
        out.push_str(&format!("<h4 class=\"filter-heading\">{label}</h4>"));
        out.push_str(&facet_group(
            action,
            "Ratings",
            &[
                "Not Rated",
                "General Audiences",
                "Teen And Up Audiences",
                "Mature",
                "Explicit",
            ],
            selected,
        ));
        out.push_str(&facet_group(
            action,
            "Archive Warnings",
            &[
                "Creator Chose Not To Use Archive Warnings",
                "No Archive Warnings Apply",
                "Graphic Depictions Of Violence",
                "Major Character Death",
                "Rape/Non-Con",
                "Underage Sex",
            ],
            selected,
        ));
        out.push_str(&facet_group(
            action,
            "Categories",
            &["F/F", "F/M", "Gen", "M/M", "Multi", "Other"],
            selected,
        ));
        let prefix = if include { "include" } else { "exclude" };
        let (fandoms, characters, relationships, additional, other) = if include {
            (
                &filter.include_fandoms,
                &filter.include_characters,
                &filter.include_relationships,
                &filter.include_additional,
                &filter.other_include,
            )
        } else {
            (
                &filter.exclude_fandoms,
                &filter.exclude_characters,
                &filter.exclude_relationships,
                &filter.exclude_additional,
                &filter.other_exclude,
            )
        };
        out.push_str(&text_group(
            &format!("{prefix}_fandoms"),
            "Fandoms",
            fandoms,
            "comma-separated fandom names",
        ));
        out.push_str(&text_group(
            &format!("{prefix}_characters"),
            "Characters",
            characters,
            "comma-separated character names",
        ));
        out.push_str(&text_group(
            &format!("{prefix}_relationships"),
            "Relationships",
            relationships,
            "comma-separated relationship names",
        ));
        out.push_str(&text_group(
            &format!("{prefix}_additional"),
            "Additional Tags",
            additional,
            "comma-separated freeform tags",
        ));
        out.push_str(&text_group(
            &format!("other_tags_{prefix}"),
            "Other tags",
            other,
            "comma-separated tag names",
        ));
    }

    out.push_str("<h4 class=\"filter-heading\">More Options</h4>");
    out.push_str(
        "<details class=\"filter-group\"><summary>Completion Status</summary><ul class=\"facet-list\">",
    );
    for (value, label) in [
        ("", "All works"),
        ("T", "Complete works only"),
        ("F", "Works in progress only"),
    ] {
        out.push_str(&format!(
            "<li><label><input type=\"radio\" name=\"complete\" value=\"{value}\"{}>\
             <span class=\"indicator\" aria-hidden=\"true\"></span><span>{label}</span></label></li>",
            if filter.complete == value { " checked" } else { "" }
        ));
    }
    out.push_str("</ul></details>");
    out.push_str(
        "<details class=\"filter-group\"><summary>Crossovers</summary><ul class=\"facet-list\">",
    );
    for (value, label) in [
        ("", "Include crossovers"),
        ("F", "Exclude crossovers"),
        ("T", "Only crossovers"),
    ] {
        out.push_str(&format!(
            "<li><label><input type=\"radio\" name=\"crossover\" value=\"{value}\"{}>\
             <span class=\"indicator\" aria-hidden=\"true\"></span><span>{label}</span></label></li>",
            if filter.crossover == value { " checked" } else { "" }
        ));
    }
    out.push_str("</ul></details>");
    out.push_str(&format!(
        "<details class=\"filter-group\" open><summary>Word Count</summary>\
         <div class=\"range\"><label>From <input type=\"number\" min=\"0\" name=\"words_from\" value=\"{}\"></label>\
         <label>To <input type=\"number\" min=\"0\" name=\"words_to\" value=\"{}\"></label></div></details>",
        escape(&filter.words_from),
        escape(&filter.words_to)
    ));
    out.push_str(&format!(
        "<label class=\"filter-query\">Search within results\
         <input type=\"search\" name=\"q\" value=\"{}\"></label>",
        escape(&filter.query)
    ));
    out.push_str(
        "<label class=\"filter-sort\">Language<select name=\"language\">\
         <option value=\"\">Any language</option>",
    );
    for (code, label) in [
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
    ] {
        out.push_str(&format!(
            "<option value=\"{code}\"{}>{label}</option>",
            if filter.language == code {
                " selected"
            } else {
                ""
            }
        ));
    }
    out.push_str("</select></label>");
    out.push_str(
        "<div class=\"filter-submit\"><button type=\"submit\">Sort and Filter</button></div>\
         <p class=\"footnote\"><a href=\"",
    );
    out.push_str(&path);
    out.push_str("\">Clear Filters</a></p></fieldset></form>");
    out
}

/// Live tag works page (`/tags/<tag>/works`).
pub fn live_tag_page(
    ctx: &PageContext,
    tag: &str,
    page: i64,
    results: &LiveSearchResults,
    filter: &TagFilter,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let total = i64::try_from(results.total).unwrap_or(i64::MAX);
    let per_page = 20;
    let start = (page - 1) * per_page + 1;
    let end = start + results.works.len() as i64 - 1;
    let noun = if total == 1 { "Work" } else { "Works" };
    let heading = if total > per_page {
        format!(
            "{start} - {end} of {} {noun} in <a href=\"{}\">{}</a>",
            crate::time::format_count(u64::try_from(total).unwrap_or(0)),
            super::tag_href(tag),
            escape(tag)
        )
    } else {
        format!(
            "{} {noun} in <a href=\"{}\">{}</a>",
            crate::time::format_count(u64::try_from(total).unwrap_or(0)),
            super::tag_href(tag),
            escape(tag)
        )
    };
    let tag_path = format!("/tags/{}/works", percent_encode(tag, SpaceMode::Percent20));
    let extra_query = filter.to_query();
    let refresh_href = if extra_query.is_empty() {
        format!("{tag_path}?page={page}&refresh=1")
    } else {
        format!("{tag_path}?{extra_query}&page={page}&refresh=1")
    };
    let mut body = format!("<h2 class=\"heading\">{heading}</h2>");
    body.push_str(&format!(
        "<div class=\"navigation actions module\">\
         <h3 class=\"landmark heading\">Navigation and Actions</h3>\
         <ul class=\"user navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">Works</span></li>\
         <li><a href=\"/bookmarks\">Bookmarks</a></li>\
         <li><a href=\"/works\">Clear tag</a></li>\
         <li><a href=\"/works/search?include_tags={}\">Search within this tag on AO3</a></li>\
         <li><a href=\"{refresh_href}\">Refresh</a></li>\
         </ul></div>",
        percent_encode(tag, SpaceMode::Percent20)
    ));
    if let Some((from_cache, fetched_at)) = cache_meta {
        let status = if from_cache {
            format!(
                "Cached from archiveofourown.org at {} (30-min cache)",
                escape(&crate::time::format_utc(fetched_at))
            )
        } else {
            "Live from archiveofourown.org (cached for 30 min)".to_string()
        };
        body.push_str(&format!(
            "<p class=\"meta\">{status} · <a href=\"{refresh_href}\">Refresh now</a></p>"
        ));
    }
    let pager = crate::render::pagination(page, per_page, total, &tag_path, &extra_query);
    body.push_str("<div class=\"browse-grid\"><section class=\"browse-results\">");
    if results.works.is_empty() {
        body.push_str("<p class=\"muted\">No works on this tag page.</p>");
    } else {
        body.push_str(&pager);
        body.push_str("<h3 class=\"landmark heading\">Listing Works</h3>");
        body.push_str("<ol class=\"work index group\">");
        for work in &results.works {
            body.push_str(&live_blurb(work));
        }
        body.push_str("</ol>");
        body.push_str(&pager);
        if ctx.account.is_empty() {
            body.push_str(
                "<p class=\"meta\">Signed out: this is AO3's signed-out tag page. \
                 <a href=\"/session#cookie-import\">Import your AO3 session</a> for your \
                 account's view.</p>",
            );
        }
    }
    body.push_str("</section>");
    body.push_str(&tag_filter_sidebar(tag, filter));
    body.push_str("</div>");
    layout_with_class(ctx, "works-index filtered", &body)
}

fn query_page(query: &LiveSearchQuery) -> i64 {
    query.page.max(1)
}

pub(super) fn live_blurb(work: &LiveWork) -> String {
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
    let fandoms = if work.fandoms.is_empty() {
        "—".to_string()
    } else {
        work.fandoms
            .iter()
            .map(|fandom| {
                format!(
                    "<a class=\"tag\" href=\"{}\">{}</a>",
                    super::tag_href(fandom),
                    escape(fandom)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let tag_links = |tags: &[String], class: &str| -> String {
        tags.iter()
            .map(|tag| {
                format!(
                    "<li class=\"{class}\"><a class=\"tag\" href=\"{}\">{}</a></li>",
                    super::tag_href(tag),
                    escape(tag)
                )
            })
            .collect()
    };
    let tags = format!(
        "{}{}{}{}",
        tag_links(&work.warnings, "warnings"),
        tag_links(&work.relationships, "relationships"),
        tag_links(&work.characters, "characters"),
        tag_links(&work.additional_tags, "freeforms"),
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
    let required = required_tags_from(
        work.rating.as_deref(),
        &work.warnings,
        &work.categories,
        work.complete,
        false,
    );
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
    for (class, label, value) in [
        ("collections", "Collections", work.collections),
        ("comments", "Comments", work.comments),
        ("kudos", "Kudos", work.kudos),
        ("bookmarks", "Bookmarks", work.bookmarks),
        ("hits", "Hits", work.hits),
    ] {
        if let Some(value) = value.filter(|value| *value > 0) {
            stats.push_str(&format!(
                "<dt class=\"{class}\">{label}:</dt><dd class=\"{class}\">{}</dd>",
                crate::time::format_count(value)
            ));
        }
    }
    format!(
        "<li id=\"work_{id}\" class=\"work blurb group\" role=\"article\">\
         <div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/open/{id}\">{title}</a> by {authors}{restricted}</h4>\
         <h5 class=\"fandoms heading\"><span class=\"landmark\">Fandoms:</span> {fandoms}</h5>\
         {required}\
         <p class=\"datetime\">{datetime}</p></div>\
         <h6 class=\"landmark heading\">Tags</h6><ul class=\"tags commas\">{tags}</ul>{summary}{series}\
         <dl class=\"stats\">{stats}</dl>\
         <h6 class=\"landmark heading\">Actions</h6>\
         <ul class=\"actions\">\
         <li><a href=\"https://archiveofourown.org/works/{id}\" \
         rel=\"noopener noreferrer\">Open on AO3</a></li>\
         <li><form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{id}\">\
         <button type=\"submit\">Cache this work</button></form></li></ul></li>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        datetime = escape(&work.datetime),
        restricted = restricted,
        series = series,
    )
}

fn radio_group(name: &str, options: &[(&str, &str)], current: &str) -> String {
    let items: String = options
        .iter()
        .map(|(value, label)| {
            format!(
                "<li><label class=\"check\"><input type=\"radio\" name=\"{name}\" value=\"{}\"{}> \
                 {label}</label></li>",
                escape(value),
                if current == *value { " checked" } else { "" }
            )
        })
        .collect();
    format!("<ul class=\"choice-list\">{items}</ul>")
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
