//! Parser for the signed-in author's works index:
//! `GET /users/:login/works` (posted) and `GET /users/:login/works/drafts`.
//!
//! Both pages render `works/_work_blurb.html.erb` inside `ol.work.index.group`
//! (`works/index.html.erb:56`) or `ul.work.index.group` (`works/drafts.html.erb:19`).
//! Unposted works render the draft caution notice and the author actions
//! `Post Draft` (`/works/:id/post_draft`) and `Delete Draft`
//! (`/works/:id/confirm_delete`); the presence of those links is the reliable
//! draft marker (`_work_blurb.html.erb:8-18`).

use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::collapse_text;
use crate::ao3::search::{LiveWork, parse_work_element};

use super::parse_max_page;

/// One work on the author's own works/drafts page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyWorkEntry {
    pub work: LiveWork,
    pub draft: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MyWorksPage {
    pub items: Vec<MyWorkEntry>,
    pub total_pages: i64,
}

/// Parses one page of the author's works or drafts index.
///
/// Returns `None` when the page has no work list at all (login wall, challenge,
/// or layout drift), so callers can fall back honestly.
pub fn parse_my_works(html: &str) -> Option<MyWorksPage> {
    let doc = Html::parse_document(html);
    let list = Selector::parse(
        "ol.work.index.group > li.work.blurb.group, ul.work.index.group > li.work.blurb.group",
    )
    .ok()?;
    let mut items = Vec::new();
    let mut saw_list = false;
    for element in doc.select(&list) {
        saw_list = true;
        let work = parse_work_element(&element).ok()?;
        items.push(MyWorkEntry {
            draft: is_draft(&element),
            work,
        });
    }
    if !saw_list
        && !(html.contains("work index group")
            && (html.contains("My Works") || html.contains("Works by") || html.contains("Drafts")))
    {
        return None;
    }
    Some(MyWorksPage {
        items,
        total_pages: parse_max_page(&doc),
    })
}

fn is_draft(element: &ElementRef<'_>) -> bool {
    if let Ok(selector) = Selector::parse("ul.actions a[href*='/post_draft']")
        && element.select(&selector).next().is_some()
    {
        return true;
    }
    if let Ok(selector) = Selector::parse("p.caution.notice")
        && let Some(notice) = element.select(&selector).next()
        && collapse_text(&notice).to_lowercase().contains("draft")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAFTS: &str = include_str!("../../../tests/fixtures/ao3_my_drafts.html");
    const WORKS: &str = include_str!("../../../tests/fixtures/ao3_my_works.html");

    #[test]
    fn parses_drafts_with_flags_and_pagination() {
        let page = parse_my_works(DRAFTS).expect("drafts page");
        assert_eq!(page.total_pages, 2);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].work.ao3_id, "8801");
        assert_eq!(page.items[0].work.title, "A Draft in the Cupboard");
        assert!(page.items[0].draft);
        assert!(page.items[1].draft);
        assert_eq!(page.items[0].work.fandoms, vec!["Test Fandom".to_string()]);
        assert_eq!(page.items[0].work.words, Some(1_234));
    }

    #[test]
    fn parses_posted_works_without_draft_flag() {
        let page = parse_my_works(WORKS).expect("works page");
        assert_eq!(page.total_pages, 3);
        assert_eq!(page.items.len(), 2);
        assert!(!page.items.iter().any(|entry| entry.draft));
        assert_eq!(page.items[0].work.ao3_id, "7701");
        assert_eq!(page.items[0].work.words, Some(3_456));
        assert_eq!(page.items[0].work.chapters.as_deref(), Some("2/2"));
        assert_eq!(page.items[0].work.kudos, Some(17));
    }

    #[test]
    fn a_login_page_is_not_an_author_index() {
        assert!(
            parse_my_works("<html><body><div id=\"login\">Log in</div></body></html>").is_none()
        );
    }
}
