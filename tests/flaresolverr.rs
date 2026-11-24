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

fn solve_search_response(upstream_url: &str, path: &str) -> Response {
    let html = std::fs::read_to_string("tests/fixtures/search_results.html").expect("fixture");
    let body = json!({
        "status": "ok",
        "message": "",
        "solution": {
            "url": format!("{upstream_url}{path}"),
            "status": 200,
            "response": html,
            "userAgent": "Mozilla/5.0 solver-browser",
            "cookies": [
                {"name": "cf_clearance", "value": "clear-secret", "domain": "127.0.0.1", "path": "/"}
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

async fn serve_with_writes(
    dir: &std::path::Path,
    upstream: &str,
    solver_url: &str,
) -> (String, Store, tokio::task::JoinHandle<()>) {
    let pairs = vec![
        ("AO3I_BASE_URL", upstream.to_string()),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir").to_string()),
        (
            "XDG_CONFIG_HOME",
            dir.to_str().expect("utf8 dir").to_string(),
        ),
        ("AO3I_MIN_SPACING_MS", "0".to_string()),
        ("AO3I_WRITE_ENABLED", "true".to_string()),
        ("AO3I_FLARESOLVERR", "auto".to_string()),
        ("AO3I_FLARESOLVERR_URL", solver_url.to_string()),
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
async fn solver_is_always_used_when_available_and_keeps_clearance_secret() {
    let upstream = Server::start(vec![
        (
            "GET /works/777?view_full_work=true",
            Response::new(403)
                .with_header("cf-ray", "ray-777")
                .with_body("challenge body _otwarchive_session=leak"),
        ),
        ("GET /works/888?view_full_work=true", Response::new(403)),
    ]);
    let solver = Server::start_sequence(vec![(
        "POST /v1",
        vec![
            probe_response(),
            solve_response(&upstream.url(), "777"),
            solve_response(&upstream.url(), "888"),
        ],
    )]);
    let dir = support::temp_dir("solver-happy");
    let (root, store, handle) = serve(&dir, &upstream.url(), &solver.url(), "auto").await;

    // Startup probe is the first solver call.
    assert_eq!(solver.requests().len(), 1, "one startup probe");

    // When the solver is available, /open uses it automatically without trying direct first.
    let opened = post_form(&root, "/open", "id=777").await;
    assert_eq!(opened.status(), 200, "303 followed to the reader");
    let body = opened.text().await.expect("reader");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "{body}"
    );
    assert!(body.contains("class=\"userstuff module\""), "{body}");
    assert!(
        upstream.requests().is_empty(),
        "direct upstream should not be called while solver is available"
    );
    assert_eq!(solver.requests().len(), 2, "probe + first work solve");

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

    // Consecutive reads keep using the solver without a cooldown lockout.
    let second = post_form(&root, "/open", "id=888").await;
    assert_eq!(second.status(), 200);
    assert!(
        upstream.requests().is_empty(),
        "direct upstream still unused while solver is available"
    );
    assert_eq!(solver.requests().len(), 3, "probe + two work solves");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn search_always_uses_solver_when_available() {
    let search_path = "/works/search?work_search%5Bquery%5D=mirror";
    let upstream = Server::start(vec![(format!("GET {search_path}"), Response::new(429))]);
    let solver = Server::start_sequence(vec![(
        "POST /v1",
        vec![
            probe_response(),
            solve_search_response(&upstream.url(), search_path),
        ],
    )]);
    let dir = support::temp_dir("solver-search");
    let (root, _store, handle) = serve(&dir, &upstream.url(), &solver.url(), "auto").await;

    let response = http()
        .get(format!("{root}/works/search?q=mirror"))
        .send()
        .await
        .expect("search");
    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("body");
    assert!(body.contains("A Mirror of Small Things"), "{body}");
    assert!(
        upstream.requests().is_empty(),
        "search should use the solver directly when available"
    );
    assert_eq!(solver.requests().len(), 2, "probe + search solve");

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

    let solve = post_form(&root, "/open", "id=555").await;
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

#[tokio::test]
async fn login_validation_and_writes_all_use_solver_when_available() {
    let upstream = Server::empty();
    let base = upstream.url();
    let login_get = Response::json(
        json!({
            "status": "ok",
            "solution": {
                "url": format!("{base}/users/login"),
                "status": 200,
                "response": r#"<html><head><meta name="csrf-token" content="login-tok"/></head><body></body></html>"#,
                "userAgent": "Mozilla/5.0 solver-browser",
                "cookies": [
                    {"name": "_otwarchive_session", "value": "pre-login-sess", "domain": "127.0.0.1", "path": "/"}
                ]
            }
        })
        .to_string(),
    );
    let login_post = Response::json(
        json!({
            "status": "ok",
            "solution": {
                "url": format!("{base}/users/someone"),
                "status": 200,
                "response": "<html><body>Welcome someone</body></html>",
                "userAgent": "Mozilla/5.0 solver-browser",
                "cookies": [
                    {"name": "_otwarchive_session", "value": "authed-sess", "domain": "127.0.0.1", "path": "/"},
                    {"name": "user_credentials", "value": "1", "domain": "127.0.0.1", "path": "/"},
                    {"name": "cf_clearance", "value": "clear-secret", "domain": "127.0.0.1", "path": "/"}
                ]
            }
        })
        .to_string(),
    );
    let subs_get = Response::json(
        json!({
            "status": "ok",
            "solution": {
                "url": format!("{base}/users/someone/subscriptions"),
                "status": 200,
                "response": "<html><body>Subscriptions</body></html>",
                "userAgent": "Mozilla/5.0 solver-browser",
                "cookies": []
            }
        })
        .to_string(),
    );
    // Real FlareSolverr/Chromium wraps .json responses inside <pre>.
    let token_get = Response::json(
        json!({
            "status": "ok",
            "solution": {
                "url": format!("{base}/token_dispenser.json"),
                "status": 200,
                "response": r#"<html><body><pre>{"token":"solver-csrf"}</pre></body></html>"#,
                "userAgent": "Mozilla/5.0 solver-browser",
                "cookies": []
            }
        })
        .to_string(),
    );
    let kudos_post = Response::json(
        json!({
            "status": "ok",
            "solution": {
                "url": format!("{base}/kudos.js"),
                "status": 201,
                "response": r#"<html><body><pre>{}</pre></body></html>"#,
                "userAgent": "Mozilla/5.0 solver-browser",
                "cookies": []
            }
        })
        .to_string(),
    );

    let home_get = Response::json(
        json!({
            "status": "ok",
            "solution": {
                "url": format!("{base}/"),
                "status": 200,
                "response": "<html><body>Home</body></html>",
                "userAgent": "Mozilla/5.0 solver-browser",
                "cookies": []
            }
        })
        .to_string(),
    );

    let solver = Server::start_sequence(vec![(
        "POST /v1",
        vec![
            probe_response(),
            login_get,
            login_post,
            subs_get,
            token_get,
            kudos_post,
            home_get,
        ],
    )]);
    let dir = support::temp_dir("solver-all-interactions");
    let (root, store, handle) = serve_with_writes(&dir, &base, &solver.url()).await;

    // Login uses solver for GET /users/login, POST /users/login, probe, and token_dispenser.
    let login = post_form(&root, "/session/login", "account=someone&password=secret").await;
    assert_eq!(login.status(), 200);
    let login_body = login.text().await.expect("login body");
    assert!(login_body.contains("Signed in to AO3."), "{login_body}");

    // Writes (kudos) also go through FlareSolverr via request.post.
    let kudos = post_form(&root, "/write/kudos", "target_id=42&confirm=yes").await;
    assert_eq!(kudos.status(), 200);
    assert!(store.has_kudos("someone", "42").await.expect("kudos"));

    // Zero direct requests ever reached AO3.
    assert!(
        upstream.requests().is_empty(),
        "expected every interaction to use FlareSolverr, got: {:?}",
        upstream.requests()
    );
    let solver_reqs = solver.requests();
    assert_eq!(
        solver_reqs.len(),
        7,
        "probe + 4 login/validation + 1 kudos + 1 home redirect"
    );
    assert!(
        solver_reqs[2].body.contains(r#""cmd":"request.post""#)
            && solver_reqs[2].body.contains("authenticity_token=login-tok"),
        "login POST went through solver: {}",
        solver_reqs[2].body
    );
    assert!(
        solver_reqs[5].body.contains(r#""cmd":"request.post""#)
            && solver_reqs[5]
                .body
                .contains("authenticity_token=solver-csrf"),
        "kudos POST went through solver: {}",
        solver_reqs[5].body
    );

    // Logout is local-only: removes account without contacting AO3 or FlareSolverr.
    let solver_calls_before_logout = solver.requests().len();
    let logout = post_form(&root, "/session/logout", "").await;
    assert_eq!(logout.status(), 200);
    let logout_body = logout.text().await.expect("logout body");
    assert!(
        logout_body.contains("Account removed locally."),
        "{logout_body}"
    );
    assert!(
        !logout_body.contains("signed in as"),
        "account name must be cleared on logout: {logout_body}"
    );
    assert!(store.load_session().await.expect("session").is_none());
    assert_eq!(
        solver.requests().len(),
        solver_calls_before_logout,
        "logout must not contact FlareSolverr"
    );
    assert!(
        upstream.requests().is_empty(),
        "logout must not contact AO3"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn persistent_solver_session_and_auth_error_direct_fallback() {
    let upstream = Server::start(vec![(
        "POST /kudos.js",
        Response::json("{}").with_status(201),
    )]);
    let base = upstream.url();
    let solver = Server::start_sequence(vec![(
        "POST /v1",
        vec![
            // Probe advertises sessions support (like real FlareSolverr).
            Response::json(r#"{"status":"ok","version":"3.5.2","sessions":[]}"#),
            // sessions.create
            Response::json(r#"{"status":"ok","session":"ao3instance"}"#),
            // GET /users/someone/subscriptions (with reversed meta attrs + cf_clearance)
            Response::json(
                json!({
                    "status": "ok",
                    "solution": {
                        "url": format!("{base}/users/someone/subscriptions"),
                        "status": 200,
                        "response": r#"<html><head><meta content="sub-csrf" name="csrf-token"></head><body>subs</body></html>"#,
                        "userAgent": "Mozilla/5.0 solver-browser",
                        "cookies": [
                            {"name": "cf_clearance", "value": "clear-tok", "domain": "127.0.0.1", "path": "/"},
                            {"name": "_otwarchive_session", "value": "sess-1", "domain": "127.0.0.1", "path": "/"},
                            {"name": "user_credentials", "value": "1", "domain": "127.0.0.1", "path": "/"}
                        ]
                    }
                })
                .to_string(),
            ),
            // GET /token_dispenser.json
            Response::json(
                json!({
                    "status": "ok",
                    "solution": {
                        "url": format!("{base}/token_dispenser.json"),
                        "status": 200,
                        "response": r#"<html><body><pre>{"token":"solver-csrf-1"}</pre></body></html>"#,
                        "userAgent": "Mozilla/5.0 solver-browser",
                        "cookies": []
                    }
                })
                .to_string(),
            ),
            // POST /kudos.js via FlareSolverr lands on /auth_error (Origin: null quirk)
            Response::json(
                json!({
                    "status": "ok",
                    "solution": {
                        "url": format!("{base}/auth_error"),
                        "status": 200,
                        "response": "<html><body>Auth error</body></html>",
                        "userAgent": "Mozilla/5.0 solver-browser",
                        "cookies": []
                    }
                })
                .to_string(),
            ),
            // Redirect to / after kudos
            Response::json(
                json!({
                    "status": "ok",
                    "solution": {
                        "url": format!("{base}/"),
                        "status": 200,
                        "response": "<html><body>Home</body></html>",
                        "userAgent": "Mozilla/5.0 solver-browser",
                        "cookies": []
                    }
                })
                .to_string(),
            ),
            // sessions.destroy on logout
            Response::json(r#"{"status":"ok"}"#),
        ],
    )]);

    let dir = support::temp_dir("solver-session-csrf");
    let (root, store, handle) = serve_with_writes(&dir, &base, &solver.url()).await;

    let imported = post_form(
        &root,
        "/session/import",
        "account=someone&session_cookie=sess-1&credentials_cookie=1",
    )
    .await;
    assert_eq!(imported.status(), 200);

    // Kudos hits /auth_error through FlareSolverr and falls back cleanly to direct
    // Client carrying the solver's cf_clearance, browser UA, Origin, and X-CSRF-Token.
    let kudos = post_form(&root, "/write/kudos", "target_id=99&confirm=yes").await;
    assert_eq!(kudos.status(), 200);
    assert!(store.has_kudos("someone", "99").await.expect("kudos"));

    let direct_posts = upstream.requests();
    assert_eq!(direct_posts.len(), 1, "direct fallback completed the write");
    assert_eq!(direct_posts[0].path, "/kudos.js");
    assert_eq!(
        direct_posts[0].header("x-csrf-token"),
        Some("solver-csrf-1")
    );
    assert_eq!(direct_posts[0].header("origin"), Some(base.as_str()));
    assert_eq!(
        direct_posts[0].header("user-agent"),
        Some("Mozilla/5.0 solver-browser")
    );
    let cookie_hdr = direct_posts[0].header("cookie").unwrap_or_default();
    assert!(
        cookie_hdr.contains("cf_clearance=clear-tok"),
        "direct fallback carries cf_clearance: {cookie_hdr}"
    );
    assert_eq!(
        cookie_hdr.matches("cf_clearance=").count(),
        1,
        "cf_clearance must not be duplicated: {cookie_hdr}"
    );

    // Logout destroys the persistent FlareSolverr browser session.
    let _ = post_form(&root, "/session/logout", "").await;

    let solver_bodies: Vec<String> = solver.requests().into_iter().map(|r| r.body).collect();
    assert!(
        solver_bodies
            .iter()
            .any(|b| b.contains(r#""cmd":"sessions.create""#) && b.contains("ao3instance")),
        "expected sessions.create: {solver_bodies:?}"
    );
    assert!(
        solver_bodies
            .iter()
            .any(|b| b.contains(r#""cmd":"request.get""#)
                && b.contains(r#""session":"ao3instance""#)),
        "expected persistent session on request.get: {solver_bodies:?}"
    );
    assert!(
        solver_bodies
            .iter()
            .any(|b| b.contains(r#""cmd":"sessions.destroy""#) && b.contains("ao3instance")),
        "expected sessions.destroy on logout: {solver_bodies:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
