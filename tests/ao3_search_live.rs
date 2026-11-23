#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Literal AO3 search: the form submission performs one upstream request to
//! AO3's search endpoint and renders its results; nothing is cached.

mod support;

use ao3instance::ao3::search::LiveSearchQuery;
use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::http;
use support::{Response, Server};

fn fixture() -> String {
    std::fs::read_to_string("tests/fixtures/search_results.html").expect("fixture")
}

fn search_path(query: &str) -> String {
    LiveSearchQuery {
        query: query.to_string(),
        ..LiveSearchQuery::default()
    }
    .path(1)
}

async fn serve(
    dir: &std::path::Path,
    upstream: &Server,
) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", upstream.url()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir").to_string()),
        (
            "XDG_CONFIG_HOME",
            dir.to_str().expect("utf8 dir").to_string(),
        ),
        ("AO3I_MIN_SPACING_MS", "0".to_string()),
    ])
    .expect("config");
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store.clone())
        .await
        .expect("router");
    let (root, handle) = support::serve(app).await;
    (root, store, handle)
}

#[tokio::test]
async fn search_is_a_literal_ao3_request_and_renders_its_entries() {
    let path = search_path("mirror");
    let upstream = Server::start(vec![(format!("GET {path}"), Response::ok(fixture()))]);
    let dir = support::temp_dir("live-search");
    let (root, store, handle) = serve(&dir, &upstream).await;
    let client = http();

    let body = client
        .get(format!("{root}/works/search?q=mirror"))
        .send()
        .await
        .expect("search")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Search Results"), "{body}");
    assert!(
        body.contains("You searched for: mirror sort by: best match descending"),
        "{body}"
    );
    assert!(body.contains("2 Found"), "{body}");
    assert!(body.contains("A Mirror of Small Things"), "{body}");
    assert!(body.contains("Second Mirror"), "{body}");
    assert!(body.contains("Edit Your Search"), "{body}");
    assert!(body.contains("Cache this work"), "{body}");

    // The literal upstream request carried AO3's own parameter names.
    let requests = upstream.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        requests[0].path.contains("work_search%5Bquery%5D=mirror"),
        "{}",
        requests[0].path
    );
    assert!(
        requests[0]
            .path
            .contains("work_search%5Bsort_column%5D=_score"),
        "{}",
        requests[0].path
    );

    // Results are not silently cached as works.
    assert_eq!(store.count_works().await.expect("count"), 0);
    let cached = store.all_works().await.expect("works");
    assert!(cached.is_empty(), "{cached:?}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn edit_search_shows_the_form_without_calling_ao3() {
    let upstream = Server::empty();
    let dir = support::temp_dir("live-search-edit");
    let (root, _store, handle) = serve(&dir, &upstream).await;

    let body = http()
        .get(format!("{root}/works/search?q=mirror&edit_search=true"))
        .send()
        .await
        .expect("edit")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Work Search"), "{body}");
    assert!(body.contains("name=\"q\""), "{body}");
    assert!(body.contains("name=\"include_tags\""), "{body}");
    assert!(!body.contains("Search Results"), "{body}");
    assert!(
        upstream.requests().is_empty(),
        "edit must not search upstream"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn upstream_search_failures_are_typed() {
    let path = search_path("mirror");
    let upstream = Server::start(vec![(format!("GET {path}"), Response::new(429))]);
    let dir = support::temp_dir("live-search-error");
    let (root, _store, handle) = serve(&dir, &upstream).await;

    let response = http()
        .get(format!("{root}/works/search?q=mirror"))
        .send()
        .await
        .expect("search");
    assert_eq!(response.status(), 502);
    let body = response.text().await.expect("body");
    assert!(body.contains("rate limiting"), "{body}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn the_form_posts_locally_and_issues_one_ao3_get() {
    let path = search_path("mirror");
    let upstream = Server::start(vec![(format!("GET {path}"), Response::ok(fixture()))]);
    let dir = support::temp_dir("live-search-post");
    let (root, _store, handle) = serve(&dir, &upstream).await;

    let response = http()
        .post(format!("{root}/works/search"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("q=mirror&sort=best_match&direction=desc")
        .send()
        .await
        .expect("post search");
    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("body");
    assert!(body.contains("Search Results"), "{body}");
    assert!(body.contains("A Mirror of Small Things"), "{body}");

    let requests = upstream.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0].method, "GET",
        "AO3's web search endpoint is GET"
    );
    assert!(
        requests[0].path.contains("work_search%5Bquery%5D=mirror"),
        "{}",
        requests[0].path
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
