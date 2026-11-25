//! Parsers for AO3 account & directory pages: subscriptions, bookmarks,
//! reading history / Marked for Later, tag search, and media/fandom indexes.
//!
//! Selectors are traced directly to `otwarchive/app/views/`:
//! - `subscriptions/index.html.erb`
//! - `bookmarks/index.html.erb`, `_bookmark_blurb.html.erb`, `_bookmark_user_module.html.erb`
//! - `readings/index.html.erb`, `_reading_blurb.html.erb`
//! - `tags/search.html.erb` + `tags_helper.rb`
//! - `media/index.html.erb` & `fandoms/index.html.erb`

mod bookmarks;
mod directory;
mod readings;
mod subscriptions;
mod works;

use scraper::{Html, Selector};

use crate::ao3::html::{collapse_text, parse_number};

pub use bookmarks::{LiveBookmark, LiveBookmarksPage, parse_bookmarks};
pub use directory::{
    LiveFandom, LiveMedium, LiveMediumFandoms, LiveTagHit, LiveTagSearchResults, parse_media_index,
    parse_medium_fandoms, parse_tag_search,
};
pub use readings::{LiveReadingItem, LiveReadingsPage, parse_readings};
pub use subscriptions::{LiveSubscription, LiveSubscriptionsPage, parse_subscriptions};
pub use works::{MyWorkEntry, MyWorksPage, parse_my_works};

pub(super) fn trailing_paren_number(text: &str) -> Option<u64> {
    let open = text.rfind('(')?;
    let close = text[open..].find(')')?;
    parse_number(&text[open + 1..open + close])
}

pub(super) fn parse_max_page(doc: &Html) -> i64 {
    let Ok(sel) = Selector::parse("ol.pagination a, ol.pagination span.current") else {
        return 1;
    };
    doc.select(&sel)
        .filter_map(|node| collapse_text(&node).parse::<i64>().ok())
        .max()
        .unwrap_or(1)
        .max(1)
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
