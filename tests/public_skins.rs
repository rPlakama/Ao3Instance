#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Importing public AO3 site skins as local themes.

mod support;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::http;
use support::{Response, Server};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}")).expect("fixture")
}

async fn serve(
    upstream: &Server,
    dir: &std::path::Path,
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
async fn public_skins_can_be_listed_imported_and_removed() {
    let upstream = Server::start(vec![
        (
            "GET /skins".to_string(),
            Response::ok(fixture("ao3_skins.html")),
        ),
        (
            "GET /skins/942".to_string(),
            Response::ok(fixture("ao3_skin.html")),
        ),
    ]);
    let dir = support::temp_dir("public-skins");
    let (root, store, handle) = serve(&upstream, &dir).await;
    let client = http();

    let page = client
        .get(format!("{root}/themes"))
        .send()
        .await
        .expect("themes")
        .text()
        .await
        .expect("body");
    assert!(page.contains("Chained Melody"), "{page}");
    assert!(page.contains("Use this skin"), "{page}");

    let applied = client
        .post(format!("{root}/themes/apply"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("href=%2Fskins%2F942")
        .send()
        .await
        .expect("apply");
    assert_eq!(applied.status(), 200);
    let home = client
        .get(&root)
        .send()
        .await
        .expect("home")
        .text()
        .await
        .expect("home body");
    assert!(home.contains("data-skin=\"public\""), "{home}");
    assert!(home.contains("id=\"public-skin\""), "{home}");
    assert!(home.contains("background: #123"), "{home}");
    assert_eq!(
        store.get_setting("display.skin").await.expect("setting"),
        Some("public".to_string())
    );

    let cleared = client
        .post(format!("{root}/themes/clear"))
        .send()
        .await
        .expect("clear");
    assert_eq!(cleared.status(), 200);
    let home = client
        .get(&root)
        .send()
        .await
        .expect("home")
        .text()
        .await
        .expect("home body");
    assert!(!home.contains("id=\"public-skin\""), "{home}");
    assert!(home.contains("data-skin=\"default\""), "{home}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
