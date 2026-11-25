//! Work top navigation actions and the AO3 `#feedback` commentable.
//!
//! Markup follows `otwarchive/app/views/works/_work_header_navigation.html.erb`
//! and `app/views/comments/_commentable.html.erb`: actions first, then the kudos
//! module, then the comments section (form + cached thread).
//!
//! Kudos and work subscriptions post directly from the work page the way AO3's
//! own forms do (button click = the deliberate action; the form carries the
//! `confirm=yes` token the server requires). Comments and bookmarks keep the
//! confirmation page (`docs/adr/0005-write-safety.md`).

use crate::render::escape;
use crate::store::{StoredChapter, StoredWork};

use super::ReadContext;
use super::comments::comments_section;

/// Top work action bar (`_work_header_navigation.html.erb`).
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

    // AO3 order: bookmark, mark, comments, subscribe (`_work_header_navigation`).
    out.push_str("<li class=\"comments\"><a href=\"#comments\">Comments</a></li>");
    if ctx.can_write {
        out.push_str(&subscription_form(id, ctx));
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
    out
}

/// Subscription form (`app/views/subscriptions/_form.html.erb`): the submit
/// button is the whole affordance, and it posts directly like AO3's own form.
fn subscription_form(id: &str, ctx: &ReadContext<'_>) -> String {
    if ctx.subscribed {
        format!(
            "<li class=\"subscribe\"><form class=\"ajax-create-destroy\" method=\"post\" \
             action=\"/write/unsubscribe\">\
             <input type=\"hidden\" name=\"kind\" value=\"Work\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
             <button type=\"submit\">Unsubscribe</button></form></li>"
        )
    } else {
        format!(
            "<li class=\"subscribe\"><form class=\"ajax-create-destroy\" method=\"post\" \
             action=\"/write/subscribe\">\
             <input type=\"hidden\" name=\"kind\" value=\"Work\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
             <input type=\"hidden\" name=\"confirm\" value=\"yes\">\
             <button type=\"submit\">Subscribe</button></form></li>"
        )
    }
}

/// Bottom feedback/comment section (`comments/_commentable.html.erb`).
pub(super) fn feedback_section(
    work: &StoredWork,
    chapters: &[StoredChapter],
    position: usize,
    ctx: &ReadContext<'_>,
) -> String {
    let id = escape(&work.ao3_id);
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
        // Kudos (`kudos/_kudos.html.erb` + `_commentable.html.erb` action form).
        if ctx.kudos_sent {
            out.push_str(
                "<li class=\"kudos\"><span class=\"state active\">kudos given</span></li>",
            );
        } else {
            out.push_str(&format!(
                "<li class=\"kudos\"><form class=\"new_kudo\" id=\"new_kudo\" method=\"post\" \
                 action=\"/write/kudos\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                 <input type=\"hidden\" name=\"confirm\" value=\"yes\">\
                 <button type=\"submit\" id=\"kudo_submit\">Kudos \u{2665}</button></form></li>"
            ));
        }
        if !ctx.bookmarked {
            out.push_str(&format!(
                "<li class=\"bookmark\">\
                 <a href=\"/write/preview?kind=bookmark&id={id}\">Bookmark</a></li>"
            ));
        }
        if ctx.reading_state == Some("for_later") {
            out.push_str(&format!(
                "<li class=\"mark\"><form class=\"inline\" method=\"post\" \
                 action=\"/write/reading\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{id}\">\
                 <input type=\"hidden\" name=\"action\" value=\"read\">\
                 <button type=\"submit\">Mark as read</button></form></li>"
            ));
        }
    }
    out.push_str(&comments_action(work, ctx));
    out.push_str("</ul>");
    out.push_str(&kudos_module(work, ctx));
    out.push_str(&comments_section(work, ctx));
    out.push_str("</div>");
    out
}

/// The `Comments` / `Hide Comments` action, or the one-click load form when the
/// thread was never fetched for this commentable.
fn comments_action(work: &StoredWork, ctx: &ReadContext<'_>) -> String {
    let id = escape(&work.ao3_id);
    let chapter = ctx.current;
    let mut href = format!("/read/{id}?chapter={chapter}");
    if ctx.full_work {
        href.push_str("&view_full_work=true");
    }
    if ctx.show_comments {
        format!("<li id=\"show_comments_link\"><a href=\"{href}#comments\">Hide Comments</a></li>")
    } else if let Some(set) = ctx.comment_set {
        let label = set.total.map_or_else(
            || "Comments".to_string(),
            |total| format!("Comments ({})", crate::time::format_count(total)),
        );
        format!(
            "<li id=\"show_comments_link\"><a href=\"{href}&show_comments=1#comments\">{label}</a></li>"
        )
    } else {
        format!(
            "<li id=\"show_comments_link\">{}</li>",
            load_comments_form(work, ctx, "Comments")
        )
    }
}

/// One-click comment fetch form shared by the action, the empty state and the
/// refresh control. Posts the reader's position so the server can resolve the
/// same commentable AO3 shows.
pub(super) fn load_comments_form(work: &StoredWork, ctx: &ReadContext<'_>, button: &str) -> String {
    let id = escape(&work.ao3_id);
    let full_work = if ctx.full_work {
        "<input type=\"hidden\" name=\"full_work\" value=\"true\">"
    } else {
        ""
    };
    format!(
        "<form class=\"inline\" method=\"post\" action=\"/read/{id}/comments\">\
         <input type=\"hidden\" name=\"chapter\" value=\"{}\">{full_work}\
         <button type=\"submit\">{button}</button></form>",
        ctx.current
    )
}

/// Kudos receipt module (`kudos/_kudos.html.erb`): only rendered when there is
/// something to report. Giver lists are not cached, so this is honest about the
/// count rather than inventing names.
fn kudos_module(work: &StoredWork, ctx: &ReadContext<'_>) -> String {
    let count = work.kudos.unwrap_or(0);
    if count == 0 && !ctx.kudos_sent {
        return String::new();
    }
    let text = if ctx.kudos_sent {
        if count > 1 {
            format!(
                "You and {} others left kudos on this work!",
                crate::time::format_count(count - 1)
            )
        } else {
            "You left kudos on this work!".to_string()
        }
    } else if count == 1 {
        "1 person left kudos on this work!".to_string()
    } else {
        format!(
            "{} people left kudos on this work!",
            crate::time::format_count(count)
        )
    };
    format!(
        "<h3 class=\"landmark heading\">Kudos</h3>\
         <div id=\"kudos\"><p class=\"kudos\">{text}</p></div>"
    )
}
