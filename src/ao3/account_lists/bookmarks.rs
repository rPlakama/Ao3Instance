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

/// The editable fields of one bookmark, from `GET /bookmarks/:id/edit`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookmarkEditFields {
    pub notes: String,
    pub tags: String,
    pub collections: String,
    pub private: bool,
    pub rec: bool,
    /// Title of the bookmarked work (for confirmation headings).
    pub bookmarkable_title: String,
    /// AO3 work id when the bookmarkable is a work.
    pub work_id: Option<String>,
}

/// Parses `GET /bookmarks/:id/edit` (owner-only `bookmarks/edit.html.erb`).
pub fn parse_bookmark_editor(html: &str) -> Option<BookmarkEditFields> {
    let doc = Html::parse_document(html);
    let form_sel = Selector::parse("form[action^='/bookmarks/']").ok()?;
    let form = doc.select(&form_sel).next()?;
    let heading_link = Selector::parse("h2.heading a[href]").ok()?;
    let (bookmarkable_title, work_id) = doc
        .select(&heading_link)
        .next()
        .map(|link| {
            let href = link.value().attr("href").unwrap_or_default();
            let work_id = href
                .split('/')
                .skip_while(|segment| *segment != "works")
                .nth(1)
                .map(str::to_string);
            (collapse_text(&link), work_id)
        })
        .unwrap_or_default();
    let textarea = |name: &str| -> String {
        let Ok(selector) = Selector::parse(&format!("textarea[name='{name}']")) else {
            return String::new();
        };
        form.select(&selector)
            .next()
            .map(|node| node.text().collect::<String>())
            .unwrap_or_default()
    };
    let input = |name: &str| -> String {
        let Ok(selector) = Selector::parse(&format!("input[name='{name}']")) else {
            return String::new();
        };
        form.select(&selector)
            .next()
            .and_then(|node| node.value().attr("value"))
            .unwrap_or_default()
            .to_string()
    };
    let checked = |name: &str| -> bool {
        let Ok(selector) =
            Selector::parse(&format!("input[type='checkbox'][name='{name}'][checked]"))
        else {
            return false;
        };
        form.select(&selector).next().is_some()
    };
    Some(BookmarkEditFields {
        notes: textarea("bookmark[bookmarker_notes]"),
        tags: input("bookmark[tag_string]"),
        collections: input("bookmark[collection_names]"),
        private: checked("bookmark[private]"),
        rec: checked("bookmark[rec]"),
        bookmarkable_title,
        work_id,
    })
}

/// Local validation for bookmark edits (`ArchiveConfig.NOTES_MAX` / `TAG_MAX`).
pub fn validate_bookmark_fields(fields: &BookmarkEditFields) -> Vec<String> {
    let mut errors = Vec::new();
    if fields.notes.chars().count() > crate::ao3::compose::NOTES_MAX {
        errors.push(format!(
            "Bookmark notes must be at most {} characters.",
            crate::ao3::compose::NOTES_MAX
        ));
    }
    for tag in fields
        .tags
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
    {
        if tag.chars().count() > crate::ao3::compose::TAG_MAX {
            errors.push(format!(
                "Each bookmark tag must be at most {} characters.",
                crate::ao3::compose::TAG_MAX
            ));
            break;
        }
    }
    errors
}

#[cfg(test)]
mod bookmark_editor_tests {
    use super::*;

    #[test]
    fn parses_the_bookmark_editor_prefill() {
        let html = include_str!("../../../tests/fixtures/ao3_bookmark_edit.html");
        let fields = parse_bookmark_editor(html).expect("bookmark editor");
        assert_eq!(fields.bookmarkable_title, "The Clockwork Ferry");
        assert_eq!(fields.work_id.as_deref(), Some("7701"));
        assert!(fields.notes.contains("PLACEHOLDER BOOKMARK NOTE"));
        assert_eq!(fields.tags, "Fluff, Comfort Reading");
        assert!(fields.collections.is_empty());
        assert!(fields.private, "private checkbox is checked");
        assert!(fields.rec, "rec checkbox is checked");
        assert!(validate_bookmark_fields(&fields).is_empty());

        let mut broken = fields;
        broken.notes = "n".repeat(crate::ao3::compose::NOTES_MAX + 1);
        broken.tags = "t".repeat(crate::ao3::compose::TAG_MAX + 1);
        let errors = validate_bookmark_fields(&broken);
        assert!(errors.iter().any(|error| error.contains("notes")));
        assert!(errors.iter().any(|error| error.contains("tag")));
    }

    #[test]
    fn a_non_editor_bookmark_page_is_rejected() {
        assert!(parse_bookmark_editor("<html><body>login</body></html>").is_none());
    }
}
