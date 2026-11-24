//! Work and reader page modelled directly on `otwarchive/app/views/works/show.html.erb`,
//! `_work_header_navigation.html.erb`, `_meta.html.erb`, `_preface.html.erb`,
//! `chapters/_chapter.html.erb`, and `comments/_commentable.html.erb`.
//!
//! Author HTML always passes through [`sanitize`] (`ammonia` allowlist,
//! `http`/`https` schemes only, forced `noopener noreferrer`) before rendering
//! inline inside `#chapters`, matching AO3's own `sanitize_field` model so
//! every built-in and imported public AO3 site skin styles prose seamlessly.

use super::blurb::author_links;
use super::chrome::{PageContext, checked_label, layout_with_class};
use super::{escape, sanitize};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{StoredChapter, StoredWork};
use crate::time::format_count;

/// Everything the reader needs that is not the work itself.
pub struct ReadContext<'a> {
    pub current: i64,
    pub progress: Option<crate::store::Progress>,
    /// Exact scroll percentage (0.0..=100.0) to restore inside the current chapter.
    pub resume_pct: Option<f64>,
    pub notice: Option<&'a str>,
    pub subscribed: bool,
    pub can_write: bool,
    pub kudos_sent: bool,
    pub bookmarked: bool,
    pub reading_state: Option<&'a str>,
    /// `?view_full_work=true`: render every cached chapter stacked.
    pub full_work: bool,
    /// Pseud name for the comment box, once this session discovered one.
    pub comment_as: Option<&'a str>,
}

