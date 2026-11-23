#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! The offline/edge failure matrix: reads never hang, writes explain themselves.

mod support;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::{config_for, http, seed_work};
use support::{Response, Server};

async fn serve_app(
    dir: &std::path::Path,
    base: &str,
) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", base),
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
async fn cached_work_survives_an_unreachable_upstream() {
    let dir = support::temp_dir("matrix-offline");
    // Port 1 refuses connections: the "AO3 is down" case.
    let (root, store, handle) = serve_app(&dir, "http://127.0.0.1:1").await;
    seed_work(&store).await;
    let client = http();

    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("cached read");
    assert_eq!(
        reader.status(),
        200,
        "cached work must read without upstream"
    );
    let body = reader.text().await.expect("body");
    assert!(body.contains("iframe"), "chapter content expected");

    let search = client
        .get(format!("{root}/search?local=1&q=PLACEHOLDER"))
        .send()
        .await
        .expect("offline search");
    assert_eq!(search.status(), 200);
    assert!(search.text().await.expect("body").contains("PLACEHOLDER"));

    let audit = client
        .get(format!("{root}/audit"))
        .send()
        .await
        .expect("audit");
    assert_eq!(audit.status(), 200);

    let new_work = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=999")
        .send()
        .await
        .expect("uncached open");
    assert_eq!(new_work.status(), 502);
    let body = new_work.text().await.expect("error body");
    assert!(
        body.contains("request failed") || body.contains("network"),
        "{body}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn blocked_and_redirected_upstreams_are_explained() {
    let upstream = Server::start(vec![
        ("GET /works/111?view_full_work=true", Response::new(403)),
        (
            "GET /works/222?view_full_work=true",
            Response::redirect("/users/login?return_to=%2Fworks%2F222"),
        ),
        (
            "GET /works/333?view_full_work=true",
            Response::new(429).with_header("retry-after", "30"),
        ),
    ]);
    let dir = support::temp_dir("matrix-edge");
    let (root, _store, handle) = {
        let config = config_for(&dir, &upstream.url(), false);
        let pool = db::open(&config.db_file).await.expect("db");
        let store = Store::new(pool);
        let app = server::router(&config, store.clone())
            .await
            .expect("router");
        let (root, handle) = support::serve(app).await;
        (root, store, handle)
    };
    let client = http();

    for (id, needle) in [
        ("111", "refused the request"),
        ("222", "redirected this work"),
        ("333", "rate limiting"),
    ] {
        let response = client
            .post(format!("{root}/open"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("id={id}"))
            .send()
            .await
            .expect("open");
        assert_eq!(response.status(), 502, "id {id} must be a readable error");
        let body = response.text().await.expect("body");
        assert!(body.contains(needle), "id {id}: {body}");
    }

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
