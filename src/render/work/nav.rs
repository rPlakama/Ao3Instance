//! Work top navigation actions and bottom feedback/comment section.

use crate::render::escape;
use crate::store::StoredChapter;

use super::ReadContext;

/// Top work action bar (`otwarchive/app/views/works/_work_header_navigation.html.erb`).
pub(super) fn top_navigation(
    id: &str,
    chapters: &[StoredChapter],
    position: usize,
    inside_pct: i64,
    ctx: &ReadContext<'_>,
) -> String {
    let total = chapters.len();
    let is_multi_chapter = total > 1;
    let current = ctx.current;
    let mut out = String::from(
        "<h3 class=\"landmark heading\">Actions</h3>\
         <ul class=\"work navigation actions\">",
    );

    if is_multi_chapter {
        if ctx.full_work {
            out.push_str(&format!(
                "<li class=\"chapter bychapter\">\
                 <a href=\"/read/{id}?chapter={current}\">Chapter by Chapter</a></li>"
            ));
        } else {
            out.push_str(&format!(
                "<li class=\"chapter entire\">\
                 <a href=\"/read/{id}?view_full_work=true\">Entire Work</a></li>"
            ));
            if position > 0 {
                let previous = chapters[position - 1].idx;
                out.push_str(&format!(
                    "<li class=\"chapter previous\">\
                     <a href=\"/read/{id}?chapter={previous}#workskin\">← Previous Chapter</a></li>"
                ));
            }
            if position + 1 < total {
                let next = chapters[position + 1].idx;
                out.push_str(&format!(
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
        out.push_str(&format!(
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

    if ctx.can_write {
        out.push_str(&write_controls(id, ctx));
    } else {
        out.push_str("<li><a href=\"/session\">Sign in to write</a></li>");
    }

    let save_pct = ctx.resume_pct.filter(|pct| pct.is_finite()).map_or_else(
        || inside_pct.to_string(),
        |pct| format!("{:.2}", pct.clamp(0.0, 100.0)),
    );
    out.push_str(&format!(
        "<li class=\"save\"><form class=\"inline save-place\" method=\"post\" action=\"/progress\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{id}\">\
         <input type=\"hidden\" name=\"chapter\" value=\"{current}\">\
         <input type=\"hidden\" name=\"pct\" value=\"{save_pct}\">\
         <button type=\"submit\">Save Place</button></form></li>\
         <li class=\"refresh\"><form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{id}\">\
         <input type=\"hidden\" name=\"refresh\" value=\"1\">\
         <button type=\"submit\">Refresh from AO3</button></form></li>\
         <li class=\"external\"><a href=\"https://archiveofourown.org/works/{id}\" \
         rel=\"noopener noreferrer\">Open on AO3</a></li></ul>"
    ));
    out
}

fn write_controls(id: &str, ctx: &ReadContext<'_>) -> String {
    let mut out = String::new();
    if ctx.bookmarked {
        out.push_str("<li class=\"bookmark\"><span class=\"state active\">bookmarked</span></li>");
    } else {
        out.push_str(&format!(
            "<li class=\"bookmark\">\
             <a href=\"/write/preview?kind=bookmark&id={id}\">Bookmark</a></li>"
        ));
    }
    match ctx.reading_state {
        Some("for_later") => {
            out.push_str(&format!(
                "<li class=\"mark\"><span class=\"state active\">marked for later</span></li>\
                 <li class=\"mark\"><form class=\"inline\" method=\"post\" action=\"/write/reading\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                 <input type=\"hidden\" name=\"action\" value=\"read\">\
                 <button type=\"submit\">Mark as read</button></form></li>"
            ));
        }
        Some("read") => {
            out.push_str(&format!(
                "<li class=\"mark\"><span class=\"state active\">marked read</span></li>\
                 <li class=\"mark\"><form class=\"inline\" method=\"post\" action=\"/write/reading\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                 <input type=\"hidden\" name=\"action\" value=\"later\">\
                 <button type=\"submit\">Mark for later</button></form></li>"
            ));
        }
        _ => {
            out.push_str(&format!(
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
    out.push_str("<li class=\"comments\"><a href=\"#feedback\">Comments</a></li>");
    if ctx.subscribed {
        out.push_str(&format!(
            "<li class=\"subscribe\"><span class=\"state active\">subscribed</span></li>\
             <li class=\"subscribe\"><form class=\"inline\" method=\"post\" \
             action=\"/write/unsubscribe\">\
             <input type=\"hidden\" name=\"kind\" value=\"Work\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
             <button type=\"submit\">Unsubscribe</button></form></li>"
        ));
    } else {
        out.push_str(&format!(
            "<li class=\"subscribe\">\
             <a href=\"/write/preview?kind=work&id={id}\">Subscribe to work</a></li>"
        ));
    }
    if ctx.kudos_sent {
        out.push_str("<li class=\"kudos\"><span class=\"state active\">kudos given</span></li>");
    } else {
        out.push_str(&format!(
            "<li class=\"kudos\">\
             <a href=\"/write/preview?kind=kudos&id={id}\">Give kudos</a></li>"
        ));
    }
    out
}

/// Bottom navigation actions & comment box (`otwarchive/app/views/comments/_commentable.html.erb`).
pub(super) fn feedback_section(
    id: &str,
    chapters: &[StoredChapter],
    position: usize,
    ctx: &ReadContext<'_>,
) -> String {
    let total = chapters.len();
    let mut out = String::from(
        "<div id=\"feedback\" class=\"feedback\">\
         <h3 class=\"landmark heading\">Actions</h3><ul class=\"actions\">\
         <li><a href=\"#top\">↑ Top</a></li>",
    );
    if !ctx.full_work && position > 0 {
        let previous = chapters[position - 1].idx;
        out.push_str(&format!(
            "<li><a href=\"/read/{id}?chapter={previous}#workskin\">← Previous Chapter</a></li>"
        ));
    }
    if !ctx.full_work && position + 1 < total {
        let next = chapters[position + 1].idx;
        out.push_str(&format!(
            "<li><a href=\"/read/{id}?chapter={next}#workskin\">Next Chapter →</a></li>"
        ));
    }
    if ctx.can_write {
        if !ctx.kudos_sent {
            out.push_str(&format!(
                "<li><a href=\"/write/preview?kind=kudos&id={id}\">Give kudos</a></li>"
            ));
        }
        if !ctx.bookmarked {
            out.push_str(&format!(
                "<li><a href=\"/write/preview?kind=bookmark&id={id}\">Bookmark</a></li>"
            ));
        }
    }
    out.push_str("</ul>");
    if ctx.can_write {
        let commenter = ctx
            .comment_as
            .map_or_else(|| "your AO3 pseud".to_string(), escape);
        out.push_str(&format!(
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
    out.push_str("</div>");
    out
}
