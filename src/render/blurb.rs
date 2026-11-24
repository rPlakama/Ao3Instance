//! AO3 work-list blurb entries (`<li class="work blurb group">`), shared by
//! offline browse/search, live search/tag/bookmarks, and home read-later cards.

use crate::ao3::home::HomeReading;
use crate::ao3::search::LiveWork;
use crate::store::StoredWork;
use crate::time::{format_count, format_date, format_iso_date};

use super::components::{
    author_links_live, author_links_local, fandom_links, stat_row, summary_block, tag_list_items,
};
use super::escape;
use super::icons::{required_tags, required_tags_from};

/// Renders one cached AO3 work blurb. `progress` is the saved chapter, if any.
pub fn work_blurb(work: &StoredWork, progress: Option<i64>) -> String {
    let authors = author_links_local(&work.authors);
    let fandoms = fandom_links(&work.tags.fandoms);

    let tags = [
        tag_list_items(&work.tags.warnings, "warnings"),
        tag_list_items(&work.tags.relationships, "relationships"),
        tag_list_items(&work.tags.characters, "characters"),
        tag_list_items(&work.tags.additional, "freeforms"),
    ]
    .concat();
    let summary = summary_block(work.summary.as_deref());

    let resume_action = progress.map_or_else(String::new, |chapter| {
        format!(
            "<li><a href=\"/read/{id}?chapter={chapter}\">Resume at chapter {chapter}</a></li>",
            id = escape(&work.ao3_id)
        )
    });

    let datetime = work.published_at.as_deref().map_or_else(
        || format!("Cached {}", format_date(work.fetched_at)),
        format_iso_date,
    );

    format!(
        "<li id=\"work_{id}\" class=\"work blurb group\" role=\"article\">\
         <div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/read/{id}\">{title}</a> by {authors}</h4>\
         <h5 class=\"fandoms heading\"><span class=\"landmark\">Fandoms:</span> {fandoms}</h5>\
         {required}<p class=\"datetime\">{datetime}</p></div>\
         <h6 class=\"landmark heading\">Tags</h6><ul class=\"tags commas\">{tags}</ul>\
         {summary}{stats}\
         <h6 class=\"landmark heading\">Actions</h6>\
         <ul class=\"actions\">\
         <li><a href=\"/read/{id}\">Read offline</a></li>\
         {resume_action}\
         <li><a href=\"https://archiveofourown.org/works/{id}\" rel=\"noopener noreferrer\">\
         Open on AO3</a></li></ul></li>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        required = required_tags(work),
        stats = cached_stats_block(work),
    )
}

/// Renders one live AO3 work blurb (search results, tag pages, bookmarks, readings).
pub(super) fn live_blurb(work: &LiveWork) -> String {
    let authors = author_links_live(&work.authors);
    let fandoms = fandom_links(&work.fandoms);
    let tags = live_tags_html(work);
    let summary = summary_block(work.summary_html.as_deref());
    let series = live_series_block(work);
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
    let stats = live_stats_rows(work, true);
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
    )
}

/// Renders a live "Is it later already?" reading blurb on the home splash page.
pub(super) fn reading_blurb(reading: &HomeReading) -> String {
    let work = &reading.work;
    let authors = author_links_local(&work.authors);
    let fandoms = fandom_links(&work.fandoms);
    let required = required_tags_from(
        work.rating.as_deref(),
        &work.warnings,
        &work.categories,
        work.complete,
        false,
    );
    let tags_html = live_tags_html(work);
    let tags_block = if tags_html.is_empty() {
        String::new()
    } else {
        format!(
            "<h6 class=\"landmark heading\">Tags</h6><ul class=\"tags commas\">{tags_html}</ul>"
        )
    };
    let summary = summary_block(work.summary_html.as_deref());
    let stats_rows = live_stats_rows(work, false);
    let stats_html = if stats_rows.is_empty() {
        String::new()
    } else {
        format!("<dl class=\"stats\">{stats_rows}</dl>")
    };
    format!(
        "<li id=\"work_{id}\" class=\"reading work blurb group\" role=\"article\">\
         <div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/open/{id}\">{title}</a> by {authors}</h4>\
         <h5 class=\"fandoms heading\"><span class=\"landmark\">Fandoms:</span> {fandoms}</h5>\
         {required}<p class=\"datetime\">{datetime}</p></div>\
         {tags_block}{summary}{stats_html}\
         <div class=\"user module group\"><h4 class=\"viewed heading\">{viewed}</h4></div></li>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        datetime = escape(&work.datetime),
        viewed = escape(&reading.meta),
    )
}

fn live_tags_html(work: &LiveWork) -> String {
    [
        tag_list_items(&work.warnings, "warnings"),
        tag_list_items(&work.relationships, "relationships"),
        tag_list_items(&work.characters, "characters"),
        tag_list_items(&work.additional_tags, "freeforms"),
    ]
    .concat()
}

fn live_series_block(work: &LiveWork) -> String {
    if work.series.is_empty() {
        return String::new();
    }
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
}

fn live_stats_rows(work: &LiveWork, full: bool) -> String {
    let mut stats = String::new();
    if full && let Some(language) = &work.language {
        stats.push_str(&stat_row("language", "Language", &escape(language)));
    }
    if let Some(words) = work.words {
        stats.push_str(&stat_row("words", "Words", &format_count(words)));
    }
    if let Some(chapters) = &work.chapters {
        stats.push_str(&stat_row("chapters", "Chapters", &escape(chapters)));
    }
    let counters: &[(&str, &str, Option<u64>)] = if full {
        &[
            ("collections", "Collections", work.collections),
            ("comments", "Comments", work.comments),
            ("kudos", "Kudos", work.kudos),
            ("bookmarks", "Bookmarks", work.bookmarks),
            ("hits", "Hits", work.hits),
        ]
    } else {
        &[("kudos", "Kudos", work.kudos), ("hits", "Hits", work.hits)]
    };
    for (class, label, value) in counters {
        let show = if full {
            value.filter(|v| *v > 0)
        } else {
            *value
        };
        if let Some(value) = show {
            stats.push_str(&stat_row(class, label, &format_count(value)));
        }
    }
    stats
}

fn cached_stats_block(work: &StoredWork) -> String {
    let mut rows = String::new();
    if let Some(language) = &work.language {
        rows.push_str(&stat_row("language", "Language", &escape(language)));
    }
    if let Some(words) = work.word_count {
        rows.push_str(&stat_row("words", "Words", &format_count(words)));
    }
    let chapters = match work.complete {
        Some(true) => format!("{0}/{0}", work.chapter_count),
        Some(false) => format!("{0}/?", work.chapter_count),
        None => work.chapter_count.to_string(),
    };
    rows.push_str(&stat_row("chapters", "Chapters", &chapters));
    for (class, label, value) in [
        ("comments", "Comments", work.comment_count),
        ("kudos", "Kudos", work.kudos),
        ("bookmarks", "Bookmarks", work.bookmarks),
    ] {
        if let Some(count) = value.filter(|count| *count > 0) {
            rows.push_str(&stat_row(class, label, &format_count(count)));
        }
    }
    if let Some(hits) = work.hits {
        rows.push_str(&stat_row("hits", "Hits", &format_count(hits)));
    }
    if rows.is_empty() {
        String::new()
    } else {
        format!("<dl class=\"stats\">{rows}</dl>")
    }
}

/// Author names as local-filter links.
pub(super) fn author_links(work: &StoredWork) -> String {
    author_links_local(&work.authors)
}
