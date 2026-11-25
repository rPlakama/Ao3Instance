//! AO3 commentable: the comment form, the cached thread and the load/refresh
//! states.
//!
//! Structure follows the pinned otwarchive clone:
//! `app/views/comments/_commentable.html.erb` (section order),
//! `_comment_form.html.erb` (fieldset, "Comment as", textarea, submit),
//! `_comment_thread.html.erb` (`ol.thread` + nested replies) and
//! `_single_comment.html.erb` (`li.comment`, `h4.byline`, `.posted`, `.icon`,
//! `blockquote.userstuff`, `.edited`).
//!
//! Author HTML is sanitized here, exactly like chapter prose. The thread is
//! rendered from the local cache; a fetch only ever happens through an explicit
//! POST to `/read/<id>/comments`.

use std::collections::{HashMap, HashSet};

use crate::ao3::comments::Comment;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::render::components::flash_notice;
use crate::render::{escape, sanitize};
use crate::store::StoredWork;
use crate::time::{format_count, format_utc};

use super::ReadContext;
use super::nav::load_comments_form;

/// Defensive cap: AO3 stops nesting at `COMMENT_THREAD_MAX_DEPTH` (5), so a
/// deeper tree means drifted markup rather than real content.
const MAX_DEPTH: usize = 8;

pub(super) fn comments_section(work: &StoredWork, ctx: &ReadContext<'_>) -> String {
    let mut out =
        String::from("<h3 class=\"landmark heading\"><a id=\"comments\">Comments</a></h3>");
    out.push_str(&flash_notice(ctx.comment_notice));

    if ctx.can_write {
        out.push_str(&comment_form(work, ctx));
    } else {
        out.push_str(
            "<p class=\"notice\">Sign in to AO3 on the <a href=\"/session\">Session page</a> \
             to comment.</p>",
        );
    }

    if ctx.show_comments {
        if ctx.comments.is_empty() {
            if ctx.comment_set.is_some() {
                out.push_str("<p class=\"notice\">No comments on AO3 for this work yet.</p>");
                // A cached zero-comment thread must still be refreshable: new
                // comments can appear upstream after the first fetch.
                out.push_str("<div class=\"comments-refresh\">");
                out.push_str(&load_comments_form(work, ctx, "Refresh comments"));
                out.push_str("</div>");
            } else {
                out.push_str(&format!(
                    "<p class=\"notice\">Comments are not cached yet. {}</p>",
                    load_comments_form(work, ctx, "Load comments from AO3")
                ));
            }
        } else {
            out.push_str(&thread_html(ctx.comments));
            out.push_str(&thread_footer(work, ctx));
        }
    } else if ctx.comment_set.is_some() {
        // The `Comments (N)` action in the actions list expands the cache; the
        // section stays quiet until it is expanded (AO3's hidden placeholder).
        out.push_str(
            "<p class=\"meta comments-cache\">The cached comment thread is hidden. \
             Use the Comments action above.</p>",
        );
    } else {
        out.push_str(
            "<p class=\"meta comments-cache\">Comments for this work are not cached yet.</p>",
        );
    }
    out
}

/// AO3 `comments/_comment_form.html.erb` shape, posting to our confirmation
/// step (the comment text is shown again before anything reaches AO3).
fn comment_form(work: &StoredWork, ctx: &ReadContext<'_>) -> String {
    let id = escape(&work.ao3_id);
    let commenter = ctx
        .comment_as
        .map_or_else(|| "your AO3 pseud".to_string(), escape);
    format!(
        "<div id=\"add_comment_placeholder\" title=\"top level comment\">\
         <div id=\"add_comment\"><div class=\"post comment\" id=\"comment_form_for_{id}\">\
         <form id=\"comment_for_{id}\" class=\"new_comment\" method=\"post\" \
         action=\"/write/comment/preview\">\
         <fieldset><legend>Comment</legend>\
         <h4 class=\"heading\">Comment as <span class=\"byline\">{commenter}</span></h4>\
         <p class=\"footnote\">It will be public on AO3. Up to 10,000 characters.</p>\
         <p><label class=\"landmark\" for=\"comment_content\">Comment</label>\
         <textarea id=\"comment_content\" name=\"content\" rows=\"6\" maxlength=\"10000\" \
         required></textarea></p>\
         <p class=\"submit actions\"><button type=\"submit\">Preview Comment</button></p>\
         </fieldset></form></div></div></div>"
    )
}

