//! Parser for `GET /users/:login/readings` (History and Marked for Later).

use scraper::{Html, Selector};

use crate::ao3::html::collapse_text;
use crate::ao3::search::{LiveWork, parse_work_element};

use super::parse_max_page;

/// One entry from `GET /users/:login/readings` (History or Marked for Later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveReadingItem {
    pub work: Option<LiveWork>,
    pub viewed_info: String,
    pub marked_for_later: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveReadingsPage {
    pub total_pages: i64,
    pub items: Vec<LiveReadingItem>,
}

/// Parses `GET /users/:login/readings` and `?show=to-read`. Returns `None` if
/// the HTML does not look like an AO3 readings page.
pub fn parse_readings(html: &str) -> Option<LiveReadingsPage> {
    if !html.contains("reading work index")
        && !html.contains("Marked for Later")
        && !html.contains("History")
    {
        return None;
    }
    let doc = Html::parse_document(html);
    let total_pages = parse_max_page(&doc);
    let Ok(li_sel) = Selector::parse("ol.reading.work.index.group > li.reading.work.blurb.group")
    else {
        return Some(LiveReadingsPage::default());
    };
    let Ok(viewed_sel) = Selector::parse("div.user.module.group h4.viewed.heading") else {
        return Some(LiveReadingsPage::default());
    };

    let mut items = Vec::new();
    for li in doc.select(&li_sel) {
        let work = parse_work_element(&li).ok();
        let viewed_info = li
            .select(&viewed_sel)
            .next()
            .map(|node| collapse_text(&node))
            .unwrap_or_default();
        let marked_for_later = viewed_info.contains("Marked for Later");
        if work.is_some() || !viewed_info.is_empty() {
            items.push(LiveReadingItem {
                work,
                viewed_info,
                marked_for_later,
            });
        }
    }

    Some(LiveReadingsPage { total_pages, items })
}
