//! Inline chapter block rendering and standalone sanitized chapter document.

use crate::consts::is_builtin_skin;
use crate::render::{escape, sanitize};
use crate::store::StoredChapter;

/// Renders one `ammonia`-sanitized chapter directly inline inside `#chapters`
/// (matching `otwarchive/app/views/chapters/_chapter.html.erb`).
pub(super) fn render_chapter_inline(
    work_id: &str,
    chapter: &StoredChapter,
    number: usize,
    total: usize,
    chapter_total: i64,
    resume_pct: Option<f64>,
    is_first_rendered: bool,
) -> String {
    let resume_exact = resume_pct
        .filter(|pct| pct.is_finite())
        .map_or(0.0, |pct| pct.clamp(0.0, 100.0));
    let resume_int = resume_exact.round() as i64;
    let resume_target_attr = if resume_pct.is_some() && (resume_exact > 0.0 || !is_first_rendered) {
        " data-resume-target=\"1\""
    } else {
        ""
    };
    let has_title = chapter
        .title
        .as_deref()
        .is_some_and(|title| !title.trim().is_empty());
    let show_heading = total > 1 || has_title;

    let mut out = format!(
        "<div class=\"chapter\" id=\"chapter-{idx}\" data-work-id=\"{work_id}\" \
         data-chapter=\"{idx}\" data-total-chapters=\"{chapter_total}\" \
         data-resume=\"{resume_int}\" data-resume-exact=\"{resume_exact:.2}\"{resume_target_attr}>",
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
    let skin = if is_builtin_skin(skin) {
        skin
    } else {
        "default"
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
