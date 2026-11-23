#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Session persistence/usage and reading progress, end to end against a mock AO3.

mod support;

use std::time::Duration;

use ao3instance::ao3::jar::Jar;
use ao3instance::ao3::work::parse_work_page;
use ao3instance::config::Config;
use ao3instance::crypto::SecretKey;
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

fn session_routes(work_id: &str) -> Vec<(String, Response)> {
    vec![
        (
            "GET /users/someone/subscriptions".to_string(),
            Response::ok("<html>subscriptions</html>"),
        ),
        (
            "GET /token_dispenser.json".to_string(),
            Response::json(r#"{"token":"action-token"}"#),
        ),
        (
            format!("GET /works/{work_id}?view_full_work=true"),
            Response::ok(fixture()).with_header("content-type", "text/html; charset=utf-8"),
        ),
    ]
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

#[tokio::test]
async fn imported_session_is_persisted_and_used_for_fetches() {
    let dir = support::temp_dir("session-import");
    let upstream = Server::start(session_routes("555"));
    let (root, store, handle) = start(&dir, &upstream).await;
    let client = http();

    // Import cookies: the session is verified against the mock upstream.
    let imported = client
        .post(format!("{root}/session/import"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("account=someone&session_cookie=s1&credentials_cookie=1")
        .send()
        .await
        .expect("import");
    assert_eq!(imported.status(), 200, "follows the redirect to /session");
    let page = imported.text().await.expect("session page");
    assert!(page.contains("Session imported and verified."), "{page}");
    assert!(page.contains("someone"));

    // It is stored encrypted, not as a plaintext cookie.
    let saved = store
        .load_session()
        .await
        .expect("load session")
        .expect("persisted");
    assert_eq!(saved.account, "someone");
    assert!(!saved.jar_enc.contains("_otwarchive_session=s1"));
    let config = config_for(&dir, &upstream.url());
    let key = SecretKey::load_or_create(&config.key_file).expect("key");
    let jar = Jar::decrypt(&key, &saved.jar_enc).expect("decrypt");
    assert!(jar.is_complete());

    // Opening a work uses that jar.
    let opened = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=555")
        .send()
        .await
        .expect("open");
    assert_eq!(opened.status(), 200);

    let work_request = upstream
        .requests()
        .into_iter()
        .find(|request| request.path.starts_with("/works/555"))
        .expect("work fetch");
    let cookie = work_request.header("cookie").unwrap_or_default();
    assert!(
        cookie.contains("_otwarchive_session=s1"),
        "signed-in fetch must carry the jar: {cookie}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn progress_is_saved_listed_and_offered_as_resume() {
    let dir = support::temp_dir("progress");
    let upstream = Server::start(Vec::<(String, Response)>::new());
    let (root, store, handle) = start(&dir, &upstream).await;

    // Seed the cache directly from the checked-in fixture.
    let mut page = parse_work_page(&fixture()).expect("fixture parses");
    page.ao3_id = "12345".into();
    store.upsert_work(&page, "hash").await.expect("seed work");

    let client = http();
    let saved = client
        .post(format!("{root}/progress"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("work_id=12345&chapter=2&pct=42")
        .send()
        .await
        .expect("save progress");
    assert_eq!(saved.status(), 200);
    assert_eq!(
        store.get_progress("12345").await.expect("progress"),
        Some(ao3instance::store::Progress {
            chapter_idx: 2,
            pct: 42
        })
    );

    // Home shows a continue link with the saved chapter.
    let home = client
        .get(&root)
        .send()
        .await
        .expect("home")
        .text()
        .await
        .expect("home body");
    assert!(home.contains("continue ch. 2 · 71%"), "{home}");

    // The reader restores the saved chapter and inside-chapter percentage.
    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("reader body");
    assert!(reader.contains("71% read"), "{reader}");
    assert!(reader.contains("data-resume=\"42\""), "{reader}");

    // The iframe beacon endpoint accepts a clamped automatic update.
    let beacon = client
        .post(format!("{root}/progress/auto"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("work_id=12345&chapter=2&pct=150")
        .send()
        .await
        .expect("beacon");
    assert_eq!(beacon.status(), 204);
    assert_eq!(
        store
            .get_progress("12345")
            .await
            .expect("progress")
            .unwrap()
            .pct,
        100
    );

    // Reading an earlier chapter offers to resume the saved one.
    let reader = client
        .get(format!("{root}/read/12345?chapter=1"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("reader body");
    assert!(reader.contains("You were at chapter 2"), "{reader}");
    assert!(reader.contains("chapter=2"));

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn adult_gate_is_explained_not_reported_as_drift() {
    let dir = support::temp_dir("adult-gate");
    let gate = "<html><body><div id=\"adult\">This work could have adult content. Proceed</div></body></html>";
    let upstream = Server::start(vec![(
        "GET /works/666?view_full_work=true",
        Response::ok(gate).with_header("content-type", "text/html; charset=utf-8"),
    )]);
    let (root, _store, handle) = start(&dir, &upstream).await;

    let response = http()
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=666")
        .send()
        .await
        .expect("open adult work");
    assert_eq!(response.status(), 502);
    let body = response.text().await.expect("error body");
    assert!(body.contains("adult-content gate"), "{body}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn authenticated_home_renders_ao3_modules() {
    let mut routes = session_routes("555");
    routes.push((
        "GET /".to_string(),
        Response::ok(
            std::fs::read_to_string("tests/fixtures/ao3_home.html").expect("home fixture"),
        )
        .with_header("content-type", "text/html; charset=utf-8"),
    ));
    let dir = support::temp_dir("home-modules");
    let upstream = Server::start(routes);
    let (root, _store, handle) = start(&dir, &upstream).await;
    let client = http();

    let imported = client
        .post(format!("{root}/session/import"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("account=someone&session_cookie=s1&credentials_cookie=1")
        .send()
        .await
        .expect("import");
    assert_eq!(imported.status(), 200);

    let home = client
        .get(&root)
        .send()
        .await
        .expect("home")
        .text()
        .await
        .expect("home body");
    assert!(home.contains("Assertive Emilia (Re:Zero)"), "{home}");
    assert!(home.contains("Five Things Hobgirl Said"), "{home}");
    assert!(home.contains("Re: Zero Gate"), "{home}");
    assert!(home.contains("Internet123 on Chapter 4"), "{home}");
    assert!(home.contains("Hungry man thinks of bread"), "{home}");
    assert!(home.contains("Refresh AO3 home"), "{home}");
    assert!(
        upstream
            .requests()
            .iter()
            .any(|request| request.path == "/"),
        "the AO3 home page was fetched once"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
