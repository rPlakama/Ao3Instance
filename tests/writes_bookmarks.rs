#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Bookmarks: confirmation, pseud discovery, one request, ledger, audit.

mod support;

use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

fn pseuds_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/pseuds.html").expect("pseuds fixture")
}

fn bookmark_routes(bookmark_responses: Vec<Response>) -> Vec<(String, Vec<Response>)> {
    let mut routes: Vec<(String, Vec<Response>)> = session_routes(vec![])
        .into_iter()
        .map(|(key, response)| (key, vec![response]))
        .collect();
    routes.push((
        "GET /users/someone/pseuds".to_string(),
        vec![
            Response::ok(pseuds_fixture()).with_header("content-type", "text/html; charset=utf-8"),
        ],
    ));
    routes.push((
        "POST /works/12345/bookmarks".to_string(),
        bookmark_responses,
    ));
    routes
}

fn post_form(path: &str, body: &'static str) -> reqwest::RequestBuilder {
    http()
        .post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
}

#[tokio::test]
async fn bookmark_is_confirmed_sent_once_and_ledgered() {
    let upstream = Server::start_sequence(bookmark_routes(vec![
        Response::new(302).with_header("location", "/bookmarks/900"),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(reader.contains("Bookmark"), "{reader}");

    let preview = client
        .get(format!("{root}/write/preview?kind=bookmark&id=12345"))
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm bookmark"), "{preview}");

    let saved = post_form(
        &format!("{root}/write/bookmark"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("bookmark");
    assert_eq!(saved.status(), 200);
    let body = saved.text().await.expect("body");
    assert!(body.contains("Bookmarked on AO3."), "{body}");
    assert!(body.contains("bookmarked"), "state chip expected: {body}");

    let posts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.path == "/works/12345/bookmarks")
        .collect();
    assert_eq!(posts.len(), 1);
    assert!(posts[0].body.contains("bookmark%5Bpseud_id%5D=111"));
    assert!(posts[0].body.contains("bookmark%5Bprivate%5D=0"));
    assert!(posts[0].body.contains("authenticity_token=action-token"));
    assert!(
        store
            .has_bookmark("someone", "12345")
            .await
            .expect("ledger")
    );

    let before = upstream.requests().len();
    let duplicate = post_form(
        &format!("{root}/write/bookmark"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("duplicate");
    let duplicate_body = duplicate.text().await.expect("body");
    assert!(
        duplicate_body.contains("Already bookmarked"),
        "{duplicate_body}"
    );
    assert_eq!(upstream.requests().len(), before, "no second request");

    let audit = store.recent_audit(5).await.expect("audit");
    assert_eq!(audit[0].action, "bookmark");
    assert_eq!(audit[0].outcome, "bookmarked");
    assert!(audit[0].evidence_or_empty().contains("900"));

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn refused_bookmark_is_a_failed_audit() {
    let refusal =
        r#"<html><body><div class="error">You can't bookmark your own work.</div></body></html>"#;
    let upstream = Server::start_sequence(bookmark_routes(vec![
        Response::ok(refusal).with_header("content-type", "text/html; charset=utf-8"),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let response = post_form(
        &format!("{root}/write/bookmark"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("bookmark");
    let body = response.text().await.expect("body");
    assert!(body.contains("AO3 rejected the bookmark"), "{body}");
    assert!(
        !store
            .has_bookmark("someone", "12345")
            .await
            .expect("ledger")
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert_eq!(audit[0].outcome, "failed");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
