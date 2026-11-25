#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Posting a new work (the "post a fanfic" capability): AO3's new-work options,
//! local validation, confirmation, exactly one `POST /works`, audit ledger, and
//! draft vs publish classification. Everything runs against a mock AO3.

mod support;

use ao3instance::ao3::parse::{SpaceMode, percent_encode};
use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

fn new_work_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_new_work.html").expect("new-work fixture")
}

fn work_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("work fixture")
}

const AO3_ERROR_PAGE: &str = r#"<html><body>
<form id="work-form" class="verbose post work" action="/works"></form>
<div id="error" class="error">
  <h4>Sorry! We couldn&#39;t save this work because:</h4>
  <ul><li>Please select at least one warning.</li><li>Content must be at least 10 characters long.</li></ul>
</div></body></html>"#;

fn compose_fields(action: &str) -> Vec<(&'static str, String)> {
    vec![
        ("title", "A Test Fic".to_string()),
        ("pseud_id", "111".to_string()),
        ("rating", "General Audiences".to_string()),
        ("warnings", "No Archive Warnings Apply".to_string()),
        ("fandom", "Test Fandom".to_string()),
        ("categories", "F/M".to_string()),
        ("relationships", "Character A/Character B".to_string()),
        ("characters", "Character A".to_string()),
        ("additional_tags", "Fluff, Domestic Fluff".to_string()),
        ("language_id", "1".to_string()),
        ("summary", "A short summary.".to_string()),
        ("notes", "Written for a test.".to_string()),
        ("chapter_title", "Chapter One".to_string()),
        (
            "content",
            "The lighthouse keeper counted every ship by lamplight.".to_string(),
        ),
        ("action", action.to_string()),
    ]
}

