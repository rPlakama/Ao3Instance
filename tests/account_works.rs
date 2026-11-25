#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! The author's works manager: AO3 works/drafts lists, draft publishing and work
//! deletion. Everything runs against a mock AO3; deletion is destructive, so it
//! is only ever one confirmed request.

mod support;

use ao3instance::ao3::work::parse_work_page;
use ao3instance::store::Store;
use support::harness::{http, session_routes, sign_in, start};
use support::{Response, Server};

fn works_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_my_works.html").expect("works fixture")
}

fn drafts_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_my_drafts.html").expect("drafts fixture")
}

fn work_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("work fixture")
}

/// Seeds the cached-work table with the fixture work under a chosen AO3 id.
async fn seed_cached_work(store: &Store, ao3_id: &str) {
    let mut page = parse_work_page(&work_fixture()).expect("fixture parses");
    page.ao3_id = ao3_id.to_string();
    store
        .upsert_work(&page, &format!("hash-{ao3_id}"))
        .await
        .expect("seed cached work");
}

fn post_confirm(
    client: &reqwest::Client,
    root: &str,
    path: &str,
    target: &str,
) -> reqwest::RequestBuilder {
    client
        .post(format!("{root}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("target_id={target}&confirm=yes"))
}

#[tokio::test]
async fn works_and_drafts_pages_render_the_ao3_lists() {
    let upstream = Server::start(session_routes(vec![
        ("GET /users/someone/works", Response::ok(works_fixture())),
        (
            "GET /users/someone/works/drafts",
            Response::ok(drafts_fixture()),
        ),
    ]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let works = client
        .get(format!("{root}/my/works"))
        .send()
        .await
        .expect("my works")
        .text()
        .await
        .expect("body");
    assert!(works.contains("My Works"), "{works}");
    assert!(works.contains("The Clockwork Ferry"), "{works}");
    assert!(works.contains("href=\"/open/7701\""), "{works}");
    assert!(works.contains("Delete This Work"), "{works}");
    assert!(
        works.contains("/write/preview?kind=work-delete&amp;id=7701")
            || works.contains("/write/preview?kind=work-delete&id=7701"),
        "{works}"
    );
    assert!(!works.contains(">Post Draft<"), "{works}");

    let drafts = client
        .get(format!("{root}/my/works/drafts"))
        .send()
        .await
        .expect("my drafts")
        .text()
        .await
        .expect("body");
    assert!(drafts.contains("My Drafts"), "{drafts}");
    assert!(drafts.contains("A Draft in the Cupboard"), "{drafts}");
    assert!(drafts.contains("Post Draft"), "{drafts}");
    assert!(
        drafts.contains("delete them") || drafts.contains("deleted by the archive"),
        "{drafts}"
    );
    assert!(drafts.contains("(Draft)"), "{drafts}");
    assert!(
        drafts.contains("/write/preview?kind=work-delete&amp;id=8801&amp;return=drafts")
            || drafts.contains("/write/preview?kind=work-delete&id=8801&return=drafts"),
        "draft deletes return to the drafts list: {drafts}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn publishing_a_draft_confirms_then_puts_once_and_opens_the_work() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /users/someone/works/drafts",
            Response::ok(drafts_fixture()),
        ),
        (
            "PUT /works/8801/post_draft",
            Response::redirect("/works/8801"),
        ),
        (
            "GET /works/8801?view_full_work=true",
            Response::ok(work_fixture()),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let confirm = client
        .get(format!("{root}/write/preview?kind=work-publish&id=8801"))
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(confirm.contains("Confirm draft publish"), "{confirm}");
    assert!(confirm.contains("become public"), "{confirm}");

    let published = post_confirm(&client, &root, "/write/work/publish", "8801")
        .send()
        .await
        .expect("publish")
        .text()
        .await
        .expect("body");
    assert!(
        published.contains("PLACEHOLDER: The Map and the Mirror"),
        "the published draft opens in the reader: {published}"
    );
    let puts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "PUT" && request.path == "/works/8801/post_draft")
        .collect();
    assert_eq!(puts.len(), 1, "exactly one publish request");
    let audit = store.recent_audit(5).await.expect("audit");
    let row = audit
        .iter()
        .find(|entry| entry.action == "publish_draft")
        .expect("audit row");
    assert_eq!(row.outcome, "work-published");
    assert_eq!(row.evidence.as_deref(), Some("work 8801"));
    assert!(store.get_work("8801").await.expect("cache").is_some());

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn ao3_refusing_to_publish_a_draft_is_reported_and_audited() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /users/someone/works/drafts",
            Response::ok(drafts_fixture()),
        ),
        (
            "PUT /works/8801/post_draft",
            Response::redirect("/users/someone/works/8801/edit"),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let body = post_confirm(&client, &root, "/write/work/publish", "8801")
        .send()
        .await
        .expect("publish")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains("could not publish that draft"),
        "AO3's refusal is surfaced: {body}"
    );
    assert_eq!(
        upstream
            .requests()
            .into_iter()
            .filter(|request| request.method == "PUT")
            .count(),
        1,
        "a refused publish is not retried"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "publish_draft" && entry.outcome == "failed"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn deleting_a_work_confirms_then_removes_it_upstream_and_locally() {
    let upstream = Server::start(session_routes(vec![
        ("GET /users/someone/works", Response::ok(works_fixture())),
        (
            "DELETE /works/7701",
            Response::redirect("/users/someone/works"),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_cached_work(&store, "7701").await;

    let confirm = client
        .get(format!("{root}/write/preview?kind=work-delete&id=7701"))
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(confirm.contains("Confirm work delete"), "{confirm}");
    assert!(confirm.contains("cannot be undone"), "{confirm}");
    assert!(confirm.contains("The Map and the Mirror"), "{confirm}");

    let body = post_confirm(&client, &root, "/write/work/delete", "7701")
        .send()
        .await
        .expect("delete")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Work deleted on AO3."), "{body}");
    let deletes: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "DELETE")
        .collect();
    assert_eq!(deletes.len(), 1, "exactly one delete request");
    assert_eq!(deletes[0].path, "/works/7701");
    assert!(
        store.get_work("7701").await.expect("cache").is_none(),
        "the deleted work is removed from the local cache"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "delete_work" && entry.outcome == "work-deleted"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn destructive_work_actions_need_confirmation_and_the_write_switch() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;

    // With the write switch off, the route is refused before any other check.
    let unconfirmed = client
        .post(format!("{root}/write/work/delete"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("target_id=7701")
        .send()
        .await
        .expect("unconfirmed delete");
    assert_eq!(unconfirmed.status(), 403);
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "DELETE"),
        "unconfirmed deletes never reach AO3"
    );

    // The confirmation preview is refused too: no misleading buttons.
    let refused = client
        .get(format!("{root}/write/preview?kind=work-delete&id=7701"))
        .send()
        .await
        .expect("preview");
    assert_eq!(refused.status(), 403);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
