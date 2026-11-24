//! Parser for `GET /users/:login/bookmarks`.

use scraper::{Html, Selector};

use crate::ao3::html::collapse_text;
use crate::ao3::search::{LiveWork, parse_total_from_heading, parse_work_element};

use super::parse_max_page;

/// One entry from `GET /users/:login/bookmarks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBookmark {
    pub bookmark_id: Option<i64>,
    pub work: Option<LiveWork>,
    pub fallback_title: Option<String>,
    pub fallback_href: Option<String>,
    pub bookmarked_at: String,
    pub tags: Vec<String>,
    pub notes_html: Option<String>,
    pub is_private: bool,
    pub is_rec: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveBookmarksPage {
    pub total: usize,
    pub total_pages: i64,
    pub items: Vec<LiveBookmark>,
}

/// Parses `GET /users/:login/bookmarks`. Returns `None` if the HTML does not
/// look like an AO3 bookmarks page.
pub fn parse_bookmarks(html: &str) -> Option<LiveBookmarksPage> {
    if !html.contains("bookmark index") && !html.contains("Bookmarks") {
        return None;
    }
    let doc = Html::parse_document(html);
    let total_pages = parse_max_page(&doc);
    let Ok(li_sel) = Selector::parse("ol.bookmark.index.group > li.bookmark.blurb.group") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(user_sel) = Selector::parse("div.user.module.group") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(dt_sel) = Selector::parse("p.datetime") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(tag_sel) = Selector::parse("ul.meta.tags a.tag") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(notes_sel) = Selector::parse("blockquote.userstuff.notes") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(private_sel) = Selector::parse("p.status span.private") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(rec_sel) = Selector::parse("p.status span.rec") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(heading_link_sel) = Selector::parse("h4.heading a") else {
        return Some(LiveBookmarksPage::default());
    };
    let Ok(deleted_sel) = Selector::parse("p.message") else {
        return Some(LiveBookmarksPage::default());
    };

    let mut items = Vec::new();
    for li in doc.select(&li_sel) {
        let bookmark_id = li
            .value()
            .attr("id")
            .and_then(|id| id.strip_prefix("bookmark_"))
            .and_then(|digits| digits.parse::<i64>().ok());
        let work = parse_work_element(&li).ok();
        let (fallback_title, fallback_href) = if work.is_some() {
            (None, None)
        } else if let Some(link) = li.select(&heading_link_sel).next() {
            (
                Some(collapse_text(&link)),
                link.value().attr("href").map(str::to_string),
            )
        } else {
            let msg = li
                .select(&deleted_sel)
                .next()
                .map(|node| collapse_text(&node))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "Deleted or unavailable bookmark".to_string());
            (Some(msg), None)
        };

        let user_mod = li.select(&user_sel).next();
        let bookmarked_at = user_mod
            .as_ref()
            .and_then(|m| m.select(&dt_sel).next())
            .map(|node| collapse_text(&node))
            .unwrap_or_default();
        let tags = user_mod
            .as_ref()
            .map(|m| {
                m.select(&tag_sel)
                    .map(|node| collapse_text(&node))
                    .filter(|tag| !tag.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let notes_html = user_mod
            .as_ref()
            .and_then(|m| m.select(&notes_sel).next())
            .map(|node| node.inner_html().trim().to_string())
            .filter(|html| !html.is_empty());
        let is_private = li.select(&private_sel).next().is_some();
        let is_rec = li.select(&rec_sel).next().is_some();

        items.push(LiveBookmark {
            bookmark_id,
            work,
            fallback_title,
            fallback_href,
            bookmarked_at,
            tags,
            notes_html,
            is_private,
            is_rec,
        });
    }

    let total = Selector::parse("h2.heading")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|node| parse_total_from_heading(&collapse_text(&node)))
        .map(|value| value as usize)
        .unwrap_or(items.len());

    Some(LiveBookmarksPage {
        total,
        total_pages,
        items,
    })
}
