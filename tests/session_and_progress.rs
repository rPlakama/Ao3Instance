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
        ("AO3I_FLARESOLVERR", "off"),
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

    // The progress beacon endpoint accepts sub-percent float updates and restores them.
    let precise = client
        .post(format!("{root}/progress/auto"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("work_id=12345&chapter=2&pct=42.75")
        .send()
        .await
        .expect("precise beacon");
    assert_eq!(precise.status(), 204);
    let reader_exact = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader exact")
        .text()
        .await
        .expect("reader exact body");
    assert!(
        reader_exact.contains("data-resume-exact=\"42.75\""),
        "{reader_exact}"
    );
    assert!(
        reader_exact.contains("data-resume-target=\"1\""),
        "{reader_exact}"
    );

    // Returning via an explicit `?chapter=<saved>` link (Continue Reading, Resume banner,
    // dashboard, reading history, Save Place redirect) also auto-scrolls to the saved spot.
    let resumed_by_chapter = client
        .get(format!("{root}/read/12345?chapter=2"))
        .send()
        .await
        .expect("reader explicit saved chapter")
        .text()
        .await
        .expect("reader explicit saved chapter body");
    assert!(
        resumed_by_chapter.contains("data-resume-exact=\"42.75\""),
        "{resumed_by_chapter}"
    );
    assert!(
        resumed_by_chapter.contains("data-resume-target=\"1\""),
        "{resumed_by_chapter}"
    );

    // Entire Work mode also anchors onto the saved chapter at the exact saved percentage.
    let full_work_resume = client
        .get(format!("{root}/read/12345?view_full_work=true"))
        .send()
        .await
        .expect("full work resume")
        .text()
        .await
        .expect("full work resume body");
    assert!(
        full_work_resume.contains(
            "id=\"chapter-1\" data-work-id=\"12345\" data-chapter=\"1\" data-total-chapters=\"2\" data-resume=\"0\" data-resume-exact=\"0.00\">"
        ),
        "{full_work_resume}"
    );
    assert!(
        full_work_resume.contains(
            "id=\"chapter-2\" data-work-id=\"12345\" data-chapter=\"2\" data-total-chapters=\"2\" data-resume=\"43\" data-resume-exact=\"42.75\" data-resume-target=\"1\">"
        ),
        "{full_work_resume}"
    );

    // Save Place preserves exact scroll precision and redirects back with auto-scroll intact.
    let saved_exact = client
        .post(format!("{root}/progress"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("work_id=12345&chapter=2&pct=42.75")
        .send()
        .await
        .expect("save exact progress");
    assert_eq!(saved_exact.status(), 200);
    let saved_exact_body = saved_exact.text().await.expect("saved exact body");
    assert!(
        saved_exact_body.contains("data-resume-exact=\"42.75\""),
        "{saved_exact_body}"
    );
    assert!(
        saved_exact_body.contains("data-resume-target=\"1\""),
        "{saved_exact_body}"
    );

    // Offline reading history preserves work title and authors for progress-only works.
    let history = store
        .list_reading_history("")
        .await
        .expect("reading history");
    assert_eq!(history.len(), 1);
    assert!(
        history[0].title.is_some(),
        "title preserved: {:?}",
        history[0]
    );
    assert!(
        !history[0].authors.is_empty(),
        "authors preserved: {:?}",
        history[0]
    );

    // Cross-site mutating requests are blocked by the same-origin CSRF guard.
    let cross_site = client
        .post(format!("{root}/progress/auto"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("sec-fetch-site", "cross-site")
        .body("work_id=12345&chapter=2&pct=10")
        .send()
        .await
        .expect("cross site");
    assert_eq!(cross_site.status(), 403);

    let bad_origin = client
        .post(format!("{root}/progress/auto"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "https://evil.example")
        .body("work_id=12345&chapter=2&pct=10")
        .send()
        .await
        .expect("bad origin");
    assert_eq!(bad_origin.status(), 403);

    // The beacon endpoint clamps out-of-range automatic updates.
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

    // Reading an earlier chapter offers to resume the saved one and does not auto-scroll.
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
    assert!(
        !reader.contains("data-resume-target=\"1\""),
        "other chapters must not auto-scroll: {reader}"
    );
    assert!(reader.contains("data-resume-exact=\"0.00\""), "{reader}");

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

#[tokio::test]
async fn restored_session_survives_restart_and_allows_writes_and_persists_rotation() {
    let dir = support::temp_dir("session-restart");
    let routes = vec![
        (
            "GET /users/someone/subscriptions".to_string(),
            Response::ok(
                r#"<html><head><meta content="meta-csrf" name="csrf-token"></head><body>subs</body></html>"#,
            )
            .with_cookie("_otwarchive_session=rotated-after-restart; Path=/; HttpOnly"),
        ),
        (
            "GET /token_dispenser.json".to_string(),
            Response::json(r#"{"token":"fresh-csrf"}"#),
        ),
        (
            "POST /kudos.js".to_string(),
            Response::json("{}").with_status(201),
        ),
    ];
    let upstream = Server::start(routes);

    // First instance: persist a session carrying remember_user_token.
    {
        let (root, _store, handle) = start(&dir, &upstream).await;
        let client = http();
        let boundary = "----ao3testboundary";
        let cookie_txt = "\
# Netscape HTTP Cookie File
#HttpOnly_.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\t_otwarchive_session\ts1
.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\tuser_credentials\t1
.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\tremember_user_token\trem-tok-99
";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"account\"\r\n\r\n\
             someone\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"cookie_file\"; filename=\"cookies.txt\"\r\n\
             Content-Type: text/plain\r\n\r\n\
             {cookie_txt}\r\n\
             --{boundary}--\r\n"
        );
        let imported = client
            .post(format!("{root}/session/import-file"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .expect("import file");
        assert_eq!(imported.status(), 200);
        let page = imported.text().await.expect("page");
        assert!(page.contains("Cookie file imported for someone"), "{page}");
        handle.abort();
    }

    // Second instance (simulating a NixOS update / service restart) on the same DB + key.
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", upstream.url().as_str()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
        ("AO3I_WRITE_ENABLED", "true"),
        ("AO3I_FLARESOLVERR", "off"),
    ])
    .expect("config");
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store.clone())
        .await
        .expect("router");
    let (root, handle) = support::serve(app).await;
    let client = http();

    // Writes work immediately across the restart without signing in again.
    let kudos = client
        .post(format!("{root}/write/kudos"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("target_id=777&confirm=yes")
        .send()
        .await
        .expect("kudos");
    assert_eq!(kudos.status(), 200);
    assert!(
        store.has_kudos("someone", "777").await.expect("has kudos"),
        "restored session must activate and complete writes across restarts"
    );

    // Session page shows active state, and rotated cookies + remember_user_token stayed persisted.
    let session_html = client
        .get(format!("{root}/session"))
        .send()
        .await
        .expect("session page")
        .text()
        .await
        .expect("body");
    assert!(session_html.contains("active"), "{session_html}");
    assert!(session_html.contains("type=\"file\""), "{session_html}");

    let saved = store
        .load_session()
        .await
        .expect("load")
        .expect("persisted");
    let key = SecretKey::load_or_create(&config.key_file).expect("key");
    let jar = Jar::decrypt(&key, &saved.jar_enc).expect("decrypt");
    assert_eq!(
        jar.get("_otwarchive_session"),
        Some("rotated-after-restart"),
        "rotated session cookie must be persisted back to SQLite"
    );
    assert_eq!(jar.get("remember_user_token"), Some("rem-tok-99"));

    // Probe button also verifies & stays active.
    let probed = client
        .post(format!("{root}/session/probe"))
        .send()
        .await
        .expect("probe")
        .text()
        .await
        .expect("body");
    assert!(probed.contains("Session verified."), "{probed}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn cookie_file_upload_autodetects_account_when_blank() {
    let dir = support::temp_dir("cookie-file-autodetect");
    let upstream = Server::start(vec![
        (
            "GET /".to_string(),
            Response::ok(
                r#"<html><body><nav id="greeting"><p>Hi, <a href="/users/file_reader">file_reader</a>!</p></nav></body></html>"#,
            ),
        ),
        (
            "GET /users/file_reader/subscriptions".to_string(),
            Response::ok("<html>subscriptions</html>"),
        ),
        (
            "GET /token_dispenser.json".to_string(),
            Response::json(r#"{"token":"tok-1"}"#),
        ),
    ]);
    let (root, store, handle) = start(&dir, &upstream).await;
    let client = http();

    let boundary = "----ao3jsonboundary";
    let json_cookies = r#"[
        {"name": "_otwarchive_session", "value": "json-s1", "domain": ".archiveofourown.org"},
        {"name": "user_credentials", "value": "1", "domain": ".archiveofourown.org"},
        {"name": "remember_user_token", "value": "rem-json", "domain": ".archiveofourown.org"}
    ]"#;
    let body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"account\"\r\n\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"cookie_file\"; filename=\"ao3.json\"\r\n\
         Content-Type: application/json\r\n\r\n\
         {json_cookies}\r\n\
         --{boundary}--\r\n"
    );
    let response = client
        .post(format!("{root}/session/import-file"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .expect("import json file");
    assert_eq!(response.status(), 200);
    let page = response.text().await.expect("page");
    assert!(
        page.contains("Cookie file imported for file_reader"),
        "{page}"
    );
    let saved = store
        .load_session()
        .await
        .expect("load")
        .expect("persisted");
    assert_eq!(saved.account, "file_reader");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
