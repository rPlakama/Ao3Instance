#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! W4 write path: confirmation, governor, intent/audit, subscription ledger.
//! Everything runs against a mock AO3; no test touches the live archive.

mod support;

use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

const SUCCESS_JSON: &str = r#"{"item_id":42,"item_success_message":"You are now following it."}"#;
const UNSUB_JSON: &str = r#"{"item_success_message":"You have successfully unsubscribed."}"#;

fn post_form(path: &str, body: &'static str) -> reqwest::RequestBuilder {
    http()
        .post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
}

#[tokio::test]
async fn subscribe_is_confirmed_audited_and_ledgered_then_unsubscribed() {
    let upstream = Server::start(session_routes(vec![
        (
            "POST /users/someone/subscriptions.json",
            Response::json(SUCCESS_JSON)
                .with_header("content-type", "application/json")
                .with_status(201),
        ),
        (
            "DELETE /users/someone/subscriptions/42.json",
            Response::json(UNSUB_JSON).with_header("content-type", "application/json"),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    // The reader offers a subscribe link when signed in.
    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(reader.contains("Subscribe to work"), "{reader}");

    // Preview is a deliberate confirmation step.
    let preview = client
        .get(format!("{root}/write/preview?kind=work&id=12345"))
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm subscription"), "{preview}");

    // Execute.
    let subscribed = post_form(
        &format!("{root}/write/subscribe"),
        "kind=Work&target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("subscribe");
    assert_eq!(subscribed.status(), 200);
    let body = subscribed.text().await.expect("body");
    assert!(body.contains("Subscribed on AO3."), "{body}");

    let writes: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path.contains("/subscriptions"))
        .collect();
    assert_eq!(writes.len(), 1, "exactly one write request");
    assert!(
        writes[0]
            .body
            .contains("subscription%5Bsubscribable_id%5D=12345")
    );
    assert!(
        writes[0]
            .body
            .contains("subscription%5Bsubscribable_type%5D=Work")
    );
    assert!(writes[0].body.contains("authenticity_token=action-token"));

    let ledger = store
        .get_subscription("someone", "Work", "12345")
        .await
        .expect("ledger")
        .expect("row");
    assert_eq!(ledger.state, "active");
    assert_eq!(ledger.subscription_id, Some(42));

    let audit = store.recent_audit(10).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.outcome == "subscribed" && entry.target == "Work 12345"),
        "{audit:?}"
    );

    // Duplicate: local ledger prevents a second upstream request.
    let before = upstream.requests().len();
    let duplicate = post_form(
        &format!("{root}/write/subscribe"),
        "kind=Work&target_id=12345&confirm=yes",
    )
    .send()
    .await
    .expect("duplicate");
    let duplicate_body = duplicate.text().await.expect("body");
    assert!(
        duplicate_body.contains("Already subscribed"),
        "{duplicate_body}"
    );
    assert_eq!(upstream.requests().len(), before, "no second request");

    // Unsubscribe uses the stored id.
    let unsubscribed = post_form(
        &format!("{root}/write/unsubscribe"),
        "kind=Work&target_id=12345",
    )
    .send()
    .await
    .expect("unsubscribe");
    assert_eq!(unsubscribed.status(), 200);
    let body = unsubscribed.text().await.expect("body");
    assert!(body.contains("Unsubscribed on AO3."), "{body}");

    let deletes: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "DELETE")
        .collect();
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].path, "/users/someone/subscriptions/42.json");
    let ledger = store
        .get_subscription("someone", "Work", "12345")
        .await
        .expect("ledger")
        .expect("row");
    assert_eq!(ledger.state, "removed");

    // The audit page shows the trail.
    let audit_page = client
        .get(format!("{root}/audit"))
        .send()
        .await
        .expect("audit page")
        .text()
        .await
        .expect("body");
    assert!(audit_page.contains("subscribe"), "{audit_page}");
    assert!(audit_page.contains("unsubscribe"), "{audit_page}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn validation_failure_is_audited_and_not_retried() {
    let upstream = Server::start(session_routes(vec![(
        "POST /users/someone/subscriptions.json",
        Response::json(r#"{"errors":["You can't subscribe to that."]}"#)
            .with_header("content-type", "application/json")
            .with_status(422),
    )]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let response = post_form(
        &format!("{root}/write/subscribe"),
        "kind=Work&target_id=777&confirm=yes",
    )
    .send()
    .await
    .expect("subscribe");
    assert_eq!(response.status(), 200, "redirected back to the start page");
    let body = response.text().await.expect("body");
    assert!(body.contains("AO3 rejected the action"), "{body}");

    let attempts = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path.contains("/subscriptions.json"))
        .count();
    assert_eq!(attempts, 1, "one attempt, no retry");
    let audit = store.recent_audit(10).await.expect("audit");
    assert_eq!(audit[0].outcome, "failed");
    assert!(audit[0].target.contains("777"));

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn writes_require_the_switch_and_an_explicit_confirmation() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, _store, dir, handle) = start(&upstream, false).await;

    // Kill switch: no network, clear error.
    let disabled = post_form(
        &format!("{root}/write/subscribe"),
        "kind=Work&target_id=123&confirm=yes",
    )
    .send()
    .await
    .expect("disabled");
    assert_eq!(disabled.status(), 403);
    assert!(
        upstream
            .requests()
            .iter()
            .all(|request| !request.path.contains("/subscriptions")),
        "writes disabled must not reach AO3"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
