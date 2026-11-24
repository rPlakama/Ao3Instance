#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! End-to-end tests for live AO3 account lists (subscriptions, bookmarks,
//! reading history / Marked for Later, dashboard), live fandoms & tag search,
//! and the 30-minute persistent SQLite page cache (including `?refresh=1` and
//! stale-on-error fallback).

mod support;

use ao3instance::ao3::search::LiveSearchQuery;
use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

fn search_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/search_results.html").expect("fixture")
}

const SUBSCRIPTIONS_HTML: &str = r#"
<h2 class="heading">My Subscriptions</h2>
<dl class="subscription index group">
  <dt>
    <a href="/works/999">Upstream Subscribed Work</a> (Work)
    by <a rel="author" href="/users/alice">alice</a>
  </dt>
  <dd>
    <form action="/users/someone/subscriptions/777" method="post">
      <input type="submit" value="Unsubscribe from Upstream Subscribed Work">
    </form>
  </dd>
  <dt><a href="/users/favorite_author">favorite_author</a></dt>
  <dd>
    <form action="/users/someone/subscriptions/778" method="post"></form>
  </dd>
</dl>
"#;

const BOOKMARKS_HTML: &str = r#"
<h2 class="heading">1 - 1 of 12 Bookmarks by someone</h2>
<ol class="bookmark index group">
  <li id="bookmark_555" class="bookmark blurb group" role="article">
    <p class="status"><span class="rec">Rec</span></p>
    <div class="header module">
      <h4 class="heading">
        <a href="/works/888">Upstream Bookmarked Fic</a> by
        <a rel="author" href="/users/bob">bob</a>
      </h4>
      <h5 class="fandoms heading"><a class="tag" href="/tags/Test/works">Test Fandom</a></h5>
      <p class="datetime">10 Nov 2026</p>
    </div>
    <div class="own user module group">
      <p class="datetime">18 Nov 2026</p>
      <ul class="meta tags commas">
        <li><a class="tag" href="/tags/comfort/bookmarks">comfort</a></li>
      </ul>
      <blockquote class="userstuff notes"><p>Loved chapter 3.</p></blockquote>
    </div>
  </li>
</ol>
"#;

const READINGS_HTML: &str = r#"
<h2 class="heading">History</h2>
<ol class="reading work index group">
  <li id="work_12345" class="reading work blurb group" role="article">
    <div class="header module">
      <h4 class="heading">
        <a href="/works/12345">PLACEHOLDER: The Map and the Mirror</a> by
        <a rel="author" href="/users/tester">tester</a>
      </h4>
      <h5 class="fandoms heading"><a class="tag" href="/tags/Test/works">Test Fandom</a></h5>
      <p class="datetime">01 Nov 2026</p>
    </div>
    <div class="user module group">
      <h4 class="viewed heading"><span>Last visited:</span> 21 Nov 2026 (Latest version.) Visited 7 times</h4>
    </div>
  </li>
</ol>
"#;

const TO_READ_HTML: &str = r#"
<h2 class="heading">Marked for Later</h2>
<ol class="reading work index group">
  <li id="work_444" class="reading work blurb group" role="article">
    <div class="header module">
      <h4 class="heading">
        <a href="/works/444">Saved For Tonight</a> by
        <a rel="author" href="/users/carol">carol</a>
      </h4>
      <h5 class="fandoms heading"><a class="tag" href="/tags/Test/works">Test Fandom</a></h5>
      <p class="datetime">05 Nov 2026</p>
    </div>
    <div class="user module group">
      <h4 class="viewed heading"><span>Last visited:</span> 22 Nov 2026 (Marked for Later.)</h4>
    </div>
  </li>
</ol>
"#;

const MEDIA_HTML: &str = r#"
<h2 class="heading">Fandoms</h2>
<ul class="media fandom index group">
  <li id="medium_1" class="medium listbox group">
    <h3 class="heading"><a href="/media/Anime%20*a*%20Manga/fandoms">Anime &amp; Manga</a></h3>
    <ol class="index group">
      <li><a class="tag" href="/tags/Re:Zero/works">Re:Zero</a> (12,345)</li>
    </ol>
  </li>
</ul>
"#;

const MEDIUM_FANDOMS_HTML: &str = r##"
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

