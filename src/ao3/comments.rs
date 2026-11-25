//! Parser for AO3's rendered comment threads (`comments/_commentable.html.erb`).
//!
//! Every selector below is derived from the pinned otwarchive clone
//! (`452391b`, read-only copy in `otwarchive/`), not guessed:
//!
//! | field | clone evidence |
//! | --- | --- |
//! | commentable wrapper | `app/views/comments/_commentable.html.erb:1` — `#feedback.feedback` |
//! | rendered list | `_commentable.html.erb:143-152` — `#comments_placeholder > ol.thread` |
//! | one comment | `app/views/comments/_single_comment.html.erb:2` — `li.comment#comment_<id>`, `h4.byline`, `.posted`, `div.icon`, `blockquote.userstuff`, `p.edited` |
//! | replies | `_comment_thread.html.erb:6-19` — the parent comment `<li>`, then a wrapper `<li>` holding the nested `ol.thread` |
//! | displayed total | `_commentable.html.erb:100` + `app/helpers/comments_helper.rb:102-131` — `#show_comments_link` text `Comments (N)` |
//! | pagination | `_commentable.html.erb:146,149` — `will_paginate` → `ol.pagination.actions` |
//!
//! Contract:
//! - `#feedback` missing means we are not looking at a commentable page (adult
//!   gate, login wall, layout change): [`Error::UpstreamChanged`]. Callers must
//!   keep the previous cache in that case.
//! - A page with no `#comments_placeholder` / no thread is a valid *empty*
//!   commentable (comments disabled, or zero comments), not an error.
//! - Deleted/hidden/unreviewed comments render no `blockquote.userstuff`; they
//!   are dropped together with their replies' parent link (their replies are
//!   hoisted to the deleted comment's parent, matching what AO3 still shows).
//! - `body_html` is untrusted author HTML and stays unsanitized here; the
//!   render layer sanitizes it exactly like chapter prose.

use std::sync::OnceLock;

use scraper::node::Node;
use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::{collapse_whitespace, parse_selector as css};
use crate::error::{Error, Result};

/// One cached comment, exactly as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Document order across the whole commentable (AO3's rendering order).
    pub seq: i64,
    /// AO3 comment id (`comment_<id>`); the reader anchors with it.
    pub comment_id: String,
    /// Parent comment id for replies; `None` for a top-level comment.
    pub parent_id: Option<String>,
    /// Nesting level in the rendered thread (0 = top level).
    pub depth: i64,
    /// Displayed name: pseud name, guest name or "Anonymous Creator".
    pub author: Option<String>,
    /// `/users/<login>` segment when the commenter has an account.
    pub author_login: Option<String>,
    /// AO3's displayed timestamp, e.g. `01 Nov 2026 10:00 UTC`.
    pub posted_at: Option<String>,
    /// AO3's "Last Edited ..." line, when present.
    pub edited_at: Option<String>,
    /// Inner HTML of `blockquote.userstuff` (unsanitized).
    pub body_html: String,
}

/// One fetched commentable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedComments {
    pub comments: Vec<Comment>,
    /// AO3's visible-comment total for this commentable, when displayed.
    pub total: Option<u64>,
    /// The upstream thread is paginated: only this page is cached.
    pub has_more: bool,
}

/// Parses an AO3 page fetched with `?show_comments=true`.
pub fn parse_comments_page(html: &str) -> Result<ParsedComments> {
    let document = Html::parse_document(html);
    let selectors = selectors()?;
    if first_in_document(&document, &selectors.feedback).is_none() {
        return Err(Error::UpstreamChanged(
            "comment page marker missing (#feedback)".to_string(),
        ));
    }

    let placeholder = first_in_document(&document, &selectors.placeholder);
    let mut comments = Vec::new();
    if let Some(placeholder) = placeholder
        && let Some(thread) = placeholder
            .children()
            .filter_map(ElementRef::wrap)
            .find(|child| is_element(child, "ol") && child.value().classes().any(|c| c == "thread"))
    {
        let mut seq = 0;
        walk_thread(thread, None, 0, &mut comments, &mut seq, selectors);
    }

    let has_more =
        placeholder.is_some_and(|element| element.select(&selectors.pagination).next().is_some());
    Ok(ParsedComments {
        comments,
        total: parse_total(&document, selectors),
        has_more,
    })
}

