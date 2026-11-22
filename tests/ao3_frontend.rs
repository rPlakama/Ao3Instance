#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Phase 3/4 frontend baseline: every route renders the AO3-style chrome while
//! the upstream is unreachable, and the embedded stylesheet is served correctly.

mod support;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::http;

async fn serve(dir: &std::path::Path) -> (String, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", "http://127.0.0.1:1"),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
    ])
    .expect("config");
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store).await.expect("router");
    let (root, handle) = support::serve(app).await;
    (root, handle)
}

#[tokio::test]
async fn every_offline_segment_renders_the_shared_chrome() {
    let dir = support::temp_dir("frontend-walk");
    let (root, handle) = serve(&dir).await;
    let client = http();

    for path in [
        "/",
        "/search?q=hello",
        "/works/search?q=hello",
        "/users/login",
        "/session",
        "/audit",
        "/diagnostics",
    ] {
        let response = client
            .get(format!("{root}{path}"))
            .send()
            .await
            .unwrap_or_else(|error| panic!("GET {path}: {error}"));
        assert_eq!(response.status(), 200, "GET {path}");
        let body = response.text().await.expect("body");
        assert!(body.contains("id=\"header\""), "{path}: header");
        assert!(body.contains("id=\"footer\""), "{path}: footer");
        assert!(body.contains("class=\"primary navigation\""), "{path}: nav");
        assert!(body.contains("/assets/ao3.css"), "{path}: stylesheet link");
        assert!(
            !body.contains("archiveofourown.org/works/"),
            "{path}: offline pages must not link out from the chrome"
        );
    }

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn stylesheet_is_embedded_and_cache_friendly() {
    let dir = support::temp_dir("frontend-css");
    let (root, handle) = serve(&dir).await;

    let response = http()
        .get(format!("{root}/assets/ao3.css"))
        .send()
        .await
        .expect("css");
    assert_eq!(response.status(), 200);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/css"), "{content_type}");
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=3600")
    );
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    let css = response.text().await.expect("css body");
    assert!(css.contains("--ao3-red: #900"), "AO3 palette");
    assert!(css.contains("ul.primary.navigation"), "primary nav");
    assert!(css.contains("work.blurb.group"), "blurb component");
    assert!(
        css.contains("@media (prefers-color-scheme: dark)"),
        "reversi"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn aliases_are_offline_and_uncached_works_stay_a_miss() {
    let dir = support::temp_dir("frontend-aliases");
    let (root, handle) = serve(&dir).await;
    let client = http();

    let login = client
        .get(format!("{root}/users/login"))
        .send()
        .await
        .expect("login alias");
    assert_eq!(login.status(), 200);
    assert!(login.text().await.expect("body").contains("AO3 session"));

    let session = client
        .get(format!("{root}/session"))
        .send()
        .await
        .expect("session");
    assert_eq!(session.status(), 200);
    assert!(
        session
            .text()
            .await
            .expect("body")
            .contains("id=\"cookie-import\"")
    );

    let missing = client
        .get(format!("{root}/works/999999"))
        .send()
        .await
        .expect("uncached alias");
    assert_eq!(missing.status(), 404, "GET must never fetch upstream");

    // Opening an uncached work with the upstream refused is a typed 502, quickly.
    let started = std::time::Instant::now();
    let open = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=999999")
        .send()
        .await
        .expect("open");
    assert_eq!(open.status(), 502);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "offline failure must not hang"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
