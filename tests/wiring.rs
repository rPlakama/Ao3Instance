#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Every displayed entry that points somewhere must actually be wired. This
//! crawls the rendered pages, extracts local anchors, and probes each target.

mod support;

use std::collections::BTreeSet;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use scraper::{Html, Selector};
use support::Server;
use support::harness::{http, seed_work, session_routes, sign_in, start};

async fn serve(dir: &std::path::Path) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", "http://127.0.0.1:1"),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
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
async fn every_rendered_local_link_resolves() {
    let dir = support::temp_dir("wiring-links");
    let (root, store, handle) = serve(&dir).await;
    seed_work(&store).await;
    let client = http();

    let pages = [
        "/",
        "/works",
        "/fandoms",
        "/search?q=Mirror",
        "/read/12345",
        "/read/12345?view_full_work=true",
        "/session",
        "/dashboard",
        "/subscriptions",
        "/bookmarks",
        "/reading",
        "/audit",
        "/diagnostics",
        "/users/login",
        "/users/logout/confirm",
    ];

    let anchor = Selector::parse("a[href]").expect("anchor selector");
    let mut targets: BTreeSet<String> = BTreeSet::new();
    for page in pages {
        let body = client
            .get(format!("{root}{page}"))
            .send()
            .await
            .unwrap_or_else(|error| panic!("GET {page}: {error}"))
            .text()
            .await
            .expect("body");
        let document = Html::parse_document(&body);
        for element in document.select(&anchor) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            if !href.starts_with('/') || href.starts_with("//") || href.starts_with('#') {
                continue;
            }
            let path = href.split('#').next().unwrap_or(href);
            if !path.is_empty() && path != "/" {
                targets.insert(path.to_string());
            }
        }
    }

    assert!(
        targets
            .iter()
            .any(|target| target.starts_with("/works?tag=")),
        "tag entries are wired: {targets:?}"
    );
    assert!(
        targets
            .iter()
            .any(|target| target.starts_with("/works?author=")),
        "author entries are wired: {targets:?}"
    );
    assert!(
        targets
            .iter()
            .any(|target| target.starts_with("/works?rating=")),
        "rating entries are wired: {targets:?}"
    );

    let mut broken = Vec::new();
    for target in &targets {
        let response = client
            .get(format!("{root}{target}"))
            .send()
            .await
            .unwrap_or_else(|error| panic!("GET {target}: {error}"));
        let status = response.status().as_u16();
        if status != 200 {
            broken.push(format!("{target} -> {status}"));
        }
    }
    assert!(broken.is_empty(), "unwired entries: {broken:?}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn non_work_subscriptions_never_point_at_reader_urls() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    store
        .upsert_subscription("someone", "Work", "12345", Some(7), "active")
        .await
        .expect("work subscription");
    store
        .upsert_subscription("someone", "Tag", "fluff", None, "active")
        .await
        .expect("tag subscription");
    store
        .upsert_subscription("someone", "User", "another", None, "active")
        .await
        .expect("user subscription");

    for page in ["/subscriptions", "/dashboard"] {
        let body = client
            .get(format!("{root}{page}"))
            .send()
            .await
            .expect("page")
            .text()
            .await
            .expect("body");
        assert!(
            !body.contains("/read/fluff") && !body.contains("/read/another"),
            "{page} must not treat tags/users as works"
        );
        assert!(body.contains("fluff"), "{page} still lists the tag");
    }

    let subscriptions = client
        .get(format!("{root}/subscriptions"))
        .send()
        .await
        .expect("subscriptions")
        .text()
        .await
        .expect("body");
    assert!(
        subscriptions.contains("/works?tag=fluff"),
        "tag subscriptions link to the tag filter"
    );
    assert!(
        subscriptions.contains("/read/12345"),
        "work subscriptions link to the reader"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