fn post_encoded(root: &str, path: &str, fields: &[(&str, String)]) -> reqwest::RequestBuilder {
    let body = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encode(key, SpaceMode::Plus),
                percent_encode(value, SpaceMode::Plus)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    http()
        .post(format!("{root}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
}

#[tokio::test]
async fn composer_renders_ao3_options_and_header_post_link() {
    let upstream = Server::start(session_routes(vec![(
        "GET /works/new",
        Response::ok(new_work_fixture()),
    )]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let body = client
        .get(format!("{root}/works/new"))
        .send()
        .await
        .expect("composer")
        .text()
        .await
        .expect("body");
    assert!(body.contains("id=\"work-form\""), "{body}");
    assert!(body.contains("New Work"), "{body}");
    // Header offers AO3's Post shortcut while writes are enabled.
    assert!(body.contains("href=\"/works/new\">Post</a>"), "{body}");
    // Language ids come from AO3's own form (not hardcoded).
    assert!(
        body.contains("<option value=\"1\" selected>English</option>"),
        "{body}"
    );
    // Rating, warning and byline options are rendered from the parsed form.
    assert!(body.contains("value=\"General Audiences\""), "{body}");
    assert!(
        body.contains("name=\"warnings\" value=\"No Archive Warnings Apply\""),
        "{body}"
    );
    assert!(body.contains(">Main Pseud</option>"), "{body}");
    assert!(body.contains("name=\"action\" value=\"publish\""), "{body}");
    assert_eq!(
        upstream
            .requests()
            .iter()
            .filter(|request| request.path == "/works/new")
            .count(),
        1,
        "options are fetched once"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn publish_flow_confirms_then_posts_exactly_once_and_audits() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("POST /works", Response::redirect("/works/7777")),
        (
            "GET /works/7777?view_full_work=true",
            Response::ok(work_fixture()),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let composer = client
        .get(format!("{root}/works/new"))
        .send()
        .await
        .expect("composer");
    assert_eq!(composer.status(), 200);

    // Local preview: no upstream write yet.
    let preview_fields = compose_fields("publish");
    let preview = post_encoded(&root, "/write/work/preview", &preview_fields)
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm Post Work"), "{preview}");
    assert!(preview.contains("A Test Fic"), "{preview}");
    assert!(preview.contains("No Archive Warnings Apply"), "{preview}");
    assert!(
        preview.contains("The lighthouse keeper counted every ship"),
        "work text is previewed: {preview}"
    );
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "POST"),
        "preview must not write to AO3"
    );

    // One confirmed create: the app caches and opens the new work.
    let mut confirmed = compose_fields("publish");
    confirmed.push(("confirm", "yes".to_string()));
    let posted = post_encoded(&root, "/write/work", &confirmed)
        .send()
        .await
        .expect("post work");
    assert_eq!(posted.status(), 200);
    let body = posted.text().await.expect("body");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "reader opens the posted work after caching it: {body}"
    );

    let posts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/works")
        .collect();
    assert_eq!(posts.len(), 1, "exactly one create request");
    let sent = &posts[0].body;
    for needle in [
        "work%5Btitle%5D=A+Test+Fic",
        "work%5Brating_string%5D=General+Audiences",
        "work%5Barchive_warning_strings%5D%5B%5D=No+Archive+Warnings+Apply",
        "work%5Bfandom_string%5D=Test+Fandom",
        "work%5Blanguage_id%5D=1",
        "work%5Bcategory_strings%5D%5B%5D=F%2FM",
        "work%5Bauthor_attributes%5D%5Bids%5D%5B%5D=111",
        "work%5Bchapter_attributes%5D%5Bcontent%5D=The+lighthouse",
        "post_button=Post",
        "authenticity_token=form-token",
    ] {
        assert!(sent.contains(needle), "missing {needle} in {sent}");
    }

    let audit = store.recent_audit(5).await.expect("audit");
    let row = audit
        .iter()
        .find(|entry| entry.action == "post_work")
        .expect("post_work audit row");
    assert_eq!(row.outcome, "work-posted");
    assert_eq!(row.target, "Work new");
    assert_eq!(row.evidence.as_deref(), Some("work 7777"));
    assert!(
        store.get_work("7777").await.expect("cache").is_some(),
        "the posted work is cached locally"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn draft_flow_saves_an_unposted_work_and_reports_it() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("POST /works", Response::redirect("/works/7778/preview")),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let _ = client
        .get(format!("{root}/works/new"))
        .send()
        .await
        .expect("composer");

    let mut confirmed = compose_fields("draft");
    confirmed.push(("confirm", "yes".to_string()));
    let draft = post_encoded(&root, "/write/work", &confirmed)
        .send()
        .await
        .expect("draft");
    assert_eq!(draft.status(), 200);
    let body = draft.text().await.expect("body");
    assert!(body.contains("Draft saved on AO3 as work 7778"), "{body}");
    let sends: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/works")
        .collect();
    assert_eq!(sends.len(), 1);
    assert!(
        sends[0].body.contains("save_button=Save+as+Draft"),
        "{}",
        sends[0].body
    );
    assert!(!sends[0].body.contains("post_button"), "{}", sends[0].body);
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit.iter().any(|entry| entry.outcome == "work-draft"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn local_validation_blocks_obviously_invalid_work_before_any_write() {
    let upstream = Server::start(session_routes(vec![(
        "GET /works/new",
        Response::ok(new_work_fixture()),
    )]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let incomplete = vec![
        ("title", String::new()),
        ("pseud_id", "111".to_string()),
        ("rating", String::new()),
        ("language_id", String::new()),
        ("content", "short".to_string()),
        ("action", "publish".to_string()),
    ];
    let body = post_encoded(&root, "/write/work/preview", &incomplete)
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    for needle in [
        "Work Title is required",
        "Fandoms is required",
        "Rating is required",
        "Archive Warning is required",
        "Language is required",
    ] {
        assert!(body.contains(needle), "missing {needle}: {body}");
    }
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "POST"),
        "validation never reaches AO3"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn ao3_validation_errors_are_surfaced_and_audited() {
    let upstream = Server::start(session_routes(vec![
        ("GET /works/new", Response::ok(new_work_fixture())),
        ("POST /works", Response::ok(AO3_ERROR_PAGE)),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    let _ = client
        .get(format!("{root}/works/new"))
        .send()
        .await
        .expect("composer");

    let mut confirmed = compose_fields("publish");
    confirmed.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/work", &confirmed)
        .send()
        .await
        .expect("post")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains("Please select at least one warning."),
        "AO3's own error text is shown: {body}"
    );
    assert!(
        body.contains("Sorry! We couldn't save this work because:"),
        "{body}"
    );
    // The composed text survives the failure so nothing is lost.
    assert!(
        body.contains("The lighthouse keeper counted every ship"),
        "{body}"
    );

    let sends = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/works")
        .count();
    assert_eq!(sends, 1, "a failed create is not retried");
    let audit = store.recent_audit(5).await.expect("audit");
    let row = audit
        .iter()
        .find(|entry| entry.action == "post_work")
        .expect("row");
    assert_eq!(row.outcome, "failed");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn composer_requires_the_write_switch() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let body = client
        .get(format!("{root}/works/new"))
        .send()
        .await
        .expect("composer")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Writes are disabled"), "{body}");
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.path == "/works/new"),
        "no upstream form fetch when writes are off"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
