//! Parsers for AO3 account & directory pages: subscriptions, bookmarks,
//! reading history / Marked for Later, tag search, and media/fandom indexes.
//!
//! Selectors are traced directly to `otwarchive/app/views/`:
//! - `subscriptions/index.html.erb`
//! - `bookmarks/index.html.erb`, `_bookmark_blurb.html.erb`, `_bookmark_user_module.html.erb`
//! - `readings/index.html.erb`, `_reading_blurb.html.erb`
//! - `tags/search.html.erb` + `tags_helper.rb`
//! - `media/index.html.erb` & `fandoms/index.html.erb`

use scraper::{ElementRef, Html, Selector};

use crate::ao3::parse::percent_decode;
use crate::ao3::search::{LiveWork, parse_number, parse_total_from_heading, parse_work_element};

/// One row from `GET /users/:login/subscriptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSubscription {
    pub subscription_id: Option<i64>,
    pub kind: String,
    pub target_id: String,
    pub title: String,
    pub href: Option<String>,
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSubscriptionsPage {
    pub items: Vec<LiveSubscription>,
    pub total_pages: i64,
}

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

/// One tag result from `GET /tags/search`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTagHit {
    pub kind: String,
    pub name: String,
    pub canonical: bool,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveTagSearchResults {
    pub total: usize,
    pub total_pages: i64,
    pub hits: Vec<LiveTagHit>,
}

/// One fandom link inside `/media` or `/media/:slug/fandoms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFandom {
    pub name: String,
    pub count: Option<u64>,
}

/// One media category box on `GET /media`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveMedium {
    pub name: String,
    pub slug: String,
    pub fandoms: Vec<LiveFandom>,
}

/// Full A–Z fandom index from `GET /media/:slug/fandoms`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveMediumFandoms {
    pub title: String,
    pub groups: Vec<(String, Vec<LiveFandom>)>,
}

/// Parses `GET /users/:login/subscriptions`. Returns `None` if the HTML does
/// not look like an AO3 subscriptions page.
pub fn parse_subscriptions(html: &str) -> Option<LiveSubscriptionsPage> {
    if !html.contains("subscription index") && !html.contains("My Subscriptions") {
        return None;
    }
    let doc = Html::parse_document(html);
    let total_pages = parse_max_page(&doc);
    let Ok(dl_sel) = Selector::parse("dl.subscription.index.group") else {
        return Some(LiveSubscriptionsPage::default());
    };
    let Some(dl) = doc.select(&dl_sel).next() else {
        return Some(LiveSubscriptionsPage {
            items: Vec::new(),
            total_pages,
        });
    };

    let mut items = Vec::new();
    let mut pending: Option<LiveSubscription> = None;
    for child in dl.children() {
        let Some(el) = ElementRef::wrap(child) else {
            continue;
        };
        match el.value().name() {
            "dt" => {
                if let Some(prev) = pending.take() {
                    items.push(prev);
                }
                pending = parse_subscription_dt(&el);
            }
            "dd" => {
                if let Some(mut sub) = pending.take() {
                    sub.subscription_id = parse_subscription_dd_id(&el);
                    items.push(sub);
                }
            }
            _ => {}
        }
    }
    if let Some(last) = pending {
        items.push(last);
    }

    Some(LiveSubscriptionsPage { items, total_pages })
}

fn parse_subscription_dt(dt: &ElementRef<'_>) -> Option<LiveSubscription> {
    let link_sel = Selector::parse("a[href]").ok()?;
    let author_sel = Selector::parse("a[rel=author]").ok()?;
    let authors: Vec<String> = dt
        .select(&author_sel)
        .map(|node| collapse_text(&node))
        .filter(|name| !name.is_empty())
        .collect();

    if let Some(link) = dt.select(&link_sel).next() {
        let href = link.value().attr("href").unwrap_or_default().to_string();
        let title = collapse_text(&link);
        let (kind, target_id) = classify_subscription_href(&href);
        if title.is_empty() && target_id.is_empty() {
            return None;
        }
        return Some(LiveSubscription {
            subscription_id: None,
            kind,
            target_id,
            title,
            href: Some(href),
            authors,
        });
    }

    let raw = collapse_text(dt);
    if raw.is_empty() {
        return None;
    }
    let kind = if raw.contains("(Work)") {
        "Work"
    } else if raw.contains("(Series)") {
        "Series"
    } else {
        "Other"
    };
    Some(LiveSubscription {
        subscription_id: None,
        kind: kind.to_string(),
        target_id: String::new(),
        title: raw,
        href: None,
        authors,
    })
}

