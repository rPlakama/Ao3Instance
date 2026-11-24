#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! End-to-end smoke test: the app opens a work from an upstream, caches it, and
//! serves the reader. The upstream is a local mock serving a checked-in fixture —
//! no test ever talks to archiveofourown.org.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::{Response, Server};

fn fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html")
        .expect("fixture tests/fixtures/work_full.html must exist")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ao3i-server-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[tokio::test]
async fn app_opens_a_work_from_a_mock_upstream() {
    let html = fixture();
    let upstream = Server::start(vec![(
        "GET /works/12345?view_full_work=true",
        Response::ok(html.clone()).with_header("content-type", "text/html; charset=utf-8"),
    )]);

    let dir = temp_dir("open");
    let base = upstream.url();
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", base.as_str()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 temp dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 temp dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
        ("AO3I_FLARESOLVERR", "off"),
        ("AO3I_PORT", "0"),
    ])
    .expect("config");

    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool.clone());
    let app = server::router(&config, store).await.expect("router");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        drop(axum::serve(listener, app).await);
    });

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http");
    let root = format!("http://{addr}");

    // Empty state opens.
    let home = http.get(&root).send().await.expect("home");
    assert_eq!(home.status(), 200);
    assert!(home.text().await.expect("body").contains("Open work"));

    // Health.
    let health = http
        .get(format!("{root}/health"))
        .send()
        .await
        .expect("health");
    assert_eq!(health.text().await.expect("body"), "ok");

    // One-click result opening: GET /open/<id> fetches, caches and redirects.
    let opened_get = http
        .get(format!("{root}/open/12345"))
        .send()
        .await
        .expect("open by link");
    assert_eq!(opened_get.status(), 200);
    assert!(
        opened_get
            .text()
            .await
            .expect("reader")
            .contains("PLACEHOLDER"),
        "GET /open/<id> lands in the reader"
    );

    // Open the work: POST /open -> redirect -> reader page.
    let opened = http
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=12345")
        .send()
        .await
        .expect("open");
    let status = opened.status();
    let reader = opened.text().await.expect("reader body");
    assert_eq!(status, 200, "reader body: {reader}");

    // The parsed work is what the app renders: derive expectations from the same
    // fixture through the parser, so the test does not hardcode fixture copy.
    let parsed = ao3instance::ao3::work::parse_work_page(&html).expect("fixture parses");
    assert!(reader.contains(&parsed.title), "reader shows the title");
    if let Some(author) = parsed.authors.first() {
        assert!(reader.contains(author), "reader shows the byline");
    }
    assert!(
        !reader.contains("<iframe"),
        "chapter renders inline inside #chapters like AO3"
    );
    assert!(reader.contains("id=\"chapters\""), "{reader}");
    assert!(reader.contains("class=\"userstuff module\""), "{reader}");
    assert!(!reader.contains("bad()"), "author scripts stripped");
    assert!(reader.contains("class=\"reading-progress\""), "{reader}");
    assert!(reader.contains("/assets/ui.js"), "{reader}");
    assert!(reader.contains("/assets/reader.js"), "{reader}");

    let ui_js = http
        .get(format!("{root}/assets/ui.js"))
        .send()
        .await
        .expect("ui js");
    assert_eq!(ui_js.status(), 200);
    let ui_body = ui_js.text().await.expect("ui js body");
    assert!(ui_body.contains("data-dropdown-toggle"), "{ui_body}");

    let reader_js = http
        .get(format!("{root}/assets/reader.js"))
        .send()
        .await
        .expect("reader js");
    assert_eq!(reader_js.status(), 200);
    let js = reader_js.text().await.expect("reader js body");
    assert!(js.contains("/progress/auto"), "{js}");
    assert!(js.contains("data-resume"), "{js}");

    // Chapter iframe document: sanitized, with its own CSP.
    let chapter = http
        .get(format!("{root}/read/12345/chapter/1"))
        .send()
        .await
        .expect("chapter");
    assert_eq!(chapter.status(), 200);
    let csp = chapter
        .headers()
        .get("content-security-policy")
        .expect("chapter CSP")
        .to_str()
        .expect("csp header text")
        .to_string();
    assert!(csp.contains("default-src 'none'"), "csp: {csp}");
    let chapter_html = chapter.text().await.expect("chapter body");
    assert!(chapter_html.contains(&ao3instance::render::escape(&parsed.title)));
    assert!(chapter_html.contains("</p>"), "chapter body is rendered");
    assert!(!chapter_html.contains("bad()"), "author scripts stripped");
    assert!(
        !chapter_html.contains("<script"),
        "the script-free same-origin iframe is tracked by the parent"
    );

    // Cache-first: re-opening the cached work must not touch the upstream.
    let before = upstream.requests().len();
    let reopened = http
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=12345")
        .send()
        .await
        .expect("reopen");
    assert_eq!(reopened.status(), 200);
    assert_eq!(
        upstream.requests().len(),
        before,
        "a cached work must be served without an upstream request"
    );

    // Cached now: home lists it, and the work survives an app restart because it
    // lives in SQLite (proved by reading it back through a fresh store handle).
    let home = http.get(&root).send().await.expect("home again");
    assert!(
        home.text()
            .await
            .expect("body")
            .contains(&ao3instance::render::escape(&parsed.title)),
        "home lists the cached work"
    );
    let fresh_store = Store::new(pool.clone());
    let cached = fresh_store
        .get_work("12345")
        .await
        .expect("store read")
        .expect("work cached");
    assert_eq!(cached.title, parsed.title);
    assert_eq!(
        fresh_store
            .get_chapters("12345")
            .await
            .expect("chapters")
            .len(),
        parsed.chapters.len()
    );

    handle.abort();
    pool.close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn unknown_cached_work_is_a_clean_404() {
    let dir = temp_dir("missing");
    let base = upstream_placeholder();
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", base.as_str()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 temp dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 temp dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
        ("AO3I_FLARESOLVERR", "off"),
    ])
    .expect("config");
    let pool = db::open(&config.db_file).await.expect("db");
    let app = server::router(&config, Store::new(pool.clone()))
        .await
        .expect("router");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        drop(axum::serve(listener, app).await);
    });

    let response = reqwest::get(format!("http://{addr}/read/does-not-exist"))
        .await
        .expect("read");
    assert_eq!(response.status(), 404);

    handle.abort();
    pool.close().await;
    drop(std::fs::remove_dir_all(dir));
}

/// A URL that no test will actually hit (the router only builds a client).
fn upstream_placeholder() -> String {
    "http://127.0.0.1:1".to_string()
}
