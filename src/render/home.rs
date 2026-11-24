//! AO3-shaped start page modelled on `otwarchive/app/views/home/index.html.erb`:
//! favorites/fandoms, news, read-later readings, and unread inbox modules inside
//! `.splash`, plus the local Continue Reading and Open a work panels.

use super::blurb::author_links_from;
use super::chrome::{PageContext, layout_with_class};
use super::icons::required_tags_from;
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

    body.push_str("<div class=\"splash\">");

    // Module 1: Find your favorites (or cached fandoms browse) + Continue Reading + Open a work
    if !modules.favorites.is_empty() {
        body.push_str("<div class=\"favorite module\">");
        body.push_str("<h3 class=\"heading\">Find your favorites</h3><ul class=\"favorites\">");
        for favorite in modules.favorites.iter().take(10) {
            body.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                tag_href(favorite),
                escape(favorite)
            ));
        }
        body.push_str("</ul>");
    } else {
        body.push_str("<div class=\"browse module\" role=\"navigation\" aria-label=\"Fandoms\">");
        body.push_str(
            "<h3 class=\"heading\"><span class=\"title\">Find your favorites</span> \
             <span class=\"link\"><a href=\"/fandoms\">All Fandoms</a></span></h3>",
        );
        if fandoms.is_empty() {
            body.push_str(
                "<p class=\"note\">Sign in on <a href=\"/session\">Session</a> to load your \
                 AO3 favorite tags, or open a work below to start the local cache.</p>",
            );
        } else {
            body.push_str("<ul class=\"favorites\">");
            for (fandom, count) in fandoms.iter().take(10) {
                body.push_str(&format!(
                    "<li><a href=\"{}\">{} ({count})</a></li>",
                    tag_href(fandom),
                    escape(fandom)
                ));
            }
            body.push_str("</ul>");
        }
    }

    let continuing: Vec<&WorkSummary> = works
        .iter()
        .filter(|work| work.progress_chapter.is_some())
        .take(4)
        .collect();
    if !continuing.is_empty() {
        body.push_str(
            "<div class=\"continue-module\"><h4 class=\"heading\">Continue Reading</h4>\
             <ul class=\"continue-list\">",
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
        body.push_str("</ul></div>");
    }

    body.push_str(
        "<div id=\"open-work\" class=\"account module open-work\">\
         <h4 class=\"heading\">Open a work</h4>\
         <form class=\"open\" method=\"post\" action=\"/open\">\
         <input name=\"id\" placeholder=\"AO3 work id or URL (e.g. 12345678)\" autofocus required>\
         <button type=\"submit\">Open work</button></form>\
         <p class=\"note\">One user-initiated fetch, cached for offline reading.</p></div>",
    );
    body.push_str("</div>");

    // Module 2: News
    let (news_href, news_label) = if modules.news.is_empty() {
        ("/works".to_string(), "All Cached Works")
    } else {
        (
            "https://archiveofourown.org/admin_posts".to_string(),
            "All News",
        )
    };
    body.push_str(&format!(
        "<div id=\"news\" class=\"latest news module\"><h3 class=\"heading\">\
         <span class=\"title\">News</span> \
         <span class=\"link\"><a href=\"{news_href}\">{news_label}</a></span></h3>"
    ));
    if !modules.news.is_empty() {
        body.push_str("<ul class=\"news index group\">");
        for post in modules.news.iter().take(4) {
            let href = post.href.as_deref().map_or_else(
                || "https://archiveofourown.org/admin_posts".to_string(),
                |href| format!("https://archiveofourown.org{href}"),
            );
            body.push_str(&format!(
                "<li class=\"post group\"><div class=\"header module\">\
                 <h4 class=\"heading\"><a href=\"{href}\">{title}</a></h4>\
                 <p class=\"meta\">{meta}</p></div>",
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
                "<p class=\"jump\"><a href=\"{href}\">Read more...</a></p></li>"
            ));
        }
        body.push_str("</ul>");
    } else if detailed.is_empty() {
        body.push_str(
            "<p class=\"note\">Sign in on <a href=\"/session\">Session</a> to load AO3 \
             news, or cache works to see recent updates here.</p>",
        );
    } else {
        body.push_str("<ul class=\"news index group\">");
        for work in detailed.iter().take(3) {
            body.push_str(&format!(
                "<li class=\"post group\"><div class=\"header module\">\
                 <h4 class=\"heading\"><a href=\"/read/{id}\">{title}</a></h4>\
                 <p class=\"meta\">Cached: {date} · Chapters: {chapters} \
                 · Comments: {comments} · Kudos: {kudos}</p></div>",
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
                    body.push_str(&format!(
                        "<blockquote class=\"userstuff\"><p>{}</p></blockquote>",
                        escape(&excerpt)
                    ));
                }
            }
            body.push_str(&format!(
                "<p class=\"jump\"><a href=\"/read/{}\">Read more...</a></p></li>",
                escape(&work.ao3_id)
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str("</div>");

    // Module 3: Is it later already? (Marked for later readings)
    body.push_str(
        "<div class=\"random readings module\"><h3 class=\"heading\">\
         <span class=\"title\">Is it later already?</span> \
         <span class=\"link\"><a href=\"/reading\">My History</a></span></h3>\
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
    body.push_str("</div>");

    // Module 4: Unread messages (Inbox)
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
        "<div class=\"latest messages module\"><h3 class=\"heading\">\
         <span class=\"title\">Unread messages</span> \
         <span class=\"link\"><a href=\"{inbox_href}\">My Inbox</a></span></h3>\
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
                "<li class=\"unread comment group\"><h4 class=\"heading byline\">{byline}</h4>\
                 <blockquote class=\"userstuff\">{content}</blockquote>\
                 <ul class=\"actions\"><li><a href=\"{href}\" rel=\"noopener noreferrer\">\
                 Open on AO3</a></li></ul></li>",
                byline = escape(&message.byline),
                content = sanitize(&message.content_html),
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str("</div>");

    body.push_str("</div>"); // .splash
    body.push_str(
        "<p class=\"meta home-refresh\"><a href=\"/?refresh=1\">Refresh AO3 home</a></p>",
    );

    layout_with_class(ctx, "home", &body)
}

fn reading_blurb(reading: &crate::ao3::home::HomeReading) -> String {
    let work = &reading.work;
    let authors = author_links_from(&work.authors);
    let fandoms = if work.fandoms.is_empty() {
        "—".to_string()
    } else {
        work.fandoms
            .iter()
            .map(|fandom| {
                format!(
                    "<a class=\"tag\" href=\"{}\">{}</a>",
                    tag_href(fandom),
                    escape(fandom)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let required = required_tags_from(
        work.rating.as_deref(),
        &work.warnings,
        &work.categories,
        work.complete,
        false,
    );
    let tag_links = |tags: &[String], class: &str| -> String {
        tags.iter()
            .map(|tag| {
                format!(
                    "<li class=\"{class}\"><a class=\"tag\" href=\"{}\">{}</a></li>",
                    tag_href(tag),
                    escape(tag)
                )
            })
            .collect()
    };
    let tags_html = format!(
        "{}{}{}{}",
        tag_links(&work.warnings, "warnings"),
        tag_links(&work.relationships, "relationships"),
        tag_links(&work.characters, "characters"),
        tag_links(&work.additional_tags, "freeforms"),
    );
    let tags_block = if tags_html.is_empty() {
        String::new()
    } else {
        format!(
            "<h6 class=\"landmark heading\">Tags</h6><ul class=\"tags commas\">{tags_html}</ul>"
        )
    };
    let summary = work
        .summary_html
        .as_deref()
        .map_or_else(String::new, |html| {
            format!(
                "<h6 class=\"landmark heading\">Summary</h6>\
                 <blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(html)
            )
        });
    let mut stats = String::new();
    if let Some(words) = work.words {
        stats.push_str(&format!(
            "<dt class=\"words\">Words:</dt><dd class=\"words\">{}</dd>",
            crate::time::format_count(words)
        ));
    }
    if let Some(chapters) = &work.chapters {
        stats.push_str(&format!(
            "<dt class=\"chapters\">Chapters:</dt><dd class=\"chapters\">{}</dd>",
            escape(chapters)
        ));
    }
    if let Some(kudos) = work.kudos {
        stats.push_str(&format!(
            "<dt class=\"kudos\">Kudos:</dt><dd class=\"kudos\">{}</dd>",
            crate::time::format_count(kudos)
        ));
    }
    if let Some(hits) = work.hits {
        stats.push_str(&format!(
            "<dt class=\"hits\">Hits:</dt><dd class=\"hits\">{}</dd>",
            crate::time::format_count(hits)
        ));
    }
    let stats_html = if stats.is_empty() {
        String::new()
    } else {
        format!("<dl class=\"stats\">{stats}</dl>")
    };
    format!(
        "<li id=\"work_{id}\" class=\"reading work blurb group\" role=\"article\">\
         <div class=\"header module\">\
         <h4 class=\"heading\"><a href=\"/open/{id}\">{title}</a> by {authors}</h4>\
         <h5 class=\"fandoms heading\"><span class=\"landmark\">Fandoms:</span> {fandoms}</h5>\
         {required}<p class=\"datetime\">{datetime}</p></div>\
         {tags_block}{summary}{stats_html}\
         <div class=\"user module group\"><h4 class=\"viewed heading\">{viewed}</h4></div></li>",
        id = escape(&work.ao3_id),
        title = escape(&work.title),
        datetime = escape(&work.datetime),
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
