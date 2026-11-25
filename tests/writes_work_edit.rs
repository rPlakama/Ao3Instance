#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Editing an existing work's metadata/tags: prefilled from AO3's edit form, a
//! local confirmation step, exactly one PATCH, and a best-effort cache refresh.

mod support;

use support::harness::{http, session_routes, sign_in, start};
use support::{Response, Server};

fn edit_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_edit_work.html").expect("edit fixture")
}

fn work_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("work fixture")
}

fn new_work_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_new_work.html").expect("new-work fixture")
}

/// Seeds the cached-work table so the edit flow exercises the refresh path.
async fn seed_cached_work(store: &ao3instance::store::Store) {
    let mut page = ao3instance::ao3::work::parse_work_page(&work_fixture()).expect("fixture");
    page.ao3_id = "7701".to_string();
    store
        .upsert_work(&page, "hash-7701")
        .await
        .expect("seed cached work");
}

fn encoded(fields: &[(&str, String)]) -> String {
    fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                ao3instance::ao3::parse::percent_encode(
                    key,
                    ao3instance::ao3::parse::SpaceMode::Plus
                ),
                ao3instance::ao3::parse::percent_encode(
                    value,
                    ao3instance::ao3::parse::SpaceMode::Plus
                )
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn post_encoded(root: &str, path: &str, fields: &[(&str, String)]) -> reqwest::RequestBuilder {
    http()
        .post(format!("{root}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(encoded(fields))
}

fn edit_fields() -> Vec<(&'static str, String)> {
    vec![
        ("work_id", "7701".to_string()),
        ("title", "The Clockwork Ferry".to_string()),
        ("rating", "General Audiences".to_string()),
        ("warnings", "No Archive Warnings Apply".to_string()),
        ("fandom", "Test Fandom".to_string()),
        ("categories", "F/M".to_string()),
        ("relationships", "Character A/Character B".to_string()),
        ("characters", "Character A".to_string()),
        ("additional_tags", "Fluff, Domestic Fluff".to_string()),
        ("language_id", "1".to_string()),
        (
            "summary",
            "A ferry that runs on time, once a century.".to_string(),
        ),
        ("notes", "Edited work note.".to_string()),
        ("endnotes", "Edited end note.".to_string()),
        ("collections", String::new()),
        ("recipients", String::new()),
    ]
}

#[tokio::test]
async fn editor_prefills_from_the_ao3_edit_form() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("GET /works/7701/edit", Response::ok(edit_fixture())),
    ]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let body = client
        .get(format!("{root}/write/work/edit?work_id=7701"))
        .send()
        .await
        .expect("editor")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Edit Work"), "{body}");
    assert!(body.contains("value=\"The Clockwork Ferry\""), "{body}");
    assert!(
        body.contains("value=\"General Audiences\" selected"),
        "current rating selected: {body}"
    );
    assert!(
        body.contains("value=\"No Archive Warnings Apply\" checked"),
        "current warning checked: {body}"
    );
    assert!(
        body.contains("Edited work note.") || body.contains("PLACEHOLDER WORK NOTE"),
        "{body}"
    );
    assert!(body.contains("Preview &amp; Update"), "{body}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn editing_confirms_then_patches_once_and_refreshes_the_cache() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("GET /works/7701/edit", Response::ok(edit_fixture())),
        ("PATCH /works/7701", Response::redirect("/works/7701")),
        (
            "GET /works/7701?view_full_work=true",
            Response::ok(work_fixture()),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_cached_work(&store).await;

    let preview = post_encoded(&root, "/write/work/edit/preview", &edit_fields())
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm Work Update"), "{preview}");
    assert!(preview.contains("Domestic Fluff"), "{preview}");
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "PATCH"),
        "preview never writes"
    );

    let mut confirmed = edit_fields();
    confirmed.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/work/edit", &confirmed)
        .send()
        .await
        .expect("edit")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Work updated on AO3."), "{body}");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "reader opens after the cache refresh: {body}"
    );
    let patches: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "PATCH" && request.path == "/works/7701")
        .collect();
    assert_eq!(patches.len(), 1, "exactly one update request");
    for needle in [
        "work%5Btitle%5D=The+Clockwork+Ferry",
        "work%5Brating_string%5D=General+Audiences",
        "work%5Barchive_warning_strings%5D%5B%5D=No+Archive+Warnings+Apply",
        "work%5Bcategory_strings%5D%5B%5D=F%2FM",
        "work%5Bsummary%5D=A+ferry",
        "authenticity_token=",
    ] {
        assert!(
            patches[0].body.contains(needle),
            "missing {needle}: {}",
            patches[0].body
        );
    }
    assert!(
        !patches[0].body.contains("work%5Bchapter_attributes%5D"),
        "metadata editing never touches chapter text"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    let row = audit
        .iter()
        .find(|entry| entry.action == "update_work")
        .expect("audit row");
    assert_eq!(row.outcome, "work-updated");
    assert!(store.get_work("7701").await.expect("cache").is_some());

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn ao3_rejecting_the_edit_is_surfaced_and_audited_without_retry() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("GET /works/7701/edit", Response::ok(edit_fixture())),
        (
            "PATCH /works/7701",
            Response::ok(
                r#"<form id="work-form"></form>
                   <div id="error" class="error"><h4>Sorry! We couldn't save this work because:</h4>
                   <ul><li>You have already used this tag on too many works.</li></ul></div>"#,
            ),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let mut confirmed = edit_fields();
    confirmed.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/work/edit", &confirmed)
        .send()
        .await
        .expect("edit")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains("You have already used this tag on too many works."),
        "AO3's message is shown: {body}"
    );
    assert!(
        body.contains("value=\"The Clockwork Ferry\""),
        "fields preserved: {body}"
    );
    assert_eq!(
        upstream
            .requests()
            .into_iter()
            .filter(|request| request.method == "PATCH")
            .count(),
        1,
        "a rejected edit is not retried"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "update_work" && entry.outcome == "failed"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn local_validation_blocks_an_empty_title_without_network() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("GET /works/7701/edit", Response::ok(edit_fixture())),
    ]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let mut fields = edit_fields();
    fields.retain(|(key, _)| *key != "title" && *key != "warnings");
    fields.push(("title", String::new()));
    let body = post_encoded(&root, "/write/work/edit/preview", &fields)
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Work Title is required"), "{body}");
    assert!(body.contains("Archive Warning is required"), "{body}");
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "PATCH"),
        "invalid edits never reach AO3"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn work_editing_requires_the_write_switch() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;

    let response = client
        .get(format!("{root}/write/work/edit?work_id=7701"))
        .send()
        .await
        .expect("editor");
    assert_eq!(response.status(), 403);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
