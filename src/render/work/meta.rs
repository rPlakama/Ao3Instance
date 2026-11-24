//! AO3 work metadata box (`otwarchive/app/views/works/_meta.html.erb`).

use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::render::chrome::checked_label;
use crate::render::components::stat_row;
use crate::render::{escape, tag_href};
use crate::store::StoredWork;
use crate::time::format_count;

pub(super) fn work_meta_box(
    id: &str,
    work: &StoredWork,
    current: i64,
    chapter_total: i64,
    overall: i64,
) -> String {
    let mut meta_rows = String::new();
    if let Some(rating) = &work.rating {
        meta_rows.push_str(&format!(
            "<dt class=\"rating tags\">Rating:</dt>\
             <dd class=\"rating tags\"><ul class=\"commas\"><li>\
             <a class=\"tag\" href=\"/works?rating={}\">{}</a></li></ul></dd>",
            percent_encode(rating, SpaceMode::Percent20),
            escape(rating)
        ));
    }
    meta_rows.push_str(&meta_row(
        "warning",
        "Archive Warning:",
        "Archive Warnings:",
        &work.tags.warnings,
    ));
    meta_rows.push_str(&meta_row(
        "category",
        "Category:",
        "Categories:",
        &work.tags.categories,
    ));
    meta_rows.push_str(&meta_row(
        "fandom",
        "Fandom:",
        "Fandoms:",
        &work.tags.fandoms,
    ));
    meta_rows.push_str(&meta_row(
        "relationship",
        "Relationship:",
        "Relationships:",
        &work.tags.relationships,
    ));
    meta_rows.push_str(&meta_row(
        "character",
        "Character:",
        "Characters:",
        &work.tags.characters,
    ));
    meta_rows.push_str(&meta_row(
        "freeform",
        "Additional Tags:",
        "Additional Tags:",
        &work.tags.additional,
    ));
    if let Some(language) = &work.language {
        meta_rows.push_str(&stat_row("language", "Language", &escape(language)));
    }

    let mut stats_rows = String::new();
    if let Some(published) = &work.published_at
        && !published.trim().is_empty()
    {
        stats_rows.push_str(&stat_row("published", "Published", &escape(published)));
    }
    if let Some(words) = work.word_count {
        stats_rows.push_str(&stat_row("words", "Words", &format_count(words)));
    }
    let chapters_display = match work.complete {
        Some(true) => format!("{0}/{0}", work.chapter_count),
        Some(false) => format!("{0}/?", work.chapter_count),
        None => work.chapter_count.to_string(),
    };
    stats_rows.push_str(&stat_row("chapters", "Chapters", &chapters_display));
    for (class, label, value) in [
        ("comments", "Comments", work.comment_count),
        ("kudos", "Kudos", work.kudos),
        ("bookmarks", "Bookmarks", work.bookmarks),
        ("hits", "Hits", work.hits),
    ] {
        if let Some(value) = value {
            stats_rows.push_str(&stat_row(class, label, &format_count(value)));
        }
    }
    stats_rows.push_str(&format!(
        "<dt class=\"reading-progress-label\">Reading:</dt>\
         <dd class=\"reading-progress\" data-work-id=\"{id}\" data-chapter=\"{current}\" \
         data-total-chapters=\"{chapter_total}\" data-initial-pct=\"{overall}\">\
         <span class=\"reading-pct\">{overall}% read</span></dd>"
    ));
    stats_rows.push_str(&stat_row(
        "checked",
        "Checked",
        &escape(&checked_label(work.fetched_at)),
    ));
    meta_rows.push_str(&format!(
        "<dt class=\"stats\">Stats:</dt><dd class=\"stats\">\
         <dl class=\"stats\">{stats_rows}</dl></dd>"
    ));
    format!(
        "<h3 class=\"landmark heading\">Work Header</h3><div class=\"wrapper\">\
         <dl class=\"work meta group\">{meta_rows}</dl></div>"
    )
}

/// One AO3 metadata row: `<dt class="{kind} tags">` + `<dd><ul class="commas">`.
fn meta_row(kind: &str, singular: &str, plural: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let label = if tags.len() == 1 { singular } else { plural };
    let items: String = tags
        .iter()
        .map(|tag| {
            format!(
                "<li><a class=\"tag\" href=\"{}\">{}</a></li>",
                tag_href(tag),
                escape(tag)
            )
        })
        .collect();
    format!(
        "<dt class=\"{kind} tags\">{label}</dt>\
         <dd class=\"{kind} tags\"><ul class=\"commas\">{items}</ul></dd>"
    )
}
