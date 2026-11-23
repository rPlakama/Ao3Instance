#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Local search over the cache, and conditional refresh by content hash.

mod support;

use std::time::Duration;

use ao3instance::ao3::work::parse_work_page;
use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::{Response, Server};

fn fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("fixture")
}

fn config_for(dir: &std::path::Path, base: &str) -> Config {
    Config::from_pairs([
        ("AO3I_BASE_URL", base),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
    ])
    .expect("config")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http")
}

async fn start(
    dir: &std::path::Path,
    upstream: &Server,
) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = config_for(dir, &upstream.url());
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store.clone())
        .await
        .expect("router");
    let (root, handle) = support::serve(app).await;
    (root, store, handle)
}

fn seeded_pages() -> (
    ao3instance::ao3::work::WorkPage,
    ao3instance::ao3::work::WorkPage,
) {
    let mut first = parse_work_page(&fixture()).expect("fixture parses");
    first.ao3_id = "111".into();
    let mut second = first.clone();
    second.ao3_id = "222".into();
    second.title = "Second Story".into();
    second.authors = vec!["otherwriter".into()];
    second.additional_tags = vec!["DistinctiveTag".into()];
    (first, second)
}

#[tokio::test]
async fn search_covers_title_author_and_tags_over_the_cache() {
    let dir = support::temp_dir("search");
    let upstream = Server::empty();
    let (root, store, handle) = start(&dir, &upstream).await;

    let (first, second) = seeded_pages();
    store.upsert_work(&first, "h1").await.expect("seed 1");
    store.upsert_work(&second, "h2").await.expect("seed 2");

    let client = http();
    let by_tag = client
        .get(format!("{root}/search?local=1&q=DistinctiveTag"))
        .send()
        .await
        .expect("search")
        .text()
        .await
        .expect("body");
    assert!(by_tag.contains("Second Story"));
    assert!(
        !by_tag.contains(&first.title),
        "tag hit must exclude the other work"
    );

    let by_author = client
        .get(format!("{root}/search?local=1&q=otherwriter"))
        .send()
        .await
        .expect("search")
        .text()
        .await
        .expect("body");
    assert!(by_author.contains("Second Story"));

    // Blank query and a miss both render the page without results or errors.
    for query in ["?q=", "?q=nothingmatchesthis"] {
        let response = client
            .get(format!("{root}/search?local=1{query}"))
            .send()
            .await
            .expect("search");
        assert_eq!(response.status(), 200);
        let body = response.text().await.expect("body");
        assert!(!body.contains("Second Story"));
    }

    // FTS syntax characters are data, never query syntax.
    let hostile_q = ao3instance::ao3::parse::percent_encode(
        "\"; DROP TABLE works; --",
        ao3instance::ao3::parse::SpaceMode::Plus,
    );
    let hostile = client
        .get(format!("{root}/search?local=1&q={hostile_q}"))
        .send()
        .await
        .expect("search");
    assert_eq!(hostile.status(), 200);
    assert_eq!(store.list_works(10).await.expect("works").len(), 2);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn refresh_with_unchanged_content_only_touches_the_check_time() {
    let original = fixture();
    let upstream = Server::start_sequence(vec![(
        "GET /works/111?view_full_work=true".to_string(),
        vec![
            Response::ok(original.clone()).with_header("content-type", "text/html; charset=utf-8"),
            Response::ok(original).with_header("content-type", "text/html; charset=utf-8"),
        ],
    )]);
    let dir = support::temp_dir("refresh-same");
    let (root, store, handle) = start(&dir, &upstream).await;
    let client = http();

    let opened = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=111")
        .send()
        .await
        .expect("open");
    assert_eq!(opened.status(), 200);
    let before = store.get_work("111").await.expect("work").expect("cached");

    let refreshed = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=111&refresh=1")
        .send()
        .await
        .expect("refresh");
    assert_eq!(refreshed.status(), 200);
    let body = refreshed.text().await.expect("body");
    assert!(
        body.contains("no changes since the cached copy"),
        "refreshed page: {body}"
    );

    let after = store.get_work("111").await.expect("work").expect("cached");
    assert_eq!(after.content_hash, before.content_hash);
    assert!(after.fetched_at >= before.fetched_at);
    assert_eq!(upstream.requests().len(), 2, "exactly open + refresh");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn refresh_with_changed_content_updates_cache_and_search() {
    let original = fixture();
    let changed = original.replace(
        "PLACEHOLDER: The Map and the Mirror",
        "UPDATED: The Map and the Mirror",
    );
    assert_ne!(original, changed, "fixture title must be replaceable");
    let upstream = Server::start_sequence(vec![(
        "GET /works/111?view_full_work=true".to_string(),
        vec![
            Response::ok(original).with_header("content-type", "text/html; charset=utf-8"),
            Response::ok(changed).with_header("content-type", "text/html; charset=utf-8"),
        ],
    )]);
    let dir = support::temp_dir("refresh-updated");
    let (root, store, handle) = start(&dir, &upstream).await;
    let client = http();

    client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=111")
        .send()
        .await
        .expect("open");

    let refreshed = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=111&refresh=1")
        .send()
        .await
        .expect("refresh");
    assert_eq!(refreshed.status(), 200);
    let body = refreshed.text().await.expect("body");
    assert!(body.contains("Updated from AO3."), "{body}");
    assert!(body.contains("UPDATED: The Map and the Mirror"), "{body}");

    let stored = store.get_work("111").await.expect("work").expect("cached");
    assert_eq!(stored.title, "UPDATED: The Map and the Mirror");

    // The FTS trigger followed the update.
    let search = client
        .get(format!("{root}/search?local=1&q=UPDATED"))
        .send()
        .await
        .expect("search")
        .text()
        .await
        .expect("body");
    assert!(search.contains("UPDATED: The Map and the Mirror"));
    let stale = client
        .get(format!("{root}/search?local=1&q=PLACEHOLDER"))
        .send()
        .await
        .expect("search")
        .text()
        .await
        .expect("body");
    assert!(
        !stale.contains("PLACEHOLDER:"),
        "old title tokens must leave the index"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
