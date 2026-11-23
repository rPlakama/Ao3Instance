#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Phase 2: every upstream failure and inbound 4xx/5xx becomes one redacted row
//! in `http_error_log`, with accurate classifier, status and safe evidence.

mod support;

use std::time::{Duration, Instant};

use ao3instance::config::Config;
use ao3instance::diag::{HttpDirection, HttpErrorEvent, Transport, format_line};
use ao3instance::store::{HttpErrorRow, Store};
use ao3instance::{db, server};
use support::{Response, Server, harness::http};

async fn serve_app(
    dir: &std::path::Path,
    base: &str,
) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", base),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
        ("AO3I_FLARESOLVERR", "off"),
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

async fn wait_for(store: &Store, direction: &str, expected: usize) -> Vec<HttpErrorRow> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let rows = store
            .recent_http_errors(100, Some(direction))
            .await
            .expect("rows");
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} {direction} rows; got {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn open_work(root: &str, id: &str) -> reqwest::Response {
    http()
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("id={id}"))
        .send()
        .await
        .expect("open")
}

fn all_text(row: &HttpErrorRow) -> String {
    format!(
        "{} {} {} {} {} {} {:?} {:?} {:?} {:?}",
        row.direction,
        row.transport,
        row.method,
        row.path,
        row.error_kind,
        row.message,
        row.location_path,
        row.server_header,
        row.cf_ray,
        row.content_type
    )
}