pub fn read_page(
    page: &PageContext,
    work: &StoredWork,
    chapters: &[StoredChapter],
    ctx: &ReadContext<'_>,
) -> String {
    let ReadContext {
        current,
        progress,
        resume_pct,
        notice,
        subscribed,
        can_write,
        kudos_sent,
        bookmarked,
        reading_state,
        full_work,
        comment_as,
    } = *ctx;
    let id = escape(&work.ao3_id);
    let authors = author_links(work);
    let chapter = chapters.iter().find(|chapter| chapter.idx == current);
    let total = chapters.len();
    let position = chapters
        .iter()
        .position(|chapter| chapter.idx == current)
        .unwrap_or(0);
    let chapter_total = i64::try_from(work.chapter_count)
        .unwrap_or(i64::MAX)
        .max(i64::try_from(total).unwrap_or(i64::MAX));
    let inside_pct = progress
        .filter(|saved| saved.chapter_idx == current)
        .map_or(0, |saved| saved.pct);
    let overall = super::overall_percent(
        i64::try_from(position).unwrap_or(0) + 1,
        inside_pct,
        chapter_total,
    );
    let is_multi_chapter = total > 1;

    let mut body = String::from(
        "<ul class=\"landmark skip\">\
         <li><a id=\"top\">&nbsp;</a></li>\
         <li><a href=\"#workskin\">Skip header</a></li></ul>",
    );
    if let Some(notice) = notice {
        body.push_str(&format!(
            "<div class=\"flash notice\">{}</div>",
            escape(notice)
        ));
    }
    if let Some(saved) = progress
        && saved.chapter_idx != current
        && !full_work
    {
        body.push_str(&format!(
            "<div class=\"continue\">You were at chapter {chapter}. \
             <a href=\"/read/{id}?chapter={chapter}\">Resume</a></div>",
            chapter = saved.chapter_idx
        ));
    }

    body.push_str("<div class=\"work\">");

    // Top work navigation actions (otwarchive/app/views/works/_work_header_navigation.html.erb)
    body.push_str("<h3 class=\"landmark heading\">Actions</h3>");
    body.push_str("<ul class=\"work navigation actions\">");
    if is_multi_chapter {
        if full_work {
            body.push_str(&format!(
                "<li class=\"chapter bychapter\">\
                 <a href=\"/read/{id}?chapter={current}\">Chapter by Chapter</a></li>"
            ));
        } else {
            body.push_str(&format!(
                "<li class=\"chapter entire\">\
                 <a href=\"/read/{id}?view_full_work=true\">Entire Work</a></li>"
            ));
            if position > 0 {
                let previous = chapters[position - 1].idx;
                body.push_str(&format!(
                    "<li class=\"chapter previous\">\
                     <a href=\"/read/{id}?chapter={previous}#workskin\">← Previous Chapter</a></li>"
                ));
            }
            if position + 1 < total {
                let next = chapters[position + 1].idx;
                body.push_str(&format!(
                    "<li class=\"chapter next\">\
                     <a href=\"/read/{id}?chapter={next}#workskin\">Next Chapter →</a></li>"
                ));
            }
        }
        let mut options = String::new();
        for (idx, candidate) in chapters.iter().enumerate() {
            let label = candidate.title.as_deref().filter(|t| !t.trim().is_empty());
            let display = match label {
                Some(title) => format!("{}. {}", idx + 1, escape(title)),
                None => format!("{}. Chapter {}", idx + 1, candidate.idx),
            };
            options.push_str(&format!(
                "<option value=\"{}\"{}>{display}</option>",
                candidate.idx,
                if candidate.idx == current {
                    " selected"
                } else {
                    ""
                },
            ));
        }
        body.push_str(&format!(
            "<li class=\"chapter dropdown\">\
             <button type=\"button\" data-dropdown-toggle aria-haspopup=\"true\" \
             aria-expanded=\"false\">Chapter Index</button>\
             <ul id=\"chapter_index\" class=\"expandable secondary menu\">\
             <li><form method=\"get\" action=\"/read/{id}\">\
             <label class=\"landmark\" for=\"selected_id\">Chapter</label>\
             <select id=\"selected_id\" name=\"chapter\">{options}</select>\
             <button type=\"submit\">Go</button></form></li>\
             </ul></li>"
        ));
    }
    if can_write {
        if bookmarked {
            body.push_str(
                "<li class=\"bookmark\"><span class=\"state active\">bookmarked</span></li>",
            );
        } else {
            body.push_str(&format!(
                "<li class=\"bookmark\">\
                 <a href=\"/write/preview?kind=bookmark&id={id}\">Bookmark</a></li>"
            ));
        }
        match reading_state {
            Some("for_later") => {
                body.push_str(&format!(
                    "<li class=\"mark\"><span class=\"state active\">marked for later</span></li>\
                     <li class=\"mark\"><form class=\"inline\" method=\"post\" action=\"/write/reading\">\
                     <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                     <input type=\"hidden\" name=\"action\" value=\"read\">\
                     <button type=\"submit\">Mark as read</button></form></li>"
                ));
            }
            Some("read") => {
                body.push_str(&format!(
                    "<li class=\"mark\"><span class=\"state active\">marked read</span></li>\
                     <li class=\"mark\"><form class=\"inline\" method=\"post\" action=\"/write/reading\">\
                     <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                     <input type=\"hidden\" name=\"action\" value=\"later\">\
                     <button type=\"submit\">Mark for later</button></form></li>"
                ));
            }
            _ => {
                body.push_str(&format!(
                    "<li class=\"mark\"><form class=\"inline\" method=\"post\" action=\"/write/reading\">\
                     <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                     <input type=\"hidden\" name=\"action\" value=\"later\">\
                     <button type=\"submit\">Mark for later</button></form></li>\
                     <li class=\"mark\"><form class=\"inline\" method=\"post\" action=\"/write/reading\">\
                     <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                     <input type=\"hidden\" name=\"action\" value=\"read\">\
                     <button type=\"submit\">Mark as read</button></form></li>"
                ));
            }
        }
        body.push_str("<li class=\"comments\"><a href=\"#feedback\">Comments</a></li>");
        if subscribed {
            body.push_str(&format!(
                "<li class=\"subscribe\"><span class=\"state active\">subscribed</span></li>\
                 <li class=\"subscribe\"><form class=\"inline\" method=\"post\" \
                 action=\"/write/unsubscribe\">\
                 <input type=\"hidden\" name=\"kind\" value=\"Work\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                 <button type=\"submit\">Unsubscribe</button></form></li>"
            ));
        } else {
            body.push_str(&format!(
                "<li class=\"subscribe\">\
                 <a href=\"/write/preview?kind=work&id={id}\">Subscribe to work</a></li>"
            ));
        }
        if kudos_sent {
            body.push_str(
                "<li class=\"kudos\"><span class=\"state active\">kudos given</span></li>",
            );
        } else {
            body.push_str(&format!(
                "<li class=\"kudos\">\
                 <a href=\"/write/preview?kind=kudos&id={id}\">Give kudos</a></li>"
            ));
        }
    } else {
        body.push_str("<li><a href=\"/session\">Sign in to write</a></li>");
    }
    body.push_str(&format!(
        "<li class=\"save\"><form class=\"inline save-place\" method=\"post\" action=\"/progress\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{id}\">\
         <input type=\"hidden\" name=\"chapter\" value=\"{current}\">\
         <input type=\"hidden\" name=\"pct\" value=\"{inside_pct}\">\
         <button type=\"submit\">Save Place</button></form></li>\
         <li class=\"refresh\"><form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{id}\">\
         <input type=\"hidden\" name=\"refresh\" value=\"1\">\
         <button type=\"submit\">Refresh from AO3</button></form></li>\
         <li class=\"external\"><a href=\"https://archiveofourown.org/works/{id}\" \
         rel=\"noopener noreferrer\">Open on AO3</a></li>"
    ));
    body.push_str("</ul>");

    // Work metadata box (otwarchive/app/views/works/_meta.html.erb)
    body.push_str("<h3 class=\"landmark heading\">Work Header</h3><div class=\"wrapper\">");
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
        meta_rows.push_str(&format!(
            "<dt class=\"language\">Language:</dt><dd class=\"language\">{}</dd>",
            escape(language)
        ));
    }

    let mut stats_rows = String::new();
    if let Some(published) = &work.published_at
        && !published.trim().is_empty()
    {
        stats_rows.push_str(&format!(
            "<dt class=\"published\">Published:</dt><dd class=\"published\">{}</dd>",
            escape(published)
        ));
    }
    if let Some(words) = work.word_count {
        stats_rows.push_str(&format!(
            "<dt class=\"words\">Words:</dt><dd class=\"words\">{}</dd>",
            format_count(words)
        ));
    }
    let chapters_display = match work.complete {
        Some(true) => format!("{0}/{0}", work.chapter_count),
        Some(false) => format!("{0}/?", work.chapter_count),
        None => work.chapter_count.to_string(),
    };
    stats_rows.push_str(&format!(
        "<dt class=\"chapters\">Chapters:</dt><dd class=\"chapters\">{chapters_display}</dd>"
    ));
    if let Some(comments) = work.comment_count {
        stats_rows.push_str(&format!(
            "<dt class=\"comments\">Comments:</dt><dd class=\"comments\">{}</dd>",
            format_count(comments)
        ));
    }
    if let Some(kudos) = work.kudos {
        stats_rows.push_str(&format!(
            "<dt class=\"kudos\">Kudos:</dt><dd class=\"kudos\">{}</dd>",
            format_count(kudos)
        ));
    }
    if let Some(bookmarks) = work.bookmarks {
        stats_rows.push_str(&format!(
            "<dt class=\"bookmarks\">Bookmarks:</dt><dd class=\"bookmarks\">{}</dd>",
            format_count(bookmarks)
        ));
    }
    if let Some(hits) = work.hits {
        stats_rows.push_str(&format!(
            "<dt class=\"hits\">Hits:</dt><dd class=\"hits\">{}</dd>",
            format_count(hits)
        ));
    }
    stats_rows.push_str(&format!(
        "<dt class=\"reading-progress-label\">Reading:</dt>\
         <dd class=\"reading-progress\" data-work-id=\"{id}\" data-chapter=\"{current}\" \
         data-total-chapters=\"{chapter_total}\" data-initial-pct=\"{overall}\">\
         <span class=\"reading-pct\">{overall}% read</span></dd>"
    ));
    stats_rows.push_str(&format!(
        "<dt class=\"checked\">Checked:</dt><dd class=\"checked\">{}</dd>",
        escape(&checked_label(work.fetched_at))
    ));
    meta_rows.push_str(&format!(
        "<dt class=\"stats\">Stats:</dt><dd class=\"stats\">\
         <dl class=\"stats\">{stats_rows}</dl></dd>"
    ));
    body.push_str(&format!(
        "<dl class=\"work meta group\">{meta_rows}</dl></div>"
    ));

    // Work skin + preface + inline chapters (otwarchive/app/views/works/show.html.erb)
    body.push_str("<div id=\"work-skin\" class=\"wrapper\"><div id=\"workskin\">");
    body.push_str(&format!(
        "<div class=\"preface group\">\
         <h2 class=\"title heading\">{}</h2>\
         <h3 class=\"byline heading\">{}</h3>",
        escape(&work.title),
        authors
    ));
    if (full_work || position == 0)
        && let Some(summary) = &work.summary
        && !summary.trim().is_empty()
    {
        body.push_str(&format!(
            "<div class=\"summary module\"><h3 class=\"heading\">Summary:</h3>\
             <blockquote class=\"userstuff\">{}</blockquote></div>",
            sanitize(summary)
        ));
    }
    body.push_str("</div>"); // .preface.group

    body.push_str("<div id=\"chapters\" role=\"article\">");
    if full_work && !chapters.is_empty() {
        for (index, candidate) in chapters.iter().enumerate() {
            let resume = resume_pct
                .filter(|_| candidate.idx == current)
                .unwrap_or(0.0);
            body.push_str(&render_chapter_inline(
                &id,
                candidate,
                index + 1,
                total,
                chapter_total,
                resume,
                index == 0,
            ));
        }
    } else {
        match chapter {
            Some(chapter) => {
                let resume = resume_pct.filter(|_| chapter.idx == current).unwrap_or(0.0);
                body.push_str(&render_chapter_inline(
                    &id,
                    chapter,
                    position + 1,
                    total,
                    chapter_total,
                    resume,
                    true,
                ));
            }
            None => body.push_str(
                "<p class=\"muted\">No chapter content was stored for this work. \
                 Re-open it to refresh (AO3's chapter markup may have changed).</p>",
            ),
        }
    }
    body.push_str("</div>"); // #chapters
    body.push_str("</div></div>"); // #workskin, #work-skin
    body.push_str("</div>"); // .work

    // Feedback & bottom navigation (otwarchive/app/views/comments/_commentable.html.erb)
    body.push_str("<div id=\"feedback\" class=\"feedback\">");
    body.push_str("<h3 class=\"landmark heading\">Actions</h3><ul class=\"actions\">");
    body.push_str("<li><a href=\"#top\">↑ Top</a></li>");
    if !full_work && position > 0 {
        let previous = chapters[position - 1].idx;
        body.push_str(&format!(
            "<li><a href=\"/read/{id}?chapter={previous}#workskin\">← Previous Chapter</a></li>"
        ));
    }
    if !full_work && position + 1 < total {
        let next = chapters[position + 1].idx;
        body.push_str(&format!(
            "<li><a href=\"/read/{id}?chapter={next}#workskin\">Next Chapter →</a></li>"
        ));
    }
    if can_write {
        if !kudos_sent {
            body.push_str(&format!(
                "<li><a href=\"/write/preview?kind=kudos&id={id}\">Give kudos</a></li>"
            ));
        }
        if !bookmarked {
            body.push_str(&format!(
                "<li><a href=\"/write/preview?kind=bookmark&id={id}\">Bookmark</a></li>"
            ));
        }
    }
    body.push_str("</ul>");
    if can_write {
        let commenter = comment_as.map_or_else(|| "your AO3 pseud".to_string(), escape);
        body.push_str(&format!(
            "<div id=\"add_comment\"><details class=\"comment\">\
             <summary class=\"action\">Comment on this work</summary>\
             <form class=\"post comment\" method=\"post\" action=\"/write/comment/preview\">\
             <fieldset><legend>Post Comment</legend>\
             <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
             <h4 class=\"heading\">Comment as <strong>{commenter}</strong></h4>\
             <p class=\"footnote\">It will be public on AO3 (max 10,000 characters).</p>\
             <p><label class=\"landmark\" for=\"comment_content\">Comment</label>\
             <textarea id=\"comment_content\" name=\"content\" rows=\"6\" \
             maxlength=\"10000\" required></textarea></p>\
             <p class=\"submit actions\"><button type=\"submit\">Preview comment</button></p>\
             </fieldset></form></details></div>"
        ));
    }
    body.push_str("</div>");

    body.push_str(&format!(
        "<script src=\"/assets/reader.js?v={:016x}\" defer></script>",
        super::READER_JS_HASH
    ));
    layout_with_class(page, "works-show", &body)
}