/// Walks one rendered `ol.thread`, preserving AO3's document order.
///
/// AO3 renders a comment as `<li class="comment">` and its replies as a sibling
/// `<li><ol class="thread">…</ol></li>` directly after it, so the parent of a
/// nested thread is the preceding comment at the same level.
fn walk_thread(
    thread: ElementRef<'_>,
    parent_id: Option<String>,
    depth: i64,
    out: &mut Vec<Comment>,
    seq: &mut i64,
    selectors: &Selectors,
) {
    // The last rendered child at this level: (comment id, whether it was stored).
    let mut last: Option<(String, bool)> = None;
    for child in thread.children().filter_map(ElementRef::wrap) {
        if !is_element(&child, "li") {
            continue;
        }
        if child.value().classes().any(|class| class == "comment") {
            let Some(comment_id) = child
                .value()
                .attr("id")
                .and_then(|id| id.strip_prefix("comment_"))
                .map(str::to_string)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let parsed = parse_comment(
                child,
                &comment_id,
                parent_id.clone(),
                depth,
                *seq,
                selectors,
            );
            if let Some(comment) = parsed {
                *seq += 1;
                out.push(comment);
                last = Some((comment_id, true));
            } else {
                // Deleted/hidden/unreviewed: AO3 still renders its replies, but
                // without a visible parent; hoist them to the level above.
                last = Some((comment_id, false));
            }
            continue;
        }
        // A wrapper `<li>` around a nested thread.
        let Some(nested) = child.select(&selectors.thread).next().or_else(|| {
            child
                .children()
                .filter_map(ElementRef::wrap)
                .find(|candidate| is_element(candidate, "ol"))
        }) else {
            continue;
        };
        let reply_parent = match &last {
            Some((id, true)) => Some(id.clone()),
            Some((_, false)) | None => parent_id.clone(),
        };
        walk_thread(nested, reply_parent, depth + 1, out, seq, selectors);
    }
}

fn parse_comment(
    li: ElementRef<'_>,
    comment_id: &str,
    parent_id: Option<String>,
    depth: i64,
    seq: i64,
    selectors: &Selectors,
) -> Option<Comment> {
    // A comment without a body block is deleted, hidden or under review.
    let body = li.select(&selectors.body).next()?;
    let byline = li.select(&selectors.byline).next();
    let (author, author_login) = byline.map_or((None, None), |byline| {
        let (author, login) = commenter(byline, selectors);
        (author, login)
    });
    let posted_at = byline
        .and_then(|byline| byline.select(&selectors.posted).next())
        .map(text_of)
        .filter(|text| !text.is_empty());
    let edited_at = li
        .select(&selectors.edited)
        .next()
        .map(text_of)
        .filter(|text| !text.is_empty());

    Some(Comment {
        seq,
        comment_id: comment_id.to_string(),
        parent_id,
        depth,
        author,
        author_login,
        posted_at,
        edited_at,
        body_html: body.inner_html(),
    })
}

/// Commenter display name and login from `h4.byline`.
///
/// AO3 renders a pseud link for registered users (`helper get_commenter_pseud_or_name`)
/// and plain text for guests or anonymous creators.
fn commenter(byline: ElementRef<'_>, selectors: &Selectors) -> (Option<String>, Option<String>) {
    if let Some(link) = byline.select(&selectors.user_link).next() {
        let name = text_of(link);
        let login = link.value().attr("href").and_then(login_from_user_href);
        let author = (!name.is_empty()).then_some(name);
        return (author, login);
    }
    let plain = byline.children().find_map(|node| {
        if let Node::Text(text) = node.value() {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        } else {
            None
        }
    });
    (plain, None)
}

/// `/users/<login>[/pseuds/<pseud>]` → `<login>`.
fn login_from_user_href(href: &str) -> Option<String> {
    let rest = href.trim().strip_prefix("/users/")?;
    let login = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    (!login.is_empty()).then(|| login.to_string())
}

/// AO3's displayed visible-comment total from `#show_comments_link`.
///
/// With `?show_comments=true` the label reads `Hide Comments (57)`; a work with
/// no comments renders a bare `Comments` link instead.
fn parse_total(document: &Html, selectors: &Selectors) -> Option<u64> {
    let element = first_in_document(document, &selectors.comments_link)?;
    let text = text_of(element);
    let start = text.find('(')? + 1;
    let digits: String = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn text_of(element: ElementRef<'_>) -> String {
    collapse_whitespace(&element.text().collect::<Vec<_>>().join(" "))
}

fn is_element(element: &ElementRef<'_>, name: &str) -> bool {
    element.value().name() == name
}

fn first_in_document<'a>(document: &'a Html, selector: &Selector) -> Option<ElementRef<'a>> {
    document.select(selector).next()
}

struct Selectors {
    feedback: Selector,
    placeholder: Selector,
    thread: Selector,
    body: Selector,
    byline: Selector,
    user_link: Selector,
    posted: Selector,
    edited: Selector,
    comments_link: Selector,
    pagination: Selector,
}