fn thread_html(comments: &[Comment]) -> String {
    let present: HashSet<&str> = comments
        .iter()
        .map(|comment| comment.comment_id.as_str())
        .collect();
    let mut children: HashMap<&str, Vec<&Comment>> = HashMap::new();
    let mut roots: Vec<&Comment> = Vec::new();
    for comment in comments {
        match comment.parent_id.as_deref() {
            Some(parent) if present.contains(parent) => {
                children.entry(parent).or_default().push(comment);
            }
            _ => roots.push(comment),
        }
    }

    let mut out = String::from("<ol class=\"thread\">");
    for (index, root) in roots.iter().enumerate() {
        render_comment(&mut out, root, index, 0, &children);
    }
    out.push_str("</ol>");
    out
}

fn render_comment(
    out: &mut String,
    comment: &Comment,
    index: usize,
    depth: usize,
    children: &HashMap<&str, Vec<&Comment>>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let parity = if index.is_multiple_of(2) {
        "odd"
    } else {
        "even"
    };
    out.push_str(&format!(
        "<li class=\"{parity} comment group\" id=\"comment_{}\" role=\"article\">",
        escape(&comment.comment_id)
    ));

    out.push_str("<h4 class=\"heading byline\">");
    match (&comment.author, &comment.author_login) {
        // Registered pseuds link like AO3's byline; guest names stay plain text.
        (Some(author), Some(_login)) => out.push_str(&format!(
            "<a href=\"/works?author={}\">{}</a>",
            percent_encode(author, SpaceMode::Percent20),
            escape(author)
        )),
        (Some(author), None) => out.push_str(&escape(author)),
        (None, _) => out.push_str("Anonymous"),
    }
    if let Some(posted) = &comment.posted_at {
        out.push_str(&format!(
            "<span class=\"posted datetime\">{}</span>",
            escape(posted)
        ));
    }
    out.push_str("</h4>");

    // AO3 renders the commenter's avatar here; this instance shows the neutral
    // visitor silhouette instead (no remote avatar fetches from a reader).
    out.push_str("<div class=\"icon\"><span class=\"visitor icon\"></span></div>");
    out.push_str("<blockquote class=\"userstuff\">");
    out.push_str(&sanitize(&comment.body_html));
    out.push_str("</blockquote>");
    if let Some(edited) = &comment.edited_at {
        out.push_str(&format!(
            "<p class=\"edited datetime\">{}</p>",
            escape(edited)
        ));
    }

    if let Some(replies) = children.get(comment.comment_id.as_str()) {
        out.push_str("<ol class=\"thread\">");
        for (reply_index, reply) in replies.iter().enumerate() {
            render_comment(out, reply, reply_index, depth + 1, children);
        }
        out.push_str("</ol>");
    }
    out.push_str("</li>");
}

fn thread_footer(work: &StoredWork, ctx: &ReadContext<'_>) -> String {
    let total = ctx
        .comment_set
        .and_then(|set| set.total)
        .unwrap_or_else(|| u64::try_from(ctx.comments.len()).unwrap_or(0));
    let mut out = format!(
        "<p class=\"meta comments-meta\">{} cached comment{}",
        format_count(total),
        if total == 1 { "" } else { "s" }
    );
    if let Some(set) = ctx.comment_set {
        out.push_str(&format!(" · last checked {}", format_utc(set.fetched_at)));
        if set.has_more
            && let Some(path) = ctx.comment_page_path
        {
            let separator = if path.contains('?') { '&' } else { '?' };
            out.push_str(&format!(
                " · more comments on <a href=\"https://archiveofourown.org{}{separator}page=2\" \
                 rel=\"noopener noreferrer\">AO3</a>",
                escape(path)
            ));
        }
    }
    out.push_str("</p>");
    // Refreshing is explicit: one POST fetches the commentable again.
    out.push_str("<div class=\"comments-refresh\">");
    out.push_str(&load_comments_form(work, ctx, "Refresh comments"));
    out.push_str("</div>");
    out
}
