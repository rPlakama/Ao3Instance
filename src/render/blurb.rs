//! The AO3 work-card component shared by home, search and browse.

use super::{escape, sanitize};
use crate::store::StoredWork;

/// Renders one `.work.blurb.group` card. `progress` is the saved chapter, if any.
pub fn work_blurb(work: &StoredWork, progress: Option<i64>) -> String {
    let authors = if work.authors.is_empty() {
        "Anonymous".to_string()
    } else {
        work.authors.join(", ")
    };

    let mut tags = String::new();
    tags.push_str(&tag_items(&work.tags.warnings, "warnings"));
    tags.push_str(&tag_items(&work.tags.fandoms, "fandoms"));
    tags.push_str(&tag_items(&work.tags.relationships, "relationships"));
    tags.push_str(&tag_items(&work.tags.characters, "characters"));
    tags.push_str(&tag_items(&work.tags.additional, "freeforms"));
    tags.push_str(&tag_items(&work.tags.categories, "categories"));

    let summary = work
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map_or_else(String::new, |summary| {
            format!(
                "<blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(summary)
            )
        });

    let resume = progress.map_or_else(String::new, |chapter| {
        format!(
            "<p class=\"meta\"><a href=\"/read/{id}?chapter={chapter}\">Resume at chapter \
             {chapter}</a></p>",
            id = escape(&work.ao3_id)
        )
    });

    let rating = work.rating.as_deref().map_or_else(String::new, |rating| {
        format!("<span class=\"tag rating\">{}</span>", escape(rating))
    });

    format!(
        "<article class=\"work blurb group\"><div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/read/{id}\">{title}</a></h4>\
         <div class=\"byline\">by {authors}</div>{rating}</div>\
         <ul class=\"tags commas\">{tags}</ul>{summary}{resume}\
         <dl class=\"stats\">\
         <dt>Words:</dt><dd>{words}</dd>\
         <dt>Chapters:</dt><dd>{chapters}</dd>\
         <dt>Kudos:</dt><dd>{kudos}</dd>\
         <dt>Comments:</dt><dd>{comments}</dd></dl>\
         <p class=\"meta\"><a href=\"/read/{id}\">Read offline</a> · \
         <a href=\"https://archiveofourown.org/works/{id}\" rel=\"noopener noreferrer\">\
         Open on AO3</a></p></article>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        authors = escape(&authors),
        words = work.word_count.unwrap_or(0),
        chapters = work.chapter_count,
        kudos = work.kudos.unwrap_or(0),
        comments = work.comment_count.unwrap_or(0),
    )
}

fn tag_items(tags: &[String], class: &str) -> String {
    let mut out = String::new();
    for tag in tags {
        out.push_str(&format!(
            "<li class=\"{class}\"><span class=\"tag\">{}</span></li>",
            escape(tag)
        ));
    }
    out
}
