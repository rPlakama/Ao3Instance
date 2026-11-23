//! AO3-shaped start page: favorites on the left, local news on the right and
//! the personal shortcuts at the bottom.

use super::blurb::author_links_from;
use super::chrome::{PageContext, layout};
use super::{escape, sanitize, tag_href};
use crate::ao3::home::HomeModules;
use crate::store::{StoredWork, WorkSummary};
use crate::time::format_utc;

pub fn home_page(
    ctx: &PageContext,
    works: &[WorkSummary],
    detailed: &[StoredWork],
    fandoms: &[(String, usize)],
    flash: Option<&str>,
    modules: &HomeModules,
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
    if !modules.favorites.is_empty() {
        body.push_str("<ul class=\"favorites\">");
        for favorite in modules.favorites.iter().take(10) {
            body.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                tag_href(favorite),
                escape(favorite)
            ));
        }
        body.push_str("</ul>");
    } else if fandoms.is_empty() {
        body.push_str(
            "<p class=\"muted\">Sign in on <a href=\"/session\">Session</a> to load your \
             AO3 favorite tags, or open a work below to start the cache.</p>",
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
            let pct =
                super::overall_percent(chapter, work.progress_pct.unwrap_or(0), work.chapter_count);
            body.push_str(&format!(
                "<li><a href=\"/read/{id}?chapter={chapter}\">{title}</a> \
                 <span class=\"meta\">· by {authors} · continue ch. {chapter} · {pct}%</span></li>",
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

    let (news_href, news_label) = if modules.news.is_empty() {
        ("/works".to_string(), "All Cached Works")
    } else {
        (
            "https://archiveofourown.org/admin_posts".to_string(),
            "All News",
        )
    };
    body.push_str(&format!(
        "<section class=\"home-news\"><h2>News <span class=\"heading-link\">\
         <a href=\"{news_href}\">{news_label}</a></span></h2>"
    ));
    if !modules.news.is_empty() {
        for post in modules.news.iter().take(4) {
            let href = post.href.as_deref().map_or_else(
                || "https://archiveofourown.org/admin_posts".to_string(),
                |href| format!("https://archiveofourown.org{href}"),
            );
            body.push_str(&format!(
                "<article class=\"news-entry\"><h3><a href=\"{href}\">{title}</a></h3>\
                 <p class=\"meta\">{meta}</p>",
                title = escape(&post.title),
                meta = escape(&post.meta),
            ));
            if !post.summary_html.trim().is_empty() {
                body.push_str(&format!(
                    "<blockquote class=\"userstuff\">{}</blockquote>",
                    sanitize(&post.summary_html)
                ));
            }
            body.push_str(&format!(
                "<p class=\"read-more\"><a href=\"{href}\">Read more...</a></p></article>"
            ));
        }
    } else if detailed.is_empty() {
        body.push_str(
            "<p class=\"muted\">Sign in on <a href=\"/session\">Session</a> to load AO3 \
             news, or cache works below.</p>",
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
        "<section class=\"home-module home-readings\"><h3>Is it later already? \
         <span class=\"heading-link\"><a href=\"/reading\">My History</a></span></h3>\
         <p class=\"note\">Some works you've marked for later.</p>",
    );
    if modules.readings.is_empty() {
        body.push_str("<p class=\"muted\">No AO3 read-later works loaded.</p>");
    } else {
        body.push_str("<ol class=\"reading work index group\">");
        for reading in modules.readings.iter().take(4) {
            body.push_str(&reading_blurb(reading));
        }
        body.push_str("</ol>");
    }
    body.push_str("</section>");
    let inbox_href = if ctx.account.is_empty() {
        "/session".to_string()
    } else {
        format!(
            "https://archiveofourown.org/users/{}/inbox",
            super::super::ao3::parse::percent_encode(
                &ctx.account,
                super::super::ao3::parse::SpaceMode::Percent20
            )
        )
    };
    body.push_str(&format!(
        "<section class=\"home-module home-inbox\"><h3>Unread messages \
         <span class=\"heading-link\"><a href=\"{inbox_href}\">My Inbox</a></span></h3>\
         <p class=\"note\">The latest unread items from your inbox.</p>"
    ));
    if modules.inbox.is_empty() {
        body.push_str(
            "<p class=\"muted\">No unread messages loaded. Sign in to AO3 to see them.</p>",
        );
    } else {
        body.push_str("<ul class=\"abbreviated comment group\">");
        for message in modules.inbox.iter().take(4) {
            let href = message.href.as_deref().map_or_else(
                || "https://archiveofourown.org".to_string(),
                |href| format!("https://archiveofourown.org{href}"),
            );
            body.push_str(&format!(
                "<li class=\"unread\"><h4 class=\"heading byline\">{byline}</h4>\
                 <blockquote class=\"userstuff\">{content}</blockquote>\
                 <p class=\"actions\"><a href=\"{href}\">Open on AO3</a></p></li>",
                byline = escape(&message.byline),
                content = sanitize(&message.content_html),
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str("</section></div>");
    body.push_str(
        "<p class=\"meta home-refresh\"><a href=\"/?refresh=1\">Refresh AO3 home</a></p>",
    );

    layout(ctx, &body)
}

fn reading_blurb(reading: &crate::ao3::home::HomeReading) -> String {
    let work = &reading.work;
    let authors = author_links_from(&work.authors);
    let fandoms = work
        .fandoms
        .iter()
        .map(|fandom| {
            format!(
                "<a class=\"tag\" href=\"{}\">{}</a>",
                tag_href(fandom),
                escape(fandom)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let summary = work
        .summary_html
        .as_deref()
        .map_or_else(String::new, |html| {
            format!(
                "<blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(html)
            )
        });
    let mut stats = String::new();
    if let Some(words) = work.words {
        stats.push_str(&format!("Words: {} · ", crate::time::format_count(words)));
    }
    if let Some(chapters) = &work.chapters {
        stats.push_str(&format!("Chapters: {} · ", escape(chapters)));
    }
    if let Some(kudos) = work.kudos {
        stats.push_str(&format!("Kudos: {} · ", crate::time::format_count(kudos)));
    }
    if let Some(hits) = work.hits {
        stats.push_str(&format!("Hits: {}", crate::time::format_count(hits)));
    }
    format!(
        "<li class=\"reading work\"><h4 class=\"heading\">\
         <a href=\"/open/{id}\">{title}</a> by {authors}</h4>\
         <p class=\"meta\">{fandoms}</p>{summary}\
         <p class=\"stats\">{stats}</p>\
         <p class=\"meta viewed\">{viewed}</p></li>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        viewed = escape(&reading.meta),
    )
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
