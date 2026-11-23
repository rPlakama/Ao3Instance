#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Local-only access history: reads are recorded and listed with a range filter.

mod support;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::{http, seed_work};

async fn serve(dir: &std::path::Path) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", "http://127.0.0.1:1".to_string()),
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
async fn reads_and_caches_are_kept_in_local_history() {
    let dir = support::temp_dir("history");
    let (root, store, handle) = serve(&dir).await;
    seed_work(&store).await;
    let client = http();

    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("read");
    assert_eq!(reader.status(), 200);

    let history = client
        .get(format!("{root}/history"))
        .send()
        .await
        .expect("history");
    assert_eq!(history.status(), 200);
    let body = history.text().await.expect("body");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "{body}"
    );
    assert!(body.contains("Local cache history"), "{body}");
    assert!(body.contains("read"), "{body}");
    assert!(body.contains("Today"), "{body}");

    let today = client
        .get(format!("{root}/history?range=today"))
        .send()
        .await
        .expect("today");
    let today = today.text().await.expect("body");
    assert!(
        today.contains("PLACEHOLDER: The Map and the Mirror"),
        "{today}"
    );
    assert!(
        today.contains("<option value=\"today\" selected>"),
        "{today}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