#[tokio::test]
async fn upstream_failures_are_classified_and_stored() {
    let upstream = Server::start(vec![
        (
            "GET /works/111?view_full_work=true",
            Response::new(403)
                .with_header("server", "cloudflare")
                .with_header("cf-ray", "ray-403")
                .with_header("content-type", "text/html")
                .with_body("challenge leaks _otwarchive_session=secret-cookie"),
        ),
        (
            "GET /works/222?view_full_work=true",
            Response::new(429)
                .with_header("retry-after", "30")
                .with_header("cf-ray", "ray-429"),
        ),
        (
            "GET /works/333?view_full_work=true",
            Response::redirect("/users/login?return_to=%2Fworks%2F333&authenticity_token=tok"),
        ),
    ]);
    let dir = support::temp_dir("diag-upstream");
    let (root, store, handle) = serve_app(&dir, &upstream.url()).await;

    for id in ["111", "222", "333"] {
        let response = open_work(&root, id).await;
        assert_eq!(response.status(), 502, "id {id}");
    }

    let rows = wait_for(&store, "upstream", 3).await;
    let by_path = |path: &str| {
        rows.iter()
            .find(|row| row.path == path)
            .unwrap_or_else(|| panic!("missing {path}: {rows:?}"))
    };

    let blocked = by_path("/works/111?view_full_work=true");
    assert_eq!(blocked.error_kind, "blocked_by_edge");
    assert_eq!(blocked.status, Some(403));
    assert_eq!(blocked.server_header.as_deref(), Some("cloudflare"));
    assert_eq!(blocked.cf_ray.as_deref(), Some("ray-403"));
    assert_eq!(blocked.transport, "direct");
    assert!(blocked.latency_ms.is_some());
    assert!(!blocked.message.contains("secret-cookie"), "{blocked:?}");

    let limited = by_path("/works/222?view_full_work=true");
    assert_eq!(limited.error_kind, "rate_limited");
    assert_eq!(limited.retry_after, Some(30));
    assert_eq!(limited.cf_ray.as_deref(), Some("ray-429"));

    let redirected = by_path("/works/333?view_full_work=true");
    assert_eq!(redirected.error_kind, "redirect_login");
    assert_eq!(redirected.location_path.as_deref(), Some("/users/login"));
    assert_ne!(
        redirected.location_path.as_deref(),
        Some("/users/login?return_to=%2Fworks%2F333&authenticity_token=tok")
    );

    // The inbound 502 pages are recorded too, with their own direction.
    let inbound = wait_for(&store, "inbound", 3).await;
    assert!(inbound.iter().all(|row| row.status == Some(502)));
    assert!(
        inbound
            .iter()
            .all(|row| row.error_kind == "inbound_server_error")
    );

    // Nothing secret reached the table, in any field.
    for row in rows.iter().chain(inbound.iter()) {
        let text = all_text(row);
        for secret in ["secret-cookie", "authenticity_token", "_otwarchive_session"] {
            assert!(!text.contains(secret), "leaked {secret}: {text}");
        }
        let line = format_line(&ao3instance::diag::HttpErrorEvent::new(
            ao3instance::diag::HttpDirection::Upstream,
            ao3instance::diag::Transport::Direct,
            &row.method,
            &row.path,
            "blocked_by_edge",
            &row.message,
        ));
        assert!(!line.contains("secret-cookie"), "{line}");
    }

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn not_found_and_connection_refused_are_typed() {
    let upstream = Server::start(vec![(
        "GET /works/444?view_full_work=true",
        Response::new(404),
    )]);
    let dir = support::temp_dir("diag-notfound");
    let (root, store, handle) = serve_app(&dir, &upstream.url()).await;

    let response = open_work(&root, "444").await;
    assert_eq!(response.status(), 404);

    let rows = wait_for(&store, "upstream", 1).await;
    assert_eq!(rows[0].error_kind, "not_found");
    assert_eq!(rows[0].status, Some(404));

    // Same app, different upstream: a fresh config pointing at a refused port.
    let dir2 = support::temp_dir("diag-refused");
    let (root2, store2, handle2) = serve_app(&dir2, "http://127.0.0.1:1").await;
    let response = open_work(&root2, "555").await;
    assert_eq!(response.status(), 502);

    let rows = wait_for(&store2, "upstream", 1).await;
    assert_eq!(rows[0].error_kind, "network");
    assert_eq!(rows[0].status, None);
    assert_eq!(rows[0].method, "GET");
    assert!(rows[0].message.contains("connect"), "{:?}", rows[0].message);

    handle.abort();
    handle2.abort();
    drop(std::fs::remove_dir_all(dir));
    drop(std::fs::remove_dir_all(dir2));
}

#[tokio::test]
async fn inbound_4xx_is_logged_without_upstream_traffic() {
    let dir = support::temp_dir("diag-inbound");
    // Unreachable upstream: the 404 must come from the cache layer, not AO3.
    let (root, store, handle) = serve_app(&dir, "http://127.0.0.1:1").await;

    let favicon = http()
        .get(format!("{root}/favicon.ico"))
        .send()
        .await
        .expect("favicon");
    assert_eq!(favicon.status(), 204);

    let response = http()
        .get(format!("{root}/read/999999"))
        .send()
        .await
        .expect("read");
    assert_eq!(response.status(), 404);

    let rows = wait_for(&store, "inbound", 1).await;
    assert_eq!(rows[0].error_kind, "inbound_client_error");
    assert_eq!(rows[0].status, Some(404));
    assert_eq!(rows[0].path, "/read/999999");
    assert_eq!(rows[0].transport, "direct");
    assert!(
        rows.iter().all(|row| row.path != "/favicon.ico"),
        "favicon must not be logged: {rows:?}"
    );
    assert!(
        store
            .recent_http_errors(10, Some("upstream"))
            .await
            .unwrap()
            .is_empty(),
        "an offline 404 must not touch upstream"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn diagnostics_page_filters_directions_and_clears() {
    let upstream = Server::start(vec![(
        "GET /works/444?view_full_work=true",
        Response::new(404),
    )]);
    let dir = support::temp_dir("diag-page");
    let (root, store, handle) = serve_app(&dir, &upstream.url()).await;

    let _ = open_work(&root, "444").await;
    wait_for(&store, "upstream", 1).await;
    wait_for(&store, "inbound", 1).await;

    // A cached work path and a cached solver target must become reader links.
    support::harness::seed_work(&store).await;
    let cached = HttpErrorEvent::new(
        HttpDirection::Upstream,
        Transport::Direct,
        "GET",
        "/works/12345?view_full_work=true",
        "blocked_by_edge",
        "refused",
    )
    .with_status(403);
    store.record_http_error(&cached, 500).await.expect("row");
    store
        .record_solver_event(
            "solved",
            "/works/12345?view_full_work=true",
            Some(200),
            Some(10),
            200,
        )
        .await
        .expect("solver row");

    let client = http();
    let all = client
        .get(format!("{root}/diagnostics"))
        .send()
        .await
        .expect("diagnostics");
    let all = all.text().await.expect("body");
    assert!(all.contains("/works/444?view_full_work=true"), "{all}");
    assert!(
        !all.contains("href=\"/read/444\""),
        "uncached paths stay text: {all}"
    );
    assert!(
        all.matches("href=\"/read/12345\"").count() >= 2,
        "both the error row and the solver row link to the cached reader: {all}"
    );
    assert!(all.contains("Recent solver outcomes"), "{all}");

    let inbound = client
        .get(format!("{root}/diagnostics?direction=inbound"))
        .send()
        .await
        .expect("filtered");
    let inbound = inbound.text().await.expect("body");
    assert!(
        !inbound.contains("/works/444"),
        "upstream row hidden: {inbound}"
    );
    assert!(
        inbound.contains("inbound_client_error") || inbound.contains("inbound"),
        "{inbound}"
    );

    let cleared = client
        .post(format!("{root}/diagnostics/clear"))
        .send()
        .await
        .expect("clear");
    assert_eq!(cleared.status(), 200);
    let after = client
        .get(format!("{root}/diagnostics"))
        .send()
        .await
        .expect("after clear");
    assert!(
        after
            .text()
            .await
            .expect("body")
            .contains("Nothing recorded yet")
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
