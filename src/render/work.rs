//! Work and reader page, plus the standalone document served to the sandboxed
//! chapter iframe.
//!
//! Author HTML only ever reaches a page through [`sanitize`]; the chapter body
//! gets its own document with `CHAPTER_CSP`, never inline in the app page.

use super::blurb::author_links;
use super::chrome::{PageContext, checked_label, layout};
use super::{escape, sanitize};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{StoredChapter, StoredWork};
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
    let authors = author_links(work);
    let mut body = String::from("<a id=\"top\"></a>");
    if let Some(notice) = notice {
        body.push_str(&format!("<div class=\"notice\">{}</div>", escape(notice)));
    }
    body.push_str(&format!(
        "<div class=\"preface group\"><h2 class=\"title heading\">{}</h2>\
         <h3 class=\"byline heading\">by {}</h3></div>",
        escape(&work.title),
        authors
    ));

    let mut meta_rows = String::new();
    if let Some(rating) = &work.rating {
        meta_rows.push_str(&format!(
            "<dt>Rating:</dt><dd><a class=\"tag\" href=\"/works?rating={}\">{}</a></dd>",
            percent_encode(rating, SpaceMode::Percent20),
            escape(rating)
        ));
    }
    meta_rows.push_str(&meta_row("Archive Warning:", &work.tags.warnings));
    meta_rows.push_str(&meta_row("Category:", &work.tags.categories));
    meta_rows.push_str(&meta_row("Fandom:", &work.tags.fandoms));
    meta_rows.push_str(&meta_row("Relationship:", &work.tags.relationships));
    meta_rows.push_str(&meta_row("Character:", &work.tags.characters));
    meta_rows.push_str(&meta_row("Additional Tags:", &work.tags.additional));
    if let Some(language) = &work.language {
        meta_rows.push_str(&format!("<dt>Language:</dt><dd>{}</dd>", escape(language)));
    }
    if let Some(words) = work.word_count {
        meta_rows.push_str(&format!("<dt>Words:</dt><dd>{words}</dd>"));
    }
    meta_rows.push_str(&format!(
        "<dt>Chapters:</dt><dd>{}</dd>",
        work.chapter_count
    ));
    if let Some(comments) = work.comment_count {
        meta_rows.push_str(&format!("<dt>Comments:</dt><dd>{comments}</dd>"));
    }
    if let Some(kudos) = work.kudos {
        meta_rows.push_str(&format!("<dt>Kudos:</dt><dd>{kudos}</dd>"));
    }
    meta_rows.push_str(&format!(
        "<dt>Checked:</dt><dd>{}</dd>",
        escape(&checked_label(work.fetched_at))
    ));
    body.push_str(&format!("<dl class=\"work meta group\">{meta_rows}</dl>"));

    if let Some(summary) = &work.summary
        && !summary.trim().is_empty()
    {
        body.push_str(&format!(
            "<div class=\"summary module\"><h3 class=\"heading\">Summary:</h3>\
             <blockquote class=\"userstuff\">{}</blockquote></div>",
            sanitize(summary)
        ));
    }

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

    if let Some(saved) = progress
        && saved.chapter_idx != current
    {
        body.push_str(&format!(
            "<div class=\"continue\">You were at chapter {chapter}. \
             <a href=\"/read/{}?chapter={chapter}\">Resume</a></div>",
            escape(&work.ao3_id),
            chapter = saved.chapter_idx
        ));
    }

    let mut toolbar = String::from("<div class=\"actions toolbar\">");
    toolbar.push_str(&format!(
        "<span>Chapter {} of {}</span>",
        if chapter.is_some() { position + 1 } else { 0 },
        total
    ));
    toolbar.push_str(&format!(
        "<span class=\"reading-progress\" data-work-id=\"{id}\" data-chapter=\"{current}\" \
         data-total-chapters=\"{chapter_total}\" data-initial-pct=\"{overall}\">\
         <progress max=\"100\" value=\"{overall}\"></progress>\
         <span class=\"reading-pct\">{overall}% read</span></span>",
        id = escape(&work.ao3_id),
    ));
    if position > 0 {
        let previous = chapters[position - 1].idx;
        toolbar.push_str(&format!(
            "<a href=\"/read/{}?chapter={previous}\">← previous</a>",
            escape(&work.ao3_id)
        ));
    }
    if position + 1 < total {
        let next = chapters[position + 1].idx;
        toolbar.push_str(&format!(
            "<a href=\"/read/{}?chapter={next}\">next →</a>",
            escape(&work.ao3_id)
        ));
    }
    let mut options = String::new();
    for candidate in chapters {
        options.push_str(&format!(
            "<option value=\"{}\"{}>Chapter {}</option>",
            candidate.idx,
            if candidate.idx == current {
                " selected"
            } else {
                ""
            },
            candidate.idx
        ));
    }
    toolbar.push_str(&format!(
        "<form class=\"inline\" method=\"get\" action=\"/read/{id}\">\
         <label class=\"meta\">Chapter <select name=\"chapter\">{options}</select></label>\
         <button type=\"submit\">Go</button></form>",
        id = escape(&work.ao3_id)
    ));
    if full_work {
        toolbar.push_str(&format!(
            "<a href=\"/read/{id}?chapter={current}\">Chapter by Chapter</a>",
            id = escape(&work.ao3_id)
        ));
    } else if !chapters.is_empty() {
        toolbar.push_str(&format!(
            "<a href=\"/read/{id}?view_full_work=true\">Entire Work</a>",
            id = escape(&work.ao3_id)
        ));
    }
    toolbar.push_str(&format!(
        "<a href=\"https://archiveofourown.org/works/{id}\" rel=\"noopener noreferrer\">\
         open on AO3</a>",
        id = escape(&work.ao3_id)
    ));
    if can_write {
        if subscribed {
            toolbar.push_str(&format!(
                "<span class=\"state active\">subscribed</span>\
                 <form class=\"inline\" method=\"post\" action=\"/write/unsubscribe\">\
                 <input type=\"hidden\" name=\"kind\" value=\"Work\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{}\">\
                 <button type=\"submit\">Unsubscribe</button></form>",
                escape(&work.ao3_id)
            ));
        } else {
            toolbar.push_str(&format!(
                "<a href=\"/write/preview?kind=work&id={}\">Subscribe to work</a>",
                escape(&work.ao3_id)
            ));
        }
    } else {
        toolbar.push_str("<a href=\"/session\">Sign in to write</a>");
    }
    if can_write {
        if kudos_sent {
            toolbar.push_str("<span class=\"state active\">kudos given</span>");
        } else {
            toolbar.push_str(&format!(
                "<a href=\"/write/preview?kind=kudos&id={}\">Give kudos</a>",
                escape(&work.ao3_id)
            ));
        }
        if bookmarked {
            toolbar.push_str("<span class=\"state active\">bookmarked</span>");
        } else {
            toolbar.push_str(&format!(
                "<a href=\"/write/preview?kind=bookmark&id={}\">Bookmark</a>",
                escape(&work.ao3_id)
            ));
        }
        toolbar.push_str(&format!(
            "<form class=\"inline\" method=\"post\" action=\"/write/reading\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{}\">\
             <input type=\"hidden\" name=\"action\" value=\"later\">\
             <button type=\"submit\">Mark for later</button></form>\
             <form class=\"inline\" method=\"post\" action=\"/write/reading\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{}\">\
             <input type=\"hidden\" name=\"action\" value=\"read\">\
             <button type=\"submit\">Mark as read</button></form>",
            escape(&work.ao3_id),
            escape(&work.ao3_id)
        ));
        if let Some(state) = reading_state {
            let label = if state == "read" {
                "marked read"
            } else {
                "marked for later"
            };
            toolbar.push_str(&format!("<span class=\"state active\">{label}</span>"));
        }
    }
    toolbar.push_str("</div>");
    body.push_str(&toolbar);
    body.push_str(&format!(
        "<form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{}\">\
         <input type=\"hidden\" name=\"refresh\" value=\"1\">\
         <button type=\"submit\">Refresh from AO3</button></form>",
        escape(&work.ao3_id)
    ));
    body.push_str(&format!(
        "<form class=\"save\" method=\"post\" action=\"/progress\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{}\">\
         <input type=\"hidden\" name=\"chapter\" value=\"{}\">\
         <input type=\"hidden\" name=\"pct\" value=\"{}\">\
         <button type=\"submit\">Save this chapter as my place</button></form>",
        escape(&work.ao3_id),
        current,
        inside_pct
    ));

    if full_work && !chapters.is_empty() {
        for (index, candidate) in chapters.iter().enumerate() {
            body.push_str(&format!(
                "<div class=\"chapter preface group\"><h3 class=\"title\">Chapter {number}: {title}</h3></div>\
                 <iframe class=\"chapter\" sandbox=\"allow-scripts\" \
                 referrerpolicy=\"no-referrer\" data-work-id=\"{id}\" \
                 data-chapter=\"{chapter_idx}\" data-total-chapters=\"{chapter_total}\" \
                 title=\"Chapter {number} of {total}\" \
                 src=\"/read/{id}/chapter/{chapter_idx}?v={fetched_at}&skin={skin}&resume={resume}\"></iframe>",
                number = index + 1,
                title = escape(candidate.title.as_deref().unwrap_or("")),
                total = total,
                id = escape(&work.ao3_id),
                chapter_idx = candidate.idx,
                fetched_at = work.fetched_at,
                skin = escape(&page.skin),
                resume = resume_pct.filter(|_| candidate.idx == current).unwrap_or(0)
            ));
        }
    } else {
        match chapter {
            Some(chapter) => {
                body.push_str(&format!(
                    "<div class=\"chapter preface group\"><h3 class=\"title\">Chapter {number}: {title}</h3></div>",
                    number = position + 1,
                    title = escape(chapter.title.as_deref().unwrap_or(""))
                ));
                let resume = resume_pct.filter(|_| chapter.idx == current).unwrap_or(0);
                body.push_str(&format!(
                    "<iframe class=\"chapter\" sandbox=\"allow-same-origin\" \
                     referrerpolicy=\"no-referrer\" data-work-id=\"{id}\" \
                     data-chapter=\"{chapter_idx}\" data-total-chapters=\"{chapter_total}\" \
                     data-resume=\"{resume}\" title=\"Chapter {number} of {total}\" \
                     src=\"/read/{id}/chapter/{chapter_idx}?v={fetched_at}&skin={skin}\">\
                     </iframe>",
                    number = position + 1,
                    total = total,
                    id = escape(&work.ao3_id),
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

    body.push_str("<div id=\"feedback\"><div class=\"actions\">");
    body.push_str("<a href=\"#top\">↑ Top</a>");
    if position > 0 {
        let previous = chapters[position - 1].idx;
        body.push_str(&format!(
            "<a href=\"/read/{}?chapter={previous}\">← previous</a>",
            escape(&work.ao3_id)
        ));
    }
    if position + 1 < total {
        let next = chapters[position + 1].idx;
        body.push_str(&format!(
            "<a href=\"/read/{}?chapter={next}\">next →</a>",
            escape(&work.ao3_id)
        ));
    }
    if can_write && !kudos_sent {
        body.push_str(&format!(
            "<a href=\"/write/preview?kind=kudos&id={}\">Give kudos</a>",
            escape(&work.ao3_id)
        ));
    }
    body.push_str("</div>");
    if can_write {
        let commenter = comment_as.map_or_else(|| "your AO3 pseud".to_string(), escape);
        body.push_str(&format!(
            "<details class=\"comment\"><summary>Comment on this work</summary>\
             <form class=\"stack\" method=\"post\" action=\"/write/comment/preview\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
             <p class=\"meta\">Comment as <strong>{commenter}</strong> · it will be public on AO3</p>\
             <label>Comment (max 10000 characters)\
             <textarea name=\"content\" rows=\"6\" maxlength=\"10000\" required></textarea></label>\
             <button type=\"submit\">Preview comment</button></form></details>",
            id = escape(&work.ao3_id)
        ));
    }
    body.push_str("</div>");

    body.push_str("<script src=\"/assets/reader.js\" defer></script>");
    layout(page, &format!("<div id=\"workskin\">{}</div>", body))
}

/// Standalone document for the chapter iframe. Served with its own CSP.
///
/// Iframes are separate documents: they do not inherit the parent page's
/// palette, so the reader passes the active skin explicitly. Without this the
/// chapter text would stay dark on a Reversi background.
pub fn chapter_document(work_title: &str, chapter: &StoredChapter, skin: &str) -> String {
    let skin = match skin {
        "reversi" => "reversi",
        _ => "default",
    };
    let mut body = String::new();
    body.push_str(&sanitize(&chapter.body_html));
    if let Some(notes) = &chapter.notes_html
        && !notes.trim().is_empty()
    {
        body.push_str(&format!(
            "<hr class=\"notes-sep\"><div class=\"notes\"><strong>Chapter notes</strong>{}</div>",
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
         body{{margin:0;padding:0 1.25rem;color:#2a2a2a;background:transparent;\
         font:16px/1.7 Georgia,serif;}} \
         a{{color:#8c2f39;}} img{{max-width:100%;height:auto;}} \
         :root[data-skin=\"reversi\"] body{{color:#eee;}} \
         :root[data-skin=\"reversi\"] a{{color:#ff9b9b;}} \
         :root[data-skin=\"reversi\"] .notes,\
         :root[data-skin=\"reversi\"] blockquote{{color:#b8b8b8;border-color:#555;}} \
         @media (prefers-color-scheme: dark) {{\
         :root:not([data-skin=\"default\"]) body{{color:#eee;}} \
         :root:not([data-skin=\"default\"]) a{{color:#ff9b9b;}} \
         :root:not([data-skin=\"default\"]) .notes,\
         :root:not([data-skin=\"default\"]) blockquote{{color:#b8b8b8;border-color:#555;}}}} \
         .notes-sep{{border:0;border-top:1px solid #ccc4;margin:1.5rem 0 1rem;}} \
         .notes{{font-size:.92rem;color:#8a857e;}} \
         blockquote{{border-left:3px solid #ccc4;margin:1rem 0;padding-left:1rem;color:#8a857e;}}</style>\
         </head><body>{body}</body></html>",
        escape(work_title)
    )
}

/// One AO3 metadata row: label plus comma-joined tag chips (or `—`).
fn meta_row(label: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        return format!("<dt>{label}</dt><dd>—</dd>");
    }
    let joined = tags
        .iter()
        .map(|tag| {
            format!(
                "<a class=\"tag\" href=\"{}\">{}</a>",
                super::tag_href(tag),
                escape(tag)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("<dt>{label}</dt><dd>{joined}</dd>")
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
