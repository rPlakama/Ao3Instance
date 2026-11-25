#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! W5: kudos (JS format is machine-readable) and AO3 reading flags.

mod support;

use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

fn post_form(path: &str, body: &'static str) -> reqwest::RequestBuilder {
    http()
        .post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
}

#[tokio::test]
async fn kudos_and_reading_flags_are_confirmed_audited_and_remembered() {
    let upstream = Server::start(session_routes(vec![
        (
            "POST /kudos.js",
            Response::new(201)
                .with_body("")
                .with_header("content-type", "text/javascript"),
        ),
        (
            "PATCH /works/12345/mark_for_later",
            Response::redirect("/works/12345?view_full_work=true"),
        ),
        (
            "PATCH /works/12345/mark_as_read",
            Response::redirect("/works/12345?view_full_work=true"),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    // Reader offers both actions for a signed-in session.
    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(
        reader.contains("action=\"/write/kudos\"")
            && reader.contains("name=\"confirm\" value=\"yes\"")
            && reader.contains("Kudos \u{2665}</button>"),
        "reader exposes the direct kudos form: {reader}"
    );
    assert!(
        reader.contains("1,024 people left kudos on this work!"),
        "cached kudos count module: {reader}"
    );
    assert!(reader.contains("Mark for later"), "{reader}");
    assert!(reader.contains("Mark as read"), "{reader}");

    // Kudos goes through the confirmation page and the JS endpoint.
    let preview = client
        .get(format!("{root}/write/preview?kind=kudos&id=12345"))
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm kudos"), "{preview}");

    let given = post_form(
        &format!("{root}/write/kudos"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("kudos");
    assert_eq!(given.status(), 200);
    let body = given.text().await.expect("body");
    assert!(body.contains("Kudos given."), "{body}");
    assert!(
        !body.contains("action=\"/write/kudos\""),
        "kudos form replaced by the receipt state: {body}"
    );
    assert!(body.contains("kudos given"), "{body}");
    assert!(
        body.contains("You and 1,023 others left kudos on this work!"),
        "receipt module for the current account: {body}"
    );

    let kudos_request = upstream
        .requests()
        .into_iter()
        .find(|request| request.path == "/kudos.js")
        .expect("kudos request");
    assert_eq!(kudos_request.method, "POST");
    assert!(
        kudos_request
            .body
            .contains("kudo%5Bcommentable_id%5D=12345")
    );
    assert!(
        kudos_request
            .body
            .contains("kudo%5Bcommentable_type%5D=Work")
    );
    assert!(
        kudos_request
            .body
            .contains("authenticity_token=action-token")
    );
    assert!(store.has_kudos("someone", "12345").await.expect("kudos"));

    // A second kudos is refused from the local receipt: no network.
    let before = upstream.requests().len();
    let duplicate = post_form(
        &format!("{root}/write/kudos"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("duplicate");
    let duplicate_body = duplicate.text().await.expect("body");
    assert!(
        duplicate_body.contains("already left kudos"),
        "{duplicate_body}"
    );
    assert_eq!(upstream.requests().len(), before);

    // Reading flags are PATCHes with the CSRF token.
    let later = post_form(
        &format!("{root}/write/reading"),
        "target_id=12345&action=later",
    )
    .send()
    .await
    .expect("mark later");
    let body = later.text().await.expect("body");
    assert!(body.contains("Marked for later."), "{body}");
    assert_eq!(
        store
            .get_reading_state("someone", "12345")
            .await
            .expect("state")
            .as_deref(),
        Some("for_later")
    );

    let read = post_form(
        &format!("{root}/write/reading"),
        "target_id=12345&action=read",
    )
    .send()
    .await
    .expect("mark read");
    let body = read.text().await.expect("body");
    assert!(body.contains("Marked as read."), "{body}");
    assert_eq!(
        store
            .get_reading_state("someone", "12345")
            .await
            .expect("state")
            .as_deref(),
        Some("read")
    );

    let patches: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "PATCH")
        .collect();
    assert_eq!(patches.len(), 2);
    assert!(
        patches
            .iter()
            .all(|request| request.body.contains("authenticity_token=action-token"))
    );
    assert!(
        patches
            .iter()
            .any(|request| request.path.ends_with("/mark_for_later"))
    );
    assert!(
        patches
            .iter()
            .any(|request| request.path.ends_with("/mark_as_read"))
    );

    // Audit trail covers all three actions.
    let audit = client
        .get(format!("{root}/audit"))
        .send()
        .await
        .expect("audit")
        .text()
        .await
        .expect("body");
    assert!(audit.contains("kudos"), "{audit}");
    assert!(audit.contains("reading_for_later"), "{audit}");
    assert!(audit.contains("reading_read"), "{audit}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn already_given_kudos_from_ao3_is_reported_as_such() {
    let upstream = Server::start(session_routes(vec![(
        "POST /kudos.js",
        Response::json(r#"{"error_message":"You have already left kudos here. :)"}"#)
            .with_status(422),
    )]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let response = post_form(
        &format!("{root}/write/kudos"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("kudos");
    let body = response.text().await.expect("body");
    assert!(body.contains("already left kudos"), "{body}");
    assert!(
        store.has_kudos("someone", "12345").await.expect("kudos"),
        "a duplicate is still a receipt"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(audit.iter().any(|entry| entry.outcome == "already-done"));

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn refused_kudos_is_a_failed_audit_and_keeps_the_button() {
    let upstream = Server::start(session_routes(vec![(
        "POST /kudos.js",
        Response::json(r#"{"error_message":"You can't leave kudos on your own work."}"#)
            .with_status(422),
    )]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let response = post_form(
        &format!("{root}/write/kudos"),
        "target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("kudos");
    let body = response.text().await.expect("body");
    assert!(body.contains("AO3 refused the kudos"), "{body}");
    assert!(!store.has_kudos("someone", "12345").await.expect("kudos"));
    let audit = store.recent_audit(5).await.expect("audit");
    assert_eq!(audit[0].outcome, "failed");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn reading_flag_csrf_failure_is_a_failed_audit() {
    let upstream = Server::start(session_routes(vec![(
        "PATCH /works/12345/mark_for_later",
        Response::redirect("/auth_error"),
    )]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let response = post_form(
        &format!("{root}/write/reading"),
        "target_id=12345&action=later",
    )
    .send()
    .await
    .expect("reading");
    let body = response.text().await.expect("body");
    assert!(body.contains("AO3 refused the reading update"), "{body}");
    assert!(
        store
            .get_reading_state("someone", "12345")
            .await
            .expect("state")
            .is_none()
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn reading_rate_limit_engages_governor_cooldown() {
    let upstream = Server::start(session_routes(vec![(
        "PATCH /works/12345/mark_for_later",
        Response::new(429)
            .with_header("retry-after", "60")
            .with_body("Retry later"),
    )]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let first = post_form(
        &format!("{root}/write/reading"),
        "target_id=12345&action=later",
    )
    .send()
    .await
    .expect("first reading");
    assert_eq!(first.status(), 200);

    let before = upstream.requests().len();
    let second = post_form(
        &format!("{root}/write/reading"),
        "target_id=12345&action=read",
    )
    .send()
    .await
    .expect("second reading");
    assert_eq!(
        second.status(),
        502,
        "governor cooldown should block immediate retry"
    );
    let body = second.text().await.expect("body");
    assert!(body.contains("rate limiting"), "{body}");
    assert_eq!(
        upstream.requests().len(),
        before,
        "governor cooldown must prevent hitting AO3 again"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
