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
        ("AO3I_FLARESOLVERR", "off".to_string()),
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
    assert!(body.contains("(Restricted)"), "{body}");
    assert!(body.contains("class=\"series\""), "{body}");
    assert!(body.contains("Collections:"), "{body}");
    assert!(body.contains("Signed out:"), "{body}");

    // The literal upstream request carried AO3's own parameter names.
    let requests = upstream.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        requests[0].path.contains("work_search%5Bquery%5D=mirror"),
        "{}",
        requests[0].path
    );
    assert!(
        !requests[0].path.contains("sort_column"),
        "default best match must match AO3's omitted sort: {}",
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
    assert!(body.contains("Search blocked"), "{body}");
    assert!(body.contains("HTTP 429"), "{body}");

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

#[tokio::test]
async fn tag_pages_are_live_and_work_entries_link_to_tags() {
    let tag_page = fixture()
        .replace(
            "<h2 class=\"heading\">Search Results</h2>",
            "<h2 class=\"heading\">1 - 2 of 2 Works in Test</h2>",
        )
        .replace("<h3 class=\"heading\">2 Found</h3>", "");
    let tag_query = LiveSearchQuery {
        include_tags: vec!["Test".into()],
        sort: Some("date".into()),
        direction: Some("desc".into()),
        page: 1,
        ..LiveSearchQuery::default()
    };
    let tag_path = tag_query.path(1);
    let filtered_query = LiveSearchQuery {
        include_tags: vec!["Explicit".into(), "Test".into()],
        sort: Some("words".into()),
        direction: Some("desc".into()),
        page: 1,
        ..LiveSearchQuery::default()
    };
    let filtered_path = filtered_query.path(1);
    let upstream = Server::start(vec![
        (format!("GET {tag_path}"), Response::ok(tag_page.clone())),
        (format!("GET {filtered_path}"), Response::ok(tag_page)),
    ]);
    let dir = support::temp_dir("live-tag");
    let (root, _store, handle) = serve(&dir, &upstream).await;

    let body = http()
        .get(format!("{root}/tags/Test/works"))
        .send()
        .await
        .expect("tag page")
        .text()
        .await
        .expect("body");
    assert!(body.contains("2 Works in"), "{body}");
    assert!(body.contains("A Mirror of Small Things"), "{body}");
    assert!(body.contains("id=\"work-filters\""), "{body}");
    assert!(body.contains("Sort and Filter"), "{body}");
    assert!(body.contains("Ratings"), "{body}");
    assert!(body.contains("Include"), "{body}");
    assert!(body.contains("Exclude"), "{body}");
    assert!(body.contains("/tags/Test%20Fandom/works"), "{body}");
    assert!(body.contains("/open/111"), "{body}");
    let filtered = http()
        .get(format!(
            "{root}/tags/Test/works?include_tag=Explicit&sort=words"
        ))
        .send()
        .await
        .expect("filtered tag page")
        .text()
        .await
        .expect("filtered body");
    assert!(
        filtered.contains("value=\"Explicit\" checked"),
        "{filtered}"
    );
    assert!(
        upstream.requests().iter().any(|request| {
            request
                .path
                .contains("work_search%5Bother_tag_names%5D=Explicit%2C%20Test")
                && request
                    .path
                    .contains("work_search%5Bsort_column%5D=word_count")
        }),
        "{:?}",
        upstream.requests()
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
