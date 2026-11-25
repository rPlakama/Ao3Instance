//! The signed-in author's works manager: AO3's `/users/:login/works` and
//! `/users/:login/works/drafts` lists, plus the publish/delete actions.
//!
//! Layout follows `otwarchive/app/views/works/index.html.erb` and
//! `works/drafts.html.erb`: heading, tabs, caution notice on drafts, the
//! `ol.work.index.group` blurb list, and numbered pagination. Posted works link
//! into the local reader (cached) or the one-click cache route; drafts offer
//! Post Draft / Delete Draft through the confirmation pages.

use std::collections::HashSet;

use super::chrome::{PageContext, layout_with_class};
use super::components::cache_banner;
use super::{escape, pagination, sanitize};
use crate::ao3::account_lists::{MyWorkEntry, MyWorksPage};
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::time::format_count;

/// One page of the author's works (or drafts).
#[allow(
    clippy::too_many_arguments,
    reason = "server-rendered page with independent state fields"
)]
pub fn my_works_page(
    ctx: &PageContext,
    page: Option<&MyWorksPage>,
    drafts: bool,
    current_page: i64,
    cached_works: &HashSet<String>,
    cache_meta: Option<(bool, i64)>,
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let base = if drafts {
        "/my/works/drafts"
    } else {
        "/my/works"
    };
    let heading = if drafts { "My Drafts" } else { "My Works" };
    let mut body = format!("<h2 class=\"heading\">{heading}</h2>");
    body.push_str(&super::components::flash_notice(notice));
    body.push_str(&super::components::flash_error(error));

    // Tabs (`works/index.html.erb:20-27` user role subnav).
    body.push_str(&format!(
        "<ul class=\"navigation actions\" role=\"navigation\">{new_work}{works}{drafts}</ul>",
        new_work = if ctx.write_enabled {
            "<li><a href=\"/works/new\">New Work</a></li>".to_string()
        } else {
            String::new()
        },
        works = if drafts {
            "<li><a href=\"/my/works\">Works</a></li>".to_string()
        } else {
            "<li class=\"active\"><span class=\"current\">Works</span></li>".to_string()
        },
        drafts = if drafts {
            "<li class=\"active\"><span class=\"current\">Drafts</span></li>".to_string()
        } else {
            "<li><a href=\"/my/works/drafts\">Drafts</a></li>".to_string()
        },
    ));
    let refresh_href = format!("{base}?page={current_page}&refresh=1");
    body.push_str(&format!(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"{refresh_href}\">Refresh from AO3</a></li></ul>"
    ));
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));
    if drafts {
        body.push_str(
            "<p class=\"caution notice\"><strong>Please note:</strong> unposted drafts are \
             deleted by the archive about 29 days after they are created. Post or delete them \
             from here, or manage them on AO3.</p>",
        );
    }

    let Some(page) = page else {
        body.push_str(
            "<p class=\"notice\">Could not load this list from AO3. It may be a temporary \
             connection or session problem; try <a href=\"/session\">the Session page</a> or \
             refresh. Your cached library is unaffected.</p>",
        );
        body.push_str(&format!(
            "<p><a href=\"{base}?refresh=1\">Try again</a></p>"
        ));
        return layout_with_class(ctx, "works-index", &body);
    };

    let total = page.total_pages.saturating_mul(20);
    if page.items.is_empty() {
        body.push_str(if drafts {
            "<p class=\"muted\">No drafts on AO3.</p>"
        } else {
            "<p class=\"muted\">No posted works on AO3 yet. <a href=\"/works/new\">Post one?</a></p>"
        });
    } else {
        body.push_str(&pagination(current_page, 20, total, base, ""));
        body.push_str("<h3 class=\"landmark heading\">Listing Works</h3>");
        body.push_str("<ol class=\"work index group\">");
        for entry in &page.items {
            body.push_str(&work_row(entry, cached_works, ctx.write_enabled));
        }
        body.push_str("</ol>");
        body.push_str(&pagination(current_page, 20, total, base, ""));
    }
    layout_with_class(ctx, "works-index", &body)
}

fn work_row(entry: &MyWorkEntry, cached_works: &HashSet<String>, can_write: bool) -> String {
    let work = &entry.work;
    let id = escape(&work.ao3_id);
    let fandoms = work
        .fandoms
        .iter()
        .map(|fandom| {
            format!(
                "<a class=\"tag\" href=\"/tags/{}/works\">{}</a>",
                percent_encode(fandom, SpaceMode::Percent20),
                escape(fandom)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut stats = String::new();
    if let Some(words) = work.words {
        stats.push_str(&format!(
            "<dt class=\"words\">Words:</dt><dd class=\"words\">{}</dd>",
            format_count(words)
        ));
    }
    if let Some(chapters) = &work.chapters {
        stats.push_str(&format!(
            "<dt class=\"chapters\">Chapters:</dt><dd class=\"chapters\">{}</dd>",
            escape(chapters)
        ));
    }

    let mut actions = String::from("<ul class=\"actions\">");
    if entry.draft {
        if can_write {
            actions.push_str(&format!(
                "<li><a href=\"/write/preview?kind=work-publish&id={id}\">Post Draft</a></li>\
                 <li><a href=\"/write/preview?kind=work-delete&amp;id={id}&amp;return=drafts\">\
                 Delete Draft</a></li>"
            ));
        } else {
            actions.push_str("<li><span class=\"state imported\">writes disabled</span></li>");
        }
    } else {
        let local = if cached_works.contains(&work.ao3_id) {
            format!("/read/{id}")
        } else {
            format!("/open/{id}")
        };
        if !work.ao3_id.is_empty() {
            actions.push_str(&format!(
                "<li><a href=\"{local}\">Open locally</a></li>\
                 <li><a href=\"https://archiveofourown.org/works/{id}\" \
                 rel=\"noopener noreferrer\">Open on AO3</a></li>\
                 <li><a href=\"/my/works/{id}/chapters\">Chapters</a></li>"
            ));
            if can_write && work.complete != Some(true) {
                actions.push_str(&format!(
                    "<li><a href=\"/write/chapter/new?work_id={id}\">Add Chapter</a></li>"
                ));
            }
        }
        if can_write {
            actions.push_str(&format!(
                "<li><a href=\"/write/preview?kind=work-delete&id={id}\">Delete This Work</a></li>"
            ));
        }
    }
    actions.push_str("</ul>");

    let summary = work
        .summary_html
        .as_deref()
        .map_or_else(String::new, |html| {
            format!(
                "<blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(html)
            )
        });
    format!(
        "<li id=\"work_{id}\" class=\"own work blurb group\" role=\"article\">\
         <div class=\"header module\"><h4 class=\"heading\">{title}{draft_status}</h4>\
         <h5 class=\"fandoms heading\">{fandoms}</h5>\
         <p class=\"datetime\">{datetime}</p></div>{summary}\
         <dl class=\"stats\">{stats}</dl>{actions}</li>",
        title = if entry.draft {
            escape(&work.title)
        } else {
            format!("<a href=\"/open/{id}\">{}</a>", escape(&work.title))
        },
        draft_status = if entry.draft {
            " <span class=\"status\">(Draft)</span>"
        } else {
            ""
        },
        fandoms = fandoms,
        datetime = escape(&work.datetime),
        summary = summary,
        stats = stats,
        actions = actions,
    )
}