/// Renders one `ammonia`-sanitized chapter directly inline inside `#chapters`
/// (matching `otwarchive/app/views/chapters/_chapter.html.erb`).
fn render_chapter_inline(
    work_id: &str,
    chapter: &StoredChapter,
    number: usize,
    total: usize,
    chapter_total: i64,
    resume_pct: f64,
    is_first_rendered: bool,
) -> String {
    let resume_exact = if resume_pct.is_finite() {
        resume_pct.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let resume_int = resume_exact.round() as i64;
    let has_title = chapter
        .title
        .as_deref()
        .is_some_and(|title| !title.trim().is_empty());
    let show_heading = total > 1 || has_title;

    let mut out = format!(
        "<div class=\"chapter\" id=\"chapter-{idx}\" data-work-id=\"{work_id}\" \
         data-chapter=\"{idx}\" data-total-chapters=\"{chapter_total}\" \
         data-resume=\"{resume_int}\" data-resume-exact=\"{resume_exact:.2}\">",
        idx = chapter.idx,
    );
    if show_heading {
        let title_suffix = chapter
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map_or_else(String::new, |title| format!(": {}", escape(title)));
        out.push_str(&format!(
            "<div class=\"chapter preface group\">\
             <h3 class=\"title\"><a href=\"/read/{work_id}?chapter={idx}\">Chapter {number}</a>{title_suffix}</h3>\
             </div>",
            idx = chapter.idx,
        ));
    }
    let work_anchor = if is_first_rendered {
        " id=\"work\""
    } else {
        ""
    };
    let landmark = if show_heading {
        "Chapter Text"
    } else {
        "Work Text:"
    };
    out.push_str(&format!(
        "<div class=\"userstuff module\" role=\"article\">\
         <h3 class=\"landmark heading\"{work_anchor}>{landmark}</h3>{}</div>",
        sanitize(&chapter.body_html)
    ));
    if let Some(notes) = &chapter.notes_html
        && !notes.trim().is_empty()
    {
        out.push_str(&format!(
            "<div class=\"chapter preface group afterword-notes\">\
             <div class=\"end notes module\" id=\"chapter_{idx}_endnotes\">\
             <h3 class=\"heading\">Notes:</h3>\
             <blockquote class=\"userstuff\">{}</blockquote></div></div>",
            sanitize(notes),
            idx = chapter.idx,
        ));
    }
    out.push_str("</div>");
    out
}

/// Standalone sanitized chapter document served by `/read/{id}/chapter/{idx}`.
pub fn chapter_document(work_title: &str, chapter: &StoredChapter, skin: &str) -> String {
    let skin = match skin {
        "auto" | "reversi" | "snow" | "frost" | "cloud" | "sepia" | "midnight" => skin,
        _ => "default",
    };
    let mut body = String::new();
    body.push_str("<div class=\"userstuff\">");
    body.push_str(&sanitize(&chapter.body_html));
    body.push_str("</div>");
    if let Some(notes) = &chapter.notes_html
        && !notes.trim().is_empty()
    {
        body.push_str(&format!(
            "<hr class=\"notes-sep\"><div class=\"notes\"><h3>Chapter notes</h3>\
             <blockquote class=\"userstuff\">{}</blockquote></div>",
            sanitize(notes)
        ));
    }
    format!(
        "<!DOCTYPE html><html lang=\"en\" data-skin=\"{skin}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{} — chapter</title><style>\
         :root{{color-scheme:light;}} \
         :root[data-skin=\"auto\"]{{color-scheme:light dark;}} \
         :root[data-skin=\"reversi\"],:root[data-skin=\"midnight\"]{{color-scheme:dark;}} \
         body{{margin:0;padding:0.5rem 1.5rem 2rem;color:#2a2a2a;background:transparent;\
         font:1.05rem/1.5 \"Lucida Grande\",\"Lucida Sans Unicode\",Verdana,Helvetica,sans-serif;}} \
         .userstuff p{{margin:1.286em auto;}} \
         .userstuff hr{{width:33%;margin:0.875em auto 1.2525em;border:0;border-top:1px solid #999;}} \
         a{{color:#900;text-decoration:none;border-bottom:1px solid;}} \
         img{{max-width:100%;height:auto;}} \
         :root[data-skin=\"snow\"] body{{color:#1c2b3a;}} \
         :root[data-skin=\"snow\"] a{{color:#347391;}} \
         :root[data-skin=\"frost\"] body{{color:#16333f;}} \
         :root[data-skin=\"frost\"] a{{color:#3f7c8c;}} \
         :root[data-skin=\"cloud\"] body{{color:#30363b;}} \
         :root[data-skin=\"cloud\"] a{{color:#5b6b7a;}} \
         :root[data-skin=\"sepia\"] body{{color:#3d2f21;}} \
         :root[data-skin=\"sepia\"] a{{color:#8a4a2b;}} \
         :root[data-skin=\"reversi\"] body{{color:#eee;}} \
         :root[data-skin=\"reversi\"] a{{color:#5998d6;}} \
         :root[data-skin=\"reversi\"] .notes,\
         :root[data-skin=\"reversi\"] blockquote{{color:#b8b8b8;border-color:#555;}} \
         :root[data-skin=\"midnight\"] body{{color:#dbe4ef;}} \
         :root[data-skin=\"midnight\"] a{{color:#8fb5d9;}} \
         :root[data-skin=\"midnight\"] .notes,\
         :root[data-skin=\"midnight\"] blockquote{{color:#a5b4c5;border-color:#3c4e60;}} \
         @media (prefers-color-scheme: dark) {{\
         :root[data-skin=\"auto\"] body{{color:#eee;}} \
         :root[data-skin=\"auto\"] a{{color:#5998d6;}} \
         :root[data-skin=\"auto\"] .notes,\
         :root[data-skin=\"auto\"] blockquote{{color:#b8b8b8;border-color:#555;}}}} \
         .notes-sep{{border:0;border-top:1px solid #ccc4;margin:1.75rem 0 1rem;}} \
         .notes h3{{font:400 1.15rem/1.2 Georgia,serif;border-bottom:1px solid #999;margin:0 0 0.6rem;}} \
         .notes{{font-size:.92rem;color:#555;}} \
         blockquote{{border-left:2px solid #999;margin:1rem 0 1rem 1.2rem;padding:0.25rem 0.9rem;}}</style>\
         </head><body>{body}</body></html>",
        escape(work_title)
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
                super::tag_href(tag),
                escape(tag)
            )
        })
        .collect();
    format!(
        "<dt class=\"{kind} tags\">{label}</dt>\
         <dd class=\"{kind} tags\"><ul class=\"commas\">{items}</ul></dd>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::NavSection;
    use crate::store::TagGroups;

    fn sample_work() -> StoredWork {
        StoredWork {
            ao3_id: "42".into(),
            title: "A Test Work".into(),
            authors: vec!["Author".into()],
            summary: Some("<p>Summary text.</p>".into()),
            tags: TagGroups {
                fandoms: vec!["Fandom A".into(), "Fandom B".into()],
                relationships: vec!["A/B".into()],
                characters: vec!["A".into()],
                additional: vec!["Fluff".into()],
                warnings: vec!["No Archive Warnings Apply".into()],
                categories: vec!["Gen".into()],
            },
            rating: Some("General Audiences".into()),
            language: Some("English".into()),
            published_at: Some("2025-01-15".into()),
            complete: Some(true),
            word_count: Some(1500),
            kudos: Some(12),
            comment_count: Some(3),
            hits: Some(100),
            bookmarks: Some(4),
            chapter_count: 1,
            content_hash: "h".into(),
            fetched_at: 1_700_000_000,
        }
    }

    #[test]
    fn read_page_renders_sanitized_chapter_inline_with_ao3_structure() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello <em>world</em></p><script>bad()</script>".into(),
            notes_html: Some("<p>Author note.</p>".into()),
        }];
        let html = read_page(
            &page,
            &work,
            &chapters,
            &ReadContext {
                current: 1,
                progress: Some(crate::store::Progress {
                    chapter_idx: 1,
                    pct: 42,
                }),
                resume_pct: Some(42.75),
                notice: None,
                subscribed: false,
                can_write: false,
                kudos_sent: false,
                bookmarked: false,
                reading_state: None,
                full_work: false,
                comment_as: None,
            },
        );
        assert!(!html.contains("<iframe"), "chapters render inline: {html}");
        assert!(html.contains("<p>Hello <em>world</em></p>"), "{html}");
        assert!(!html.contains("bad()"), "author scripts stripped: {html}");
        assert!(html.contains("data-resume=\"43\""), "{html}");
        assert!(html.contains("data-resume-exact=\"42.75\""), "{html}");
        assert!(html.contains("Fandoms:"), "pluralised tag label: {html}");
        assert!(html.contains("Published:"), "published stat: {html}");
        assert!(html.contains("2025-01-15"), "{html}");
        assert!(html.contains("42% read"), "{html}");
        assert!(
            !html.contains("Entire Work"),
            "single-chapter work hides multi-chapter nav: {html}"
        );
        assert!(
            !html.contains("Chapter 1</a>"),
            "untitled single-chapter work omits redundant Chapter 1 header: {html}"
        );
    }

    #[test]
    fn chapter_document_includes_sanitized_body_notes_and_all_skins() {
        let chapter = StoredChapter {
            idx: 1,
            title: Some("Chapter <One>".into()),
            body_html: "<p>body</p><script>bad()</script>".into(),
            notes_html: Some("<p>note</p>".into()),
        };
        let doc = chapter_document("Work <One>", &chapter, "default");
        assert!(doc.contains("Work &lt;One&gt;"));
        assert!(!doc.contains("<h3 class=\"chapter-title\""));
        assert!(doc.contains("<p>body</p>"));
        assert!(!doc.contains("bad()"));
        assert!(doc.contains("Chapter notes"));
        assert!(doc.contains("data-skin=\"default\""));

        for skin in [
            "auto", "reversi", "snow", "frost", "cloud", "sepia", "midnight",
        ] {
            let themed = chapter_document("w", &chapter, skin);
            assert!(
                themed.contains(&format!("data-skin=\"{skin}\"")),
                "skin {skin}: {themed}"
            );
        }
        assert_eq!(
            chapter_document("w", &chapter, "neon"),
            chapter_document("w", &chapter, "default"),
            "unknown skins fall back to the light document"
        );
    }
}