fn classify_subscription_href(href: &str) -> (String, String) {
    let clean = href.split(['?', '#']).next().unwrap_or(href);
    if let Some(rest) = clean.strip_prefix("/works/") {
        let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return ("Work".to_string(), id);
        }
    }
    if let Some(rest) = clean.strip_prefix("/series/") {
        let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return ("Series".to_string(), id);
        }
    }
    if let Some(rest) = clean.strip_prefix("/users/") {
        let seg = rest.split('/').next().unwrap_or_default();
        if !seg.is_empty() {
            let decoded = percent_decode(seg).unwrap_or_else(|| seg.to_string());
            return ("User".to_string(), decoded);
        }
    }
    ("Other".to_string(), clean.to_string())
}

fn parse_subscription_dd_id(dd: &ElementRef<'_>) -> Option<i64> {
    let form_sel = Selector::parse("form[action]").ok()?;
    let action = dd.select(&form_sel).next()?.value().attr("action")?;
    let marker = "/subscriptions/";
    let start = action.find(marker)? + marker.len();
    let digits: String = action[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
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

/// Parses `GET /tags/search?tag_search[name]=...`.
pub fn parse_tag_search(html: &str) -> LiveTagSearchResults {
    let doc = Html::parse_document(html);
    let total_pages = parse_max_page(&doc);
    let Ok(li_sel) = Selector::parse("ol.tag.index.group > li") else {
        return LiveTagSearchResults::default();
    };
    let Ok(tag_sel) = Selector::parse("a.tag") else {
        return LiveTagSearchResults::default();
    };
    let Ok(canon_sel) = Selector::parse("span.canonical") else {
        return LiveTagSearchResults::default();
    };

    let mut hits = Vec::new();
    for li in doc.select(&li_sel) {
        let Some(link) = li.select(&tag_sel).next() else {
            continue;
        };
        let name = collapse_text(&link);
        if name.is_empty() {
            continue;
        }
        let canonical = li.select(&canon_sel).next().is_some();
        let full = collapse_text(&li);
        let kind = full
            .split_once(':')
            .map(|(prefix, _)| prefix.trim().to_string())
            .filter(|prefix| !prefix.is_empty() && prefix.len() <= 32)
            .unwrap_or_else(|| "Tag".to_string());
        let count = trailing_paren_number(&full).unwrap_or(0);
        hits.push(LiveTagHit {
            kind,
            name,
            canonical,
            count,
        });
    }

    let total = Selector::parse("h3.heading")
        .ok()
        .and_then(|sel| {
            doc.select(&sel)
                .map(|node| collapse_text(&node))
                .find(|heading| heading.contains("Found"))
        })
        .and_then(|heading| parse_number(&heading.replace("Found", "")))
        .map(|value| value as usize)
        .unwrap_or(hits.len());

    LiveTagSearchResults {
        total,
        total_pages,
        hits,
    }
}

/// Parses `GET /media` (categories and top fandoms).
pub fn parse_media_index(html: &str) -> Vec<LiveMedium> {
    let doc = Html::parse_document(html);
    let Ok(med_sel) = Selector::parse("ul.media.fandom.index.group > li.medium.listbox.group")
    else {
        return Vec::new();
    };
    let Ok(heading_sel) = Selector::parse("h3.heading a") else {
        return Vec::new();
    };
    let Ok(item_sel) = Selector::parse("ol.index.group > li") else {
        return Vec::new();
    };
    let Ok(tag_sel) = Selector::parse("a.tag") else {
        return Vec::new();
    };

    let mut media = Vec::new();
    for li in doc.select(&med_sel) {
        let Some(heading_link) = li.select(&heading_sel).next() else {
            continue;
        };
        let name = collapse_text(&heading_link);
        let href = heading_link.value().attr("href").unwrap_or_default();
        let slug = extract_media_slug(href).unwrap_or_else(|| name.clone());
        let fandoms: Vec<LiveFandom> = li
            .select(&item_sel)
            .filter_map(|item| {
                let link = item.select(&tag_sel).next()?;
                let fandom_name = collapse_text(&link);
                if fandom_name.is_empty() {
                    return None;
                }
                let count = trailing_paren_number(&collapse_text(&item));
                Some(LiveFandom {
                    name: fandom_name,
                    count,
                })
            })
            .collect();
        if !name.is_empty() {
            media.push(LiveMedium {
                name,
                slug,
                fandoms,
            });
        }
    }
    media
}

/// Parses `GET /media/:slug/fandoms` (full A–Z list for one media category).
pub fn parse_medium_fandoms(html: &str) -> LiveMediumFandoms {
    let doc = Html::parse_document(html);
    let title = Selector::parse("h2.heading")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|node| {
            let full = collapse_text(&node);
            full.rsplit_once('>')
                .map_or(full.as_str(), |(_, tail)| tail)
                .trim()
                .to_string()
        })
        .unwrap_or_default();

    let Ok(letter_sel) =
        Selector::parse("ol.alphabet.fandom.index.group > li.letter.listbox.group")
    else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };
    let Ok(heading_sel) = Selector::parse("h3.heading") else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };
    let Ok(item_sel) = Selector::parse("ul.tags.index.group > li") else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };
    let Ok(tag_sel) = Selector::parse("a.tag") else {
        return LiveMediumFandoms {
            title,
            groups: Vec::new(),
        };
    };

    let mut groups = Vec::new();
    for li in doc.select(&letter_sel) {
        let letter = li
            .value()
            .attr("id")
            .and_then(|id| id.strip_prefix("letter-"))
            .map(str::to_string)
            .or_else(|| {
                li.select(&heading_sel)
                    .next()
                    .and_then(|node| collapse_text(&node).split_whitespace().next().map(str::to_string))
            })
            .unwrap_or_else(|| "#".to_string());
        let fandoms: Vec<LiveFandom> = li
            .select(&item_sel)
            .filter_map(|item| {
                let link = item.select(&tag_sel).next()?;
                let name = collapse_text(&link);
                if name.is_empty() {
                    return None;
                }
                let count = trailing_paren_number(&collapse_text(&item));
                Some(LiveFandom { name, count })
            })
            .collect();
        if !fandoms.is_empty() {
            groups.push((letter, fandoms));
        }
    }

    LiveMediumFandoms { title, groups }
}

