//! Work and reader page modelled on `otwarchive/app/views/works/show.html.erb`,
//! `_work_header_navigation.html.erb`, `_meta.html.erb`, `_preface.html.erb`,
//! and `comments/_commentable.html.erb`, plus the standalone document served to
//! the sandboxed chapter iframe.
//!
//! Author HTML only ever reaches a page through [`sanitize`]; the chapter body
//! gets its own document with `CHAPTER_CSP`, never inline in the app page.

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
    /// Scroll percentage to restore inside the current chapter.
    pub resume_pct: Option<i64>,
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

    let mut body = String::from(
        "<ul class=\"landmark skip\">\
         <li><a id=\"top\">&nbsp;</a></li>\
         <li><a href=\"#chapters\">Skip header</a></li></ul>",
    );
    if let Some(notice) = notice {
        body.push_str(&format!(
            "<div class=\"flash notice\">{}</div>",
            escape(notice)
        ));
    }
    if let Some(saved) = progress
        && saved.chapter_idx != current
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
    if full_work {
        body.push_str(&format!(
            "<li class=\"chapter bychapter\">\
             <a href=\"/read/{id}?chapter={current}\">Chapter by Chapter</a></li>"
        ));
    } else if !chapters.is_empty() {
        body.push_str(&format!(
            "<li class=\"chapter entire\">\
             <a href=\"/read/{id}?view_full_work=true\">Entire Work</a></li>"
        ));
    }
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
    if !chapters.is_empty() {
        let mut options = String::new();
        for candidate in chapters {
            let label = candidate.title.as_deref().filter(|t| !t.trim().is_empty());
            let display = match label {
                Some(title) => format!("{}. {}", candidate.idx, escape(title)),
                None => format!("Chapter {}", candidate.idx),
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
            "<li class=\"chapter jump\"><form class=\"inline\" method=\"get\" action=\"/read/{id}\">\
             <label class=\"landmark\" for=\"chapter_jump\">Chapter</label>\
             <select id=\"chapter_jump\" name=\"chapter\">{options}</select>\
             <button type=\"submit\">Go</button></form></li>"
        ));
    }
    if can_write {
        if bookmarked {
            body.push_str("<li class=\"bookmark\"><span class=\"state active\">bookmarked</span></li>");
        } else {
            body.push_str(&format!(
                "<li class=\"bookmark\">\
                 <a href=\"/write/preview?kind=bookmark&id={id}\">Bookmark</a></li>"
            ));
        }
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
        if let Some(state) = reading_state {
            let label = if state == "read" {
                "marked read"
            } else {
                "marked for later"
            };
            body.push_str(&format!(
                "<li class=\"mark\"><span class=\"state active\">{label}</span></li>"
            ));
        }
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
            body.push_str("<li class=\"kudos\"><span class=\"state active\">kudos given</span></li>");
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
        "<li class=\"refresh\"><form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{id}\">\
         <input type=\"hidden\" name=\"refresh\" value=\"1\">\
         <button type=\"submit\">Refresh from AO3</button></form></li>\
         <li class=\"external\"><a href=\"https://archiveofourown.org/works/{id}\" \
         rel=\"noopener noreferrer\">Open on AO3</a></li>"
    ));
    body.push_str("</ul>");

    // Local reading progress & save place strip
    body.push_str(&format!(
        "<div class=\"reader-status module\">\
         <span class=\"chapter-pos\">Chapter {pos} of {total}</span>\
         <span class=\"reading-progress\" data-work-id=\"{id}\" data-chapter=\"{current}\" \
         data-total-chapters=\"{chapter_total}\" data-initial-pct=\"{overall}\">\
         <progress max=\"100\" value=\"{overall}\"></progress>\
         <span class=\"reading-pct\">{overall}% read</span></span>\
         <form class=\"save inline\" method=\"post\" action=\"/progress\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{id}\">\
         <input type=\"hidden\" name=\"chapter\" value=\"{current}\">\
         <input type=\"hidden\" name=\"pct\" value=\"{inside_pct}\">\
         <button type=\"submit\">Save this chapter as my place</button></form></div>",
        pos = if chapter.is_some() { position + 1 } else { 0 },
    ));

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
        &work.tags.warnings,
    ));
    meta_rows.push_str(&meta_row("category", "Category:", &work.tags.categories));
    meta_rows.push_str(&meta_row("fandom", "Fandom:", &work.tags.fandoms));
    meta_rows.push_str(&meta_row(
        "relationship",
        "Relationship:",
        &work.tags.relationships,
    ));
    meta_rows.push_str(&meta_row("character", "Character:", &work.tags.characters));
    meta_rows.push_str(&meta_row(
        "freeform",
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

    // Work skin + preface + chapters (otwarchive/app/views/works/show.html.erb)
    body.push_str("<div id=\"work-skin\" class=\"wrapper\"><div id=\"workskin\">");
    body.push_str(&format!(
        "<div class=\"preface group\">\
         <h2 class=\"title heading\">{}</h2>\
         <h3 class=\"byline heading\">by {}</h3>",
        escape(&work.title),
        authors
    ));
    if let Some(summary) = &work.summary
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
            let chapter_heading = chapter_heading_text(index + 1, candidate.title.as_deref());
            body.push_str(&format!(
                "<div class=\"chapter\">\
                 <div class=\"chapter preface group\"><h3 class=\"title\">{chapter_heading}</h3></div>\
                 <iframe class=\"chapter\" sandbox=\"allow-scripts\" \
                 referrerpolicy=\"no-referrer\" data-work-id=\"{id}\" \
                 data-chapter=\"{chapter_idx}\" data-total-chapters=\"{chapter_total}\" \
                 title=\"Chapter {number} of {total}\" \
                 src=\"/read/{id}/chapter/{chapter_idx}?v={fetched_at}&skin={skin}&resume={resume}\">\
                 </iframe></div>",
                number = index + 1,
                total = total,
                chapter_idx = candidate.idx,
                fetched_at = work.fetched_at,
                skin = escape(&page.skin),
                resume = resume_pct.filter(|_| candidate.idx == current).unwrap_or(0)
            ));
        }
    } else {
        match chapter {
            Some(chapter) => {
                let chapter_heading =
                    chapter_heading_text(position + 1, chapter.title.as_deref());
                let resume = resume_pct.filter(|_| chapter.idx == current).unwrap_or(0);
                body.push_str(&format!(
                    "<div class=\"chapter\">\
                     <div class=\"chapter preface group\"><h3 class=\"title\">{chapter_heading}</h3></div>\
                     <iframe class=\"chapter\" sandbox=\"allow-same-origin\" \
                     referrerpolicy=\"no-referrer\" data-work-id=\"{id}\" \
                     data-chapter=\"{chapter_idx}\" data-total-chapters=\"{chapter_total}\" \
                     data-resume=\"{resume}\" title=\"Chapter {number} of {total}\" \
                     src=\"/read/{id}/chapter/{chapter_idx}?v={fetched_at}&skin={skin}\">\
                     </iframe></div>",
                    number = position + 1,
                    total = total,
                    chapter_idx = chapter.idx,
                    fetched_at = work.fetched_at,
                    skin = escape(&page.skin)
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

    // Feedback & comments (otwarchive/app/views/comments/_commentable.html.erb)
    body.push_str("<div id=\"feedback\" class=\"feedback\">");
    body.push_str("<h3 class=\"landmark heading\">Actions</h3><ul class=\"actions\">");
    body.push_str("<li><a href=\"#top\">↑ Top</a></li>");
    if position > 0 {
        let previous = chapters[position - 1].idx;
        body.push_str(&format!(
            "<li><a href=\"/read/{id}?chapter={previous}#workskin\">← Previous Chapter</a></li>"
        ));
    }
    if position + 1 < total {
        let next = chapters[position + 1].idx;
        body.push_str(&format!(
            "<li><a href=\"/read/{id}?chapter={next}#workskin\">Next Chapter →</a></li>"
        ));
    }
    if can_write && !kudos_sent {
        body.push_str(&format!(
            "<li><a href=\"/write/preview?kind=kudos&id={id}\">Give kudos</a></li>"
        ));
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

fn chapter_heading_text(number: usize, title: Option<&str>) -> String {
    match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => format!("Chapter {number}: {}", escape(title)),
        None => format!("Chapter {number}"),
    }
}

/// Standalone document for the chapter iframe. Served with its own CSP.
///
/// Iframes are separate documents: they do not inherit the parent page's
/// palette, so the reader passes the active skin explicitly. Without this the
/// chapter text would stay dark on a Reversi background.
pub fn chapter_document(work_title: &str, chapter: &StoredChapter, skin: &str) -> String {
    let skin = match skin {
        "auto" => "auto",
        "reversi" | "midnight" => "reversi",
        "sepia" => "sepia",
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
         :root[data-skin=\"reversi\"]{{color-scheme:dark;}} \
         body{{margin:0;padding:0.5rem 1.5rem 2rem;color:#2a2a2a;background:transparent;\
         font:1.05rem/1.65 \"Lucida Grande\",\"Lucida Sans Unicode\",Verdana,Helvetica,sans-serif;}} \
         .userstuff p{{margin:1.15em auto;}} \
         .userstuff hr{{width:33%;margin:1.25em auto;border:0;border-bottom:2px solid #999;}} \
         a{{color:#900;text-decoration:none;border-bottom:1px solid;}} \
         img{{max-width:100%;height:auto;}} \
         :root[data-skin=\"sepia\"] body{{color:#3d2f21;}} \
         :root[data-skin=\"sepia\"] a{{color:#8a5a33;}} \
         :root[data-skin=\"reversi\"] body{{color:#eee;}} \
         :root[data-skin=\"reversi\"] a{{color:#5998d6;}} \
         :root[data-skin=\"reversi\"] .notes,\
         :root[data-skin=\"reversi\"] blockquote{{color:#b8b8b8;border-color:#555;}} \
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
fn meta_row(kind: &str, label: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
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

    #[test]
    fn chapter_document_includes_sanitized_body_and_notes() {
        let chapter = StoredChapter {
            idx: 1,
            title: Some("Chapter <One>".into()),
            body_html: "<p>body</p><script>bad()</script>".into(),
            notes_html: Some("<p>note</p>".into()),
        };
        let doc = chapter_document("Work <One>", &chapter, "default");
        assert!(doc.contains("Work &lt;One&gt;"));
        assert!(
            !doc.contains("<h3 class=\"chapter-title\""),
            "preface owns the title"
        );
        assert!(doc.contains("<p>body</p>"));
        assert!(!doc.contains("bad()"));
        assert!(doc.contains("Chapter notes"));
        assert!(doc.contains("data-skin=\"default\""));

        let dark = chapter_document("w", &chapter, "reversi");
        assert!(dark.contains("data-skin=\"reversi\""));
        assert!(dark.contains(":root[data-skin=\"reversi\"] body{color:#eee;}"));
        assert_eq!(
            chapter_document("w", &chapter, "neon"),
            chapter_document("w", &chapter, "default"),
            "unknown skins fall back to the light document"
        );
    }
}
