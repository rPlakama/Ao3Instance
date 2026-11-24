//! Live AO3 Work Search form, result list, and Cloudflare challenge page.

use crate::ao3::search::{LiveSearchQuery, LiveSearchResults};
use crate::render::blurb::live_blurb;
use crate::render::chrome::{NavSection, PageContext, layout, layout_with_class};
use crate::render::components::cache_banner;
use crate::render::escape;

use super::{
    AO3_LANGUAGES, COMPLETION_OPTIONS, CROSSOVER_OPTIONS, autocomplete_input, live_select,
    number_input, radio_group, row, text_input,
};

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
            COMPLETION_OPTIONS,
            query.complete.as_deref().unwrap_or(""),
        ),
    ));
    body.push_str(&row(
        "Crossovers",
        &radio_group(
            "crossover",
            CROSSOVER_OPTIONS,
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
            AO3_LANGUAGES,
            query.language.as_deref().unwrap_or(""),
        ),
    ));
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset><legend>Work Tags</legend><h3 class=\"landmark heading\">Work Tags</h3><dl>",
    );
    body.push_str(&row(
        "Fandoms",
        &autocomplete_input(
            None,
            "fandom",
            query.fandom.as_deref(),
            "fandom name",
            "fandom",
        ),
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
        &autocomplete_input(
            None,
            "characters",
            query.characters.as_deref(),
            "character name",
            "character",
        ),
    ));
    body.push_str(&row(
        "Relationships",
        &autocomplete_input(
            None,
            "relationships",
            query.relationships.as_deref(),
            "relationship name",
            "relationship",
        ),
    ));
    body.push_str(&row(
        "Additional Tags",
        &autocomplete_input(
            None,
            "additional_tags",
            query.additional_tags.as_deref(),
            "freeform tag",
            "freeform",
        ),
    ));
    body.push_str(&row(
        "Other tags to include",
        &autocomplete_input(
            None,
            "include_tags",
            Some(&query.include_tags.join(", ")),
            "Search tags to include",
            "tag",
        ),
    ));
    body.push_str(&row(
        "Other tags to exclude",
        &autocomplete_input(
            None,
            "exclude_tags",
            Some(&query.exclude_tags.join(", ")),
            "Search tags to exclude",
            "tag",
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
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live results from archiveofourown.org",
    ));
    if results.total == 0 {
        body.push_str(
            "<p>No results found. You may want to edit your search to make it less \
             specific.</p>",
        );
    } else {
        let pager = crate::render::pagination(
            query.page.max(1),
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

/// Edge block on a live search: one explicit solve-and-retry, never automatic.
pub fn search_challenge_page(
    status: u16,
    target: &str,
    retry: &str,
    solver_available: bool,
    send_cookies_allowed: bool,
) -> String {
    let mut body = format!(
        "<h2>Search blocked</h2><div class=\"error\">AO3 answered HTTP {status}. \
         Search is one literal AO3 request; it has not been retried automatically.</div>"
    );
    if solver_available {
        body.push_str(
            "<div class=\"continue\"><strong>A local FlareSolverr is available.</strong> \
             It loads this exact result page once in a real browser. Only press this if you \
             want to spend that solve.</div>",
        );
        body.push_str(&format!(
            "<form class=\"stack\" method=\"post\" action=\"/solve\">\
             <input type=\"hidden\" name=\"target\" value=\"{}\">\
             <input type=\"hidden\" name=\"next\" value=\"{}\">",
            escape(target),
            escape(retry)
        ));
        if send_cookies_allowed {
            body.push_str(
                "<label><input type=\"checkbox\" name=\"send_cookies\" value=\"1\"> \
                 Send my AO3 session cookies to the local solver (needed for restricted or \
                 adult results)</label>",
            );
        }
        body.push_str("<button type=\"submit\">Solve &amp; show this search</button></form>");
    } else {
        body.push_str(
            "<p class=\"meta\">The solver is off or unreachable. A host-provided \
             FlareSolverr can help; check <a href=\"/diagnostics\">Diagnostics</a>.</p>",
        );
    }
    body.push_str(
        "<p><a href=\"/session#cookie-import\">Import fresh session cookies</a> · \
         <a href=\"/diagnostics\">Diagnostics</a> · <a href=\"/works\">Browse the cache</a></p>",
    );
    layout(
        &PageContext::new("AO3Instance — search blocked", NavSection::Search),
        &body,
    )
}
