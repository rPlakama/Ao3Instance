#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Phase 2.5: the host-provided FlareSolverr fallback.
//!
//! The solver is a mock HTTP service here; the app must never spawn it, never
//! use it for writes, keep its budget, and never persist a clearance in clear.

mod support;

use std::time::Duration;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use serde_json::json;
use support::harness::http;
use support::{Response, Server};

fn fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("fixture")
}

fn probe_response() -> Response {
    Response::json(r#"{"status":"ok","version":"3.5.2"}"#)
}

fn solve_response(upstream_url: &str, work_id: &str) -> Response {
    let html = fixture();
    let body = json!({
        "status": "ok",
        "message": "",
        "solution": {
            "url": format!("{upstream_url}/works/{work_id}?view_full_work=true"),
            "status": 200,
            "response": html,
            "userAgent": "Mozilla/5.0 solver-browser",
            "cookies": [
                {"name": "cf_clearance", "value": "clear-secret", "domain": "127.0.0.1", "path": "/"},
                {"name": "_otwarchive_session", "value": "session-secret", "domain": "127.0.0.1", "path": "/"}
            ]
        }
    });
    Response::json(body.to_string())
}

async fn serve(
    dir: &std::path::Path,
    upstream: &str,
    solver_url: &str,
    mode: &str,
) -> (String, Store, tokio::task::JoinHandle<()>) {
    let pairs = vec![
        ("AO3I_BASE_URL", upstream.to_string()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir").to_string()),
        (
            "XDG_CONFIG_HOME",
            dir.to_str().expect("utf8 dir").to_string(),
        ),
        ("AO3I_MIN_SPACING_MS", "0".to_string()),
        ("AO3I_FLARESOLVERR", mode.to_string()),
        ("AO3I_FLARESOLVERR_URL", solver_url.to_string()),
        ("AO3I_FLARESOLVERR_SEND_COOKIES", "true".to_string()),
    ];
    let config = Config::from_pairs(pairs).expect("config");
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store.clone())
        .await
        .expect("router");
    let (root, handle) = support::serve(app).await;
    (root, store, handle)
}

async fn post_form(root: &str, path: &str, body: &str) -> reqwest::Response {
    http()
        .post(format!("{root}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.to_string())
        .send()
        .await
        .expect("post")
}

#[tokio::test]
async fn solve_caches_the_page_and_keeps_the_clearance_secret() {
    let upstream = Server::start(vec![
        (
            "GET /works/777?view_full_work=true",
            Response::new(403)
                .with_header("cf-ray", "ray-777")
                .with_body("challenge body _otwarchive_session=leak"),
        ),
        (
            "GET /works/888?view_full_work=true",
            Response::ok(fixture()),
        ),
    ]);
    let solver = Server::start_sequence(vec![(
        "POST /v1",
        vec![probe_response(), solve_response(&upstream.url(), "777")],
    )]);
    let dir = support::temp_dir("solver-happy");
    let (root, store, handle) = serve(&dir, &upstream.url(), &solver.url(), "on").await;

    // Startup probe is the first solver call.
    assert_eq!(solver.requests().len(), 1, "one startup probe");

    let blocked = post_form(&root, "/open", "id=777").await;
    assert_eq!(blocked.status(), 502);
    let body = blocked.text().await.expect("body");
    assert!(body.contains("refused the request"), "{body}");
    assert!(
        body.contains("Solve &amp; cache this page") || body.contains("Solve & cache"),
        "{body}"
    );
    assert_eq!(solver.requests().len(), 1, "no automatic solve");

    let solved = post_form(&root, "/solve", "target=777&send_cookies=1").await;
    assert_eq!(solved.status(), 200, "303 followed to the reader");
    let body = solved.text().await.expect("reader");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "{body}"
    );
    assert!(body.contains("iframe"), "{body}");

    let events = store.recent_solver_events(10).await.expect("events");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].outcome, "solved");
    assert_eq!(events[0].target_path, "/works/777?view_full_work=true");

    // The clearance is stored encrypted: neither cookie value is readable.
    let clearance = store
        .load_solver_clearance()
        .await
        .expect("clearance")
        .expect("one row");
    assert_eq!(clearance.user_agent, "Mozilla/5.0 solver-browser");
    assert!(!clearance.ciphertext.contains("clear-secret"));
    assert!(!clearance.ciphertext.contains("session-secret"));
    assert!(clearance.expires_at > ao3instance::time::now_secs());

    // The clearance window now rides on direct reads too.
    let fresh = post_form(&root, "/open", "id=888").await;
    assert_eq!(fresh.status(), 200);
    let requests = upstream.requests();
    let direct = requests
        .iter()
        .find(|request| request.path.contains("/works/888"))
        .expect("direct fetch");
    assert_eq!(
        direct.header("user-agent"),
        Some("Mozilla/5.0 solver-browser")
    );
    let cookie = direct.header("cookie").unwrap_or_default();
    assert!(cookie.contains("cf_clearance=clear-secret"), "{cookie}");

    // Cooldown: a second solve is refused before any solver traffic.
    let solver_calls = solver.requests().len();
    let again = post_form(&root, "/solve", "target=888").await;
    assert_eq!(again.status(), 429);
    assert_eq!(
        solver.requests().len(),
        solver_calls,
        "budget must not call out"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn absent_solver_degrades_without_hanging() {
    let upstream = Server::start(vec![(
        "GET /works/999?view_full_work=true",
        Response::new(403),
    )]);
    let dir = support::temp_dir("solver-absent");
    let (root, store, handle) = serve(&dir, &upstream.url(), "http://127.0.0.1:1", "on").await;

    let start = std::time::Instant::now();
    let blocked = post_form(&root, "/open", "id=999").await;
    assert_eq!(blocked.status(), 502);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "an absent solver must not hang the page"
    );
    let body = blocked.text().await.expect("body");
    assert!(body.contains("can help when it is reachable"), "{body}");

    let solve = post_form(&root, "/solve", "target=999").await;
    assert_eq!(solve.status(), 503);
    let body = solve.text().await.expect("body");
    assert!(body.contains("unavailable"), "{body}");

    let events = store.recent_solver_events(10).await.expect("events");
    assert_eq!(events[0].outcome, "unavailable");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn captcha_is_typed_and_recorded() {
    let upstream = Server::start(vec![(
        "GET /works/555?view_full_work=true",
        Response::new(403),
    )]);
    let solver = Server::start_sequence(vec![(
        "POST /v1",
        vec![
            probe_response(),
            Response::json(
                r#"{"status":"error","message":"Captcha detected but no automatic solver is configured.","solution":null}"#,
            ),
        ],
    )]);
    let dir = support::temp_dir("solver-captcha");
    let (root, store, handle) = serve(&dir, &upstream.url(), &solver.url(), "on").await;

    let _ = post_form(&root, "/open", "id=555").await;
    let solve = post_form(&root, "/solve", "target=555").await;
    assert_eq!(solve.status(), 502);
    let body = solve.text().await.expect("body");
    assert!(body.contains("captcha"), "{body}");

    let events = store.recent_solver_events(10).await.expect("events");
    assert_eq!(events[0].outcome, "captcha");

    // The failure is also a redacted HTTP diagnostic with the solver transport.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let errors = store
            .recent_http_errors(10, Some("upstream"))
            .await
            .expect("errors");
        if errors
            .iter()
            .any(|row| row.transport == "solver" && row.error_kind == "solver_captcha")
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{errors:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn off_mode_never_probes_and_refuses_to_solve() {
    let upstream = Server::start(vec![(
        "GET /works/321?view_full_work=true",
        Response::new(403),
    )]);
    let solver = Server::start(vec![("POST /v1", probe_response())]);
    let dir = support::temp_dir("solver-off");
    let (root, _store, handle) = serve(&dir, &upstream.url(), &solver.url(), "off").await;

    assert!(solver.requests().is_empty(), "off mode must not probe");
    let _ = post_form(&root, "/open", "id=321").await;
    let solve = post_form(&root, "/solve", "target=321").await;
    assert_eq!(solve.status(), 403);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