const TAG_SEARCH_HTML: &str = r#"
<h3 class="heading">5 Found</h3>
<ol class="tag index group">
  <li><span class="canonical">Fandom: <a class="tag" href="/tags/Re:Zero">Re:Zero</a> &lrm;(12,345)</span></li>
  <li><span>Freeform: <a class="tag" href="/tags/Time%20Loop">Time Loop</a> &lrm;(42)</span></li>
</ol>
"#;

#[tokio::test]
async fn search_results_are_cached_30_minutes_and_support_refresh_and_stale_fallback() {
    let path = LiveSearchQuery {
        query: "mirror".into(),
        ..LiveSearchQuery::default()
    }
    .path(1);
    let upstream = Server::start_sequence(vec![(
        format!("GET {path}"),
        vec![
            Response::ok(search_fixture()),
            Response::ok(search_fixture().replace("A Mirror of Small Things", "Refreshed Mirror")),
            Response::new(503),
        ],
    )]);
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let client = http();

    // 1st request: cache miss -> fetches upstream once.
    let first = client
        .get(format!("{root}/works/search?q=mirror"))
        .send()
        .await
        .expect("first search")
        .text()
        .await
        .expect("body");
    assert!(first.contains("A Mirror of Small Things"), "{first}");
    assert!(first.contains("cached for 30 min"), "{first}");
    assert_eq!(upstream.requests().len(), 1);

    // 2nd identical request: served from 30-min SQLite cache (0 extra upstream requests).
    let second = client
        .get(format!("{root}/works/search?q=mirror"))
        .send()
        .await
        .expect("cached search")
        .text()
        .await
        .expect("body");
    assert!(second.contains("A Mirror of Small Things"), "{second}");
    assert!(second.contains("30-min cache"), "{second}");
    assert_eq!(
        upstream.requests().len(),
        1,
        "second search must hit the 30-min cache"
    );

    // Explicit refresh bypasses the cache and updates it.
    let refreshed = client
        .get(format!("{root}/works/search?q=mirror&refresh=1"))
        .send()
        .await
        .expect("refresh search")
        .text()
        .await
        .expect("body");
    assert!(refreshed.contains("Refreshed Mirror"), "{refreshed}");
    assert_eq!(upstream.requests().len(), 2);

    // Stale-on-error fallback: when upstream fails with 503 on refresh, cached copy still renders.
    let fallback = client
        .get(format!("{root}/works/search?q=mirror&refresh=1"))
        .send()
        .await
        .expect("stale fallback")
        .text()
        .await
        .expect("body");
    assert!(
        fallback.contains("Refreshed Mirror"),
        "cached search should survive upstream 503: {fallback}"
    );
    assert_eq!(upstream.requests().len(), 3);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn account_lists_and_dashboard_fetch_real_ao3_data_and_cache_it() {
    let mut routes = session_routes(vec![
        ("GET /users/someone/bookmarks", Response::ok(BOOKMARKS_HTML)),
        ("GET /users/someone/readings", Response::ok(READINGS_HTML)),
        (
            "GET /users/someone/readings?show=to-read",
            Response::ok(TO_READ_HTML),
        ),
        (
            "DELETE /users/someone/subscriptions/777.json",
            Response::json(r#"{"item_success_message":"Unsubscribed."}"#),
        ),
    ]);
    // Override the stub subscriptions route from session_routes with real AO3 markup.
    for (route, response) in &mut routes {
        if route == "GET /users/someone/subscriptions" {
            *response = Response::ok(SUBSCRIPTIONS_HTML);
        }
    }
    let upstream = Server::start(routes);
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    store.save_progress("12345", 2, 50).await.expect("progress");

    // Subscriptions come from AO3 (not just local receipts), with real subscription_id for unsubscribe.
    let subs = client
        .get(format!("{root}/subscriptions"))
        .send()
        .await
        .expect("subs")
        .text()
        .await
        .expect("body");
    assert!(subs.contains("Upstream Subscribed Work"), "{subs}");
    assert!(subs.contains("favorite_author"), "{subs}");
    assert!(subs.contains("/open/999"), "{subs}");
    assert!(subs.contains("value=\"777\""), "{subs}");

    // Bookmarks show full AO3 blurbs, bookmarker tags, notes, and rec status.
    let bm = client
        .get(format!("{root}/bookmarks"))
        .send()
        .await
        .expect("bookmarks")
        .text()
        .await
        .expect("body");
    assert!(bm.contains("12 Bookmarks"), "{bm}");
    assert!(bm.contains("Upstream Bookmarked Fic"), "{bm}");
    assert!(bm.contains("Loved chapter 3."), "{bm}");
    assert!(bm.contains("comfort"), "{bm}");
    assert!(bm.contains("Rec"), "{bm}");

    // History and Marked for Later come from AO3, merged with local progress.
    let history = client
        .get(format!("{root}/reading"))
        .send()
        .await
        .expect("reading")
        .text()
        .await
        .expect("body");
    assert!(history.contains("Visited 7 times"), "{history}");
    assert!(history.contains("Local progress: chapter 2"), "{history}");

    let to_read = client
        .get(format!("{root}/reading?show=to-read"))
        .send()
        .await
        .expect("to-read")
        .text()
        .await
        .expect("body");
    assert!(to_read.contains("Saved For Tonight"), "{to_read}");
    assert!(to_read.contains("Marked for Later"), "{to_read}");

    // Dashboard reuses the 30-min cached AO3 lists (0 extra upstream requests for those lists!).
    let count_before_dash = upstream.requests().len();
    let dash = client
        .get(format!("{root}/dashboard"))
        .send()
        .await
        .expect("dashboard")
        .text()
        .await
        .expect("body");
    assert!(dash.contains("Upstream Subscribed Work"), "{dash}");
    assert!(dash.contains("Upstream Bookmarked Fic"), "{dash}");
    assert!(dash.contains("Visited 7 times"), "{dash}");
    assert_eq!(
        upstream.requests().len(),
        count_before_dash,
        "dashboard must reuse cached AO3 lists within 30 min"
    );

    // Unsubscribing from an AO3-only subscription works via its subscription_id and invalidates cache.
    let unsub = client
        .post(format!("{root}/write/unsubscribe"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("kind=Work&target_id=999&subscription_id=777")
        .send()
        .await
        .expect("unsubscribe");
    assert_eq!(unsub.status(), 200);
    assert!(
        store
            .get_cached_page("someone", "/users/someone/subscriptions")
            .await
            .expect("cache")
            .is_none(),
        "subscriptions cache must be invalidated after unsubscribe"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn fandoms_and_tag_search_fetch_live_ao3_and_cache_30_minutes() {
    let upstream = Server::start(vec![
        ("GET /media", Response::ok(MEDIA_HTML)),
        (
            "GET /media/Anime%20*a*%20Manga/fandoms",
            Response::ok(MEDIUM_FANDOMS_HTML),
        ),
        (
            "GET /tags/search?tag_search%5Bname%5D=Re",
            Response::ok(TAG_SEARCH_HTML),
        ),
    ]);
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let client = http();

    let media = client
        .get(format!("{root}/fandoms"))
        .send()
        .await
        .expect("media")
        .text()
        .await
        .expect("body");
    assert!(media.contains("Anime &amp; Manga"), "{media}");
    assert!(media.contains("Re:Zero"), "{media}");
    assert!(
        media.contains("/media/Anime%20*a*%20Manga/fandoms"),
        "{media}"
    );

    let medium = client
        .get(format!("{root}/media/Anime%20*a*%20Manga/fandoms"))
        .send()
        .await
        .expect("medium")
        .text()
        .await
        .expect("body");
    assert!(medium.contains("Re:Zero"), "{medium}");
    assert!(medium.contains("12,345"), "{medium}");

    let tags = client
        .get(format!("{root}/tags/search?q=Re"))
        .send()
        .await
        .expect("tags")
        .text()
        .await
        .expect("body");
    assert!(tags.contains("5 Found"), "{tags}");
    assert!(tags.contains("class=\"canonical\""), "{tags}");
    assert!(tags.contains("Time Loop"), "{tags}");

    // Second visits to all three hit the 30-min SQLite cache (0 extra requests).
    let reqs = upstream.requests().len();
    assert_eq!(reqs, 3);
    for path in [
        "/fandoms",
        "/media/Anime%20*a*%20Manga/fandoms",
        "/tags/search?q=Re",
    ] {
        let resp = client
            .get(format!("{root}{path}"))
            .send()
            .await
            .expect(path);
        assert_eq!(resp.status(), 200);
    }
    assert_eq!(
        upstream.requests().len(),
        reqs,
        "second visits must hit the 30-min cache"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
