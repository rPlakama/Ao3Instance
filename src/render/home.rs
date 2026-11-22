//! AO3-shaped start page: favorites on the left, local news on the right and
//! the personal shortcuts at the bottom.

use super::blurb::author_links_from;
use super::chrome::{PageContext, layout};
use super::{escape, tag_href};
use crate::store::{StoredWork, WorkSummary};
use crate::time::format_utc;

pub fn home_page(
    ctx: &PageContext,
    works: &[WorkSummary],
    detailed: &[StoredWork],
    fandoms: &[(String, usize)],
    flash: Option<&str>,
) -> String {
    let mut body = String::new();
    if let Some(flash) = flash {
        body.push_str(&format!(
            "<div class=\"flash notice\">{}</div>",
            escape(flash)
        ));
    }

    body.push_str("<div class=\"home-grid\">");
    body.push_str("<section class=\"home-favorites\">");
    body.push_str("<h2>Find your favorites</h2>");
    if fandoms.is_empty() {
        body.push_str(
            "<p class=\"muted\">No cached fandom yet. Open a work below and the cache \
             starts building itself.</p>",
        );
    } else {
        body.push_str("<ul class=\"favorites\">");
        for (fandom, count) in fandoms.iter().take(8) {
            body.push_str(&format!(
                "<li><a href=\"{}\">{}</a> <span class=\"meta\">({count})</span></li>",
                tag_href(fandom),
                escape(fandom)
            ));
        }
        body.push_str("</ul>");
        body.push_str("<p class=\"meta\"><a href=\"/fandoms\">All fandoms →</a></p>");
    }

    let continuing: Vec<&WorkSummary> = works
        .iter()
        .filter(|work| work.progress_chapter.is_some())
        .take(4)
        .collect();
    if !continuing.is_empty() {
        body.push_str(
            "<h3 class=\"home-subheading\">Continue Reading</h3><ul class=\"favorites\">",
        );
        for work in continuing {
            let chapter = work.progress_chapter.unwrap_or(1);
            body.push_str(&format!(
                "<li><a href=\"/read/{id}?chapter={chapter}\">{title}</a> \
                 <span class=\"meta\">· by {authors} · continue ch. {chapter}</span></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                authors = author_links_from(&work.authors),
            ));
        }
        body.push_str("</ul>");
    }

    body.push_str("<div id=\"open-work\" class=\"module open-work\"><h3>Open a work</h3>");
    body.push_str(
        "<form class=\"open\" method=\"post\" action=\"/open\">\
         <input name=\"id\" placeholder=\"AO3 work id or URL (e.g. 12345678)\" autofocus required>\
         <button type=\"submit\">Open work</button></form>\
         <p class=\"meta\">One user-initiated fetch, cached for offline reading.</p></div>",
    );
    body.push_str("</section>");

    body.push_str(
        "<section class=\"home-news\"><h2>News <span class=\"heading-link\">\
                   <a href=\"/works\">All Cached Works</a></span></h2>",
    );
    if detailed.is_empty() {
        body.push_str(
            "<p class=\"muted\">Nothing cached yet. This instance's news column fills \
             with the works you add.</p>",
        );
    } else {
        for work in detailed.iter().take(3) {
            body.push_str("<article class=\"news-entry\">");
            body.push_str(&format!(
                "<h3><a href=\"/read/{id}\">{title}</a></h3>\
                 <p class=\"meta\">Cached: {date} · Chapters: {chapters} \
                 · Comments: {comments} · Kudos: {kudos}</p>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                date = escape(&format_utc(work.fetched_at)),
                chapters = work.chapter_count,
                comments = work.comment_count.unwrap_or(0),
                kudos = work.kudos.unwrap_or(0),
            ));
            if let Some(summary) = &work.summary {
                let excerpt = plain_text(summary, 260);
                if !excerpt.is_empty() {
                    body.push_str(&format!("<p>{}</p>", escape(&excerpt)));
                }
            }
            body.push_str(&format!(
                "<p class=\"read-more\"><a href=\"/read/{}\">Read more...</a></p></article>",
                escape(&work.ao3_id)
            ));
        }
    }
    body.push_str("</section></div>");

    body.push_str("<div class=\"home-bottom\">");
    body.push_str(
        "<section class=\"home-module\"><h3>Is it later already?</h3>\
         <p class=\"meta\"><a href=\"/reading\">My History</a></p></section>",
    );
    body.push_str(
        "<section class=\"home-module\"><h3>Unread messages</h3>\
         <p class=\"meta\"><a href=\"/subscriptions\">My Subscriptions</a></p></section>",
    );
    body.push_str("</div>");

    layout(ctx, &body)
}

/// First `max` characters of an HTML summary's text, without markup.
fn plain_text(html: &str, max: usize) -> String {
    let document = scraper::Html::parse_fragment(html);
    let text: String = document.root_element().text().collect::<Vec<_>>().join(" ");
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let mut out: String = collapsed.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
