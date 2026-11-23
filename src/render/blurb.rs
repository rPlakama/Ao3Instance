//! AO3 work-list entry: `<li class="work blurb group">` inside
//! `<ol class="work index group">`, shared by search, browse and tag pages.

use super::icons::required_tags;
use super::{escape, sanitize, tag_href};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::StoredWork;
use crate::time::{format_count, format_date, format_iso_date};

/// Renders one AO3 work blurb. `progress` is the saved chapter, if any.
pub fn work_blurb(work: &StoredWork, progress: Option<i64>) -> String {
    let authors = author_links(work);
    let fandoms = fandom_links(work);

    let mut tags = String::new();
    tags.push_str(&tag_items(&work.tags.warnings, "warnings"));
    tags.push_str(&tag_items(&work.tags.relationships, "relationships"));
    tags.push_str(&tag_items(&work.tags.characters, "characters"));
    tags.push_str(&tag_items(&work.tags.additional, "freeforms"));

    let summary = work
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map_or_else(String::new, |summary| {
            format!(
                "<h6 class=\"landmark heading\">Summary</h6>\
                 <blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(summary)
            )
        });

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
        authors = authors,
        fandoms = fandoms,
        stats = stats_block(work),
    )
}

fn fandom_links(work: &StoredWork) -> String {
    if work.tags.fandoms.is_empty() {
        return "—".to_string();
    }
    work.tags
        .fandoms
        .iter()
        .map(|fandom| {
            format!(
                "<a class=\"tag\" href=\"{}\">{}</a>",
                tag_href(fandom),
                escape(fandom)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn tag_items(tags: &[String], class: &str) -> String {
    let mut out = String::new();
    for tag in tags {
        out.push_str(&format!(
            "<li class=\"{class}\"><a class=\"tag\" href=\"{}\">{}</a></li>",
            tag_href(tag),
            escape(tag)
        ));
    }
    out
}

fn stats_block(work: &StoredWork) -> String {
    let mut rows = String::new();
    if let Some(language) = &work.language {
        rows.push_str(&format!(
            "<dt class=\"language\">Language:</dt><dd class=\"language\">{}</dd>",
            escape(language)
        ));
    }
    if let Some(words) = work.word_count {
        rows.push_str(&format!(
            "<dt class=\"words\">Words:</dt><dd class=\"words\">{}</dd>",
            format_count(words)
        ));
    }
    let chapters = match work.complete {
        Some(true) => format!("{0}/{0}", work.chapter_count),
        Some(false) => format!("{0}/?", work.chapter_count),
        None => work.chapter_count.to_string(),
    };
    rows.push_str(&format!(
        "<dt class=\"chapters\">Chapters:</dt><dd class=\"chapters\">{chapters}</dd>"
    ));
    if let Some(comments) = work.comment_count.filter(|count| *count > 0) {
        rows.push_str(&format!(
            "<dt class=\"comments\">Comments:</dt><dd class=\"comments\">{}</dd>",
            format_count(comments)
        ));
    }
    if let Some(kudos) = work.kudos.filter(|count| *count > 0) {
        rows.push_str(&format!(
            "<dt class=\"kudos\">Kudos:</dt><dd class=\"kudos\">{}</dd>",
            format_count(kudos)
        ));
    }
    if let Some(bookmarks) = work.bookmarks.filter(|count| *count > 0) {
        rows.push_str(&format!(
            "<dt class=\"bookmarks\">Bookmarks:</dt><dd class=\"bookmarks\">{}</dd>",
            format_count(bookmarks)
        ));
    }
    if let Some(hits) = work.hits {
        rows.push_str(&format!(
            "<dt class=\"hits\">Hits:</dt><dd class=\"hits\">{}</dd>",
            format_count(hits)
        ));
    }
    if rows.is_empty() {
        String::new()
    } else {
        format!("<dl class=\"stats\">{rows}</dl>")
    }
}

/// Author names as local-filter links.
pub(super) fn author_links(work: &StoredWork) -> String {
    author_links_from(&work.authors)
}

pub(super) fn author_links_from(authors: &[String]) -> String {
    if authors.is_empty() {
        return "Anonymous".to_string();
    }
    authors
        .iter()
        .map(|author| {
            format!(
                "<a href=\"/works?author={}\">{}</a>",
                percent_encode(author, SpaceMode::Percent20),
                escape(author)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