fn selectors() -> Result<&'static Selectors> {
    static CACHE: OnceLock<std::result::Result<Selectors, String>> = OnceLock::new();
    let compiled = CACHE.get_or_init(|| build_selectors().map_err(|err| err.to_string()));
    compiled
        .as_ref()
        .map_err(|msg| Error::UpstreamChanged(msg.clone()))
}

fn build_selectors() -> Result<Selectors> {
    Ok(Selectors {
        feedback: css("#feedback")?,
        placeholder: css("#comments_placeholder")?,
        thread: css("ol.thread")?,
        body: css("blockquote.userstuff")?,
        byline: css("h4.byline")?,
        user_link: css(r#"h4.byline a[href^="/users/"]"#)?,
        posted: css("span.posted")?,
        edited: css("p.edited")?,
        comments_link: css("#show_comments_link, #show_comments_link_top")?,
        pagination: css("ol.pagination")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../tests/fixtures/work_comments.html");

    #[test]
    fn parses_threads_replies_totals_and_pagination() {
        let parsed = parse_comments_page(PAGE).expect("fixture parses");

        assert_eq!(parsed.total, Some(4), "reads AO3's displayed comment count");
        assert!(parsed.has_more, "pagination marks more pages upstream");
        let ids: Vec<&str> = parsed
            .comments
            .iter()
            .map(|comment| comment.comment_id.as_str())
            .collect();
        assert_eq!(ids, ["9001", "9002", "9004", "9005"]);

        let first = &parsed.comments[0];
        assert_eq!(first.seq, 0);
        assert_eq!(first.parent_id, None);
        assert_eq!(first.depth, 0);
        assert_eq!(first.author.as_deref(), Some("reader_one"));
        assert_eq!(first.author_login.as_deref(), Some("reader_one"));
        assert_eq!(first.posted_at.as_deref(), Some("01 Nov 2026 10:00 UTC"));
        assert!(
            first.body_html.contains("<em>markup</em>"),
            "creator markup survives: {}",
            first.body_html
        );
        assert!(
            first.body_html.contains("<script>bad()</script>"),
            "the parser stores raw author HTML (the renderer sanitizes): {}",
            first.body_html
        );

        let reply = &parsed.comments[1];
        assert_eq!(reply.comment_id, "9002");
        assert_eq!(reply.parent_id.as_deref(), Some("9001"));
        assert_eq!(reply.depth, 1);
        assert_eq!(reply.author.as_deref(), Some("guest_name"));
        assert_eq!(reply.author_login, None, "guests have no login");
        assert_eq!(
            reply.edited_at.as_deref(),
            Some("Last Edited 03 Nov 2026 08:00 UTC")
        );

        // 9003 is a deleted comment: dropped. Its reply (9004) is hoisted to
        // the deleted comment's own level, not lost.
        let hoisted = &parsed.comments[2];
        assert_eq!(hoisted.comment_id, "9004");
        assert_eq!(hoisted.parent_id, None);
        assert_eq!(hoisted.author.as_deref(), Some("third_user"));

        let last = &parsed.comments[3];
        assert_eq!(last.comment_id, "9005");
        assert_eq!(last.seq, 3);
        assert_eq!(last.depth, 0);
    }

    #[test]
    fn a_commentable_without_comments_is_an_empty_result() {
        let html = r##"<div id="feedback" class="feedback">
            <p class="notice">Sorry, this work doesn&#39;t allow comments.</p>
            <div id="comments_placeholder" style="display:none;"></div>
        </div>"##;
        let parsed = parse_comments_page(html).expect("empty commentable");
        assert!(parsed.comments.is_empty());
        assert_eq!(parsed.total, None);
        assert!(!parsed.has_more);
    }

    #[test]
    fn a_non_commentable_page_fails_closed() {
        let error = parse_comments_page("<html><body><p>adult gate</p></body></html>")
            .expect_err("not a commentable page");
        assert!(
            matches!(error, Error::UpstreamChanged(_)),
            "typed as markup drift: {error:?}"
        );
    }

    #[test]
    fn logged_out_comments_link_without_a_count_yields_no_total() {
        let html = r##"<div id="feedback" class="feedback">
            <ul class="actions"><li class="comments" id="show_comments_link_top">
              <a href="#comments">Comments</a></li></ul>
            <div id="comments_placeholder"><ol class="thread"></ol></div>
        </div>"##;
        let parsed = parse_comments_page(html).expect("commentable");
        assert!(parsed.comments.is_empty());
        assert_eq!(parsed.total, None);
    }
}