fn extract_media_slug(href: &str) -> Option<String> {
    let rest = href.strip_prefix("/media/")?;
    let seg = rest.split('/').next()?;
    if seg.is_empty() {
        return None;
    }
    Some(percent_decode(seg).unwrap_or_else(|| seg.to_string()))
}

fn trailing_paren_number(text: &str) -> Option<u64> {
    let open = text.rfind('(')?;
    let close = text[open..].find(')')?;
    parse_number(&text[open + 1..open + close])
}

fn parse_max_page(doc: &Html) -> i64 {
    let Ok(sel) = Selector::parse("ol.pagination a, ol.pagination span.current") else {
        return 1;
    };
    doc.select(&sel)
        .filter_map(|node| collapse_text(&node).parse::<i64>().ok())
        .max()
        .unwrap_or(1)
        .max(1)
}

fn collapse_text(element: &ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subscriptions_with_ids_kinds_and_authors() {
        let html = r#"
        <dl class="subscription index group">
          <dt>
            <a href="/works/111">A Mirror of Small Things</a> (Work)
            by <a rel="author" href="/users/tester">tester</a>
          </dt>
          <dd>
            <form action="/users/me/subscriptions/501" method="post">
              <input type="submit" value="Unsubscribe from A Mirror of Small Things">
            </form>
          </dd>
          <dt>
            <a href="/series/222">Mirror Series</a> (Series)
            by <a rel="author" href="/users/tester">tester</a>
          </dt>
          <dd>
            <form action="/users/me/subscriptions/502" method="post"></form>
          </dd>
          <dt><a href="/users/some_author">some_author</a></dt>
          <dd><form action="/users/me/subscriptions/503" method="post"></form></dd>
        </dl>
        <ol class="pagination"><li><span class="current">1</span></li><li><a href="?page=3">3</a></li></ol>
        "#;

        let page = parse_subscriptions(html).expect("subscriptions");
        assert_eq!(page.total_pages, 3);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.items[0].subscription_id, Some(501));
        assert_eq!(page.items[0].kind, "Work");
        assert_eq!(page.items[0].target_id, "111");
        assert_eq!(page.items[0].title, "A Mirror of Small Things");
        assert_eq!(page.items[0].authors, ["tester"]);
        assert_eq!(page.items[1].kind, "Series");
        assert_eq!(page.items[1].target_id, "222");
        assert_eq!(page.items[2].kind, "User");
        assert_eq!(page.items[2].target_id, "some_author");
    }

    #[test]
    fn parses_bookmarks_with_work_blurb_and_user_module() {
        let html = r#"
        <h2 class="heading">1 - 2 of 14 Bookmarks by me</h2>
        <ol class="bookmark index group">
          <li id="bookmark_900" class="bookmark blurb group" role="article">
            <p class="status"><span class="private">Private</span><span class="rec">Rec</span></p>
            <div class="header module">
              <h4 class="heading">
                <a href="/works/111">A Mirror of Small Things</a> by
                <a rel="author" href="/users/tester">tester</a>
              </h4>
              <h5 class="fandoms heading"><a class="tag" href="/tags/Test/works">Test Fandom</a></h5>
              <p class="datetime">12 Nov 2026</p>
            </div>
            <div class="own user module group">
              <p class="datetime">15 Nov 2026</p>
              <ul class="meta tags commas">
                <li><a class="tag" href="/tags/favorite/bookmarks">favorite</a></li>
              </ul>
              <blockquote class="userstuff notes"><p>Re-read this!</p></blockquote>
            </div>
          </li>
        </ol>
        "#;

        let page = parse_bookmarks(html).expect("bookmarks");
        assert_eq!(page.total, 14);
        assert_eq!(page.items.len(), 1);
        let item = &page.items[0];
        assert_eq!(item.bookmark_id, Some(900));
        assert!(item.is_private);
        assert!(item.is_rec);
        assert_eq!(item.bookmarked_at, "15 Nov 2026");
        assert_eq!(item.tags, ["favorite"]);
        assert!(
            item.notes_html
                .as_deref()
                .is_some_and(|n| n.contains("Re-read this!"))
        );
        let work = item.work.as_ref().expect("parsed work");
        assert_eq!(work.ao3_id, "111");
        assert_eq!(work.title, "A Mirror of Small Things");
    }

    #[test]
    fn parses_readings_and_marked_for_later() {
        let html = r#"
        <ol class="reading work index group">
          <li id="work_111" class="reading work blurb group" role="article">
            <div class="header module">
              <h4 class="heading">
                <a href="/works/111">Night Watch</a> by
                <a rel="author" href="/users/alice">alice</a>
              </h4>
              <h5 class="fandoms heading"><a class="tag" href="/tags/F/works">F</a></h5>
              <p class="datetime">01 Nov 2026</p>
            </div>
            <div class="user module group">
              <h4 class="viewed heading"><span>Last visited:</span> 20 Nov 2026 (Latest version.) Visited 4 times (Marked for Later.)</h4>
            </div>
          </li>
        </ol>
        "#;

        let page = parse_readings(html).expect("readings");
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].marked_for_later);
        assert!(page.items[0].viewed_info.contains("Visited 4 times"));
        assert_eq!(page.items[0].work.as_ref().expect("work").ao3_id, "111");
    }

    #[test]
    fn parses_tag_search_results() {
        let html = r#"
        <h3 class="heading">24 Found</h3>
        <ol class="tag index group">
          <li><span class="canonical">Fandom: <a class="tag" href="/tags/Re:Zero">Re:Zero</a> &lrm;(12,345)</span></li>
          <li><span>Freeform: <a class="tag" href="/tags/Time%20Loop">Time Loop</a> &lrm;(99)</span></li>
        </ol>
        "#;

        let results = parse_tag_search(html);
        assert_eq!(results.total, 24);
        assert_eq!(results.hits.len(), 2);
        assert_eq!(results.hits[0].kind, "Fandom");
        assert_eq!(results.hits[0].name, "Re:Zero");
        assert!(results.hits[0].canonical);
        assert_eq!(results.hits[0].count, 12345);
        assert_eq!(results.hits[1].kind, "Freeform");
        assert!(!results.hits[1].canonical);
        assert_eq!(results.hits[1].count, 99);
    }

    #[test]
    fn parses_media_index_and_medium_fandoms() {
        let media_html = r#"
        <ul class="media fandom index group">
          <li id="medium_1" class="medium listbox group">
            <h3 class="heading"><a href="/media/Anime%20*a*%20Manga/fandoms">Anime &amp; Manga</a></h3>
            <ol class="index group">
              <li><a class="tag" href="/tags/Re:Zero/works">Re:Zero</a> (12,345)</li>
            </ol>
          </li>
        </ul>
        "#;
        let media = parse_media_index(media_html);
        assert_eq!(media.len(), 1);
        assert_eq!(media[0].name, "Anime & Manga");
        assert_eq!(media[0].slug, "Anime *a* Manga");
        assert_eq!(media[0].fandoms[0].name, "Re:Zero");
        assert_eq!(media[0].fandoms[0].count, Some(12345));

        let fandoms_html = r##"
        <h2 class="heading"><a href="/media">Fandoms</a> &gt; Anime &amp; Manga</h2>
        <ol class="alphabet fandom index group">
          <li id="letter-R" class="letter listbox group">
            <h3 class="heading">R <span class="action top"><a href="#alphabet">↑</a></span></h3>
            <ul class="tags index group">
              <li><a class="tag" href="/tags/Re:Zero/works">Re:Zero</a> (12,345)</li>
            </ul>
          </li>
        </ol>
        "##;
        let parsed = parse_medium_fandoms(fandoms_html);
        assert_eq!(parsed.title, "Anime & Manga");
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].0, "R");
        assert_eq!(parsed.groups[0].1[0].name, "Re:Zero");
    }
}
