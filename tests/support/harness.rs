//! Shared harness for the write-path integration tests: an app on an ephemeral
//! port, a temp SQLite file, a signed-in session, and a seeded cached work.

use std::path::Path;
use std::time::Duration;

use ao3instance::ao3::work::parse_work_page;
use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};

use super::{Response, Server, serve, temp_dir};

pub fn fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("fixture")
}

pub fn config_for(dir: &Path, base: &str, writes: bool) -> Config {
    let mut pairs = vec![
        ("AO3I_BASE_URL", base.to_string()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir").to_string()),
        (
            "XDG_CONFIG_HOME",
            dir.to_str().expect("utf8 dir").to_string(),
        ),
        ("AO3I_MIN_SPACING_MS", "0".to_string()),
    ];
    if writes {
        pairs.push(("AO3I_WRITE_ENABLED", "true".to_string()));
    }
    Config::from_pairs(pairs).expect("config")
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http")
}

/// Routes every signed-in session needs, plus whatever the test adds.
pub fn session_routes(extra: Vec<(&str, Response)>) -> Vec<(String, Response)> {
    let mut routes = vec![
        (
            "GET /users/someone/subscriptions".to_string(),
            Response::ok("<html>subscriptions</html>"),
        ),
        (
            "GET /token_dispenser.json".to_string(),
            Response::json(r#"{"token":"action-token"}"#),
        ),
    ];
    routes.extend(
        extra
            .into_iter()
            .map(|(key, response)| (key.to_string(), response)),
    );
    routes
}

pub async fn start(
    upstream: &Server,
    writes: bool,
) -> (
    String,
    Store,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    // Unique per call: tests in one binary run concurrently and must not share a
    // SQLite file.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tag = format!(
        "harness-{}",
        SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    );
    let dir = temp_dir(&tag);
    let config = config_for(&dir, &upstream.url(), writes);
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store.clone())
        .await
        .expect("router");
    let (root, handle) = serve(app).await;
    (root, store, dir, handle)
}

pub async fn sign_in(client: &reqwest::Client, root: &str) {
    let imported = client
        .post(format!("{root}/session/import"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("account=someone&session_cookie=s1&credentials_cookie=1")
        .send()
        .await
        .expect("import");
    assert_eq!(imported.status(), 200);
    let body = imported.text().await.expect("body");
    assert!(body.contains("Session imported and verified"), "{body}");
}

pub async fn seed_work(store: &Store) {
    let mut page = parse_work_page(&fixture()).expect("fixture parses");
    page.ao3_id = "12345".into();
    store.upsert_work(&page, "hash").await.expect("seed");
}
