#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Chapter management: AO3 chapter index, add chapter (post or draft) and edit
//! chapter. One confirmed request per save; the local cache refreshes afterwards.

mod support;

use support::harness::{http, session_routes, sign_in, start};
use support::{Response, Server};

fn navigate_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_navigate.html").expect("navigate fixture")
}

fn edit_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_chapter_edit.html").expect("edit fixture")
}

fn work_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_full.html").expect("work fixture")
}

fn body_encoded(fields: &[(&str, String)]) -> String {
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
        .body(body_encoded(fields))
}

fn chapter_fields(action: &str) -> Vec<(&'static str, String)> {
    vec![
        ("work_id", "12345".to_string()),
        ("title", "Chapter Three: The Long Way Home".to_string()),
        ("position", "3".to_string()),
        ("wip_length", "3".to_string()),
        ("summary", "A short chapter summary.".to_string()),
        ("notes", "Thanks for reading.".to_string()),
        ("endnotes", "Next time: the harbour.".to_string()),
        (
            "content",
            "The apprentice walked home the long way, counting lamps.".to_string(),
        ),
        ("action", action.to_string()),
    ]
}

#[tokio::test]
async fn chapter_manager_lists_chapters_with_edit_links() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /works/12345/navigate",
            Response::ok(navigate_fixture()),
        ),
        (
            "GET /users/someone/works",
            Response::ok(
                std::fs::read_to_string("tests/fixtures/ao3_my_works.html").expect("works fixture"),
            ),
        ),
    ]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let body = client
        .get(format!("{root}/my/works/12345/chapters"))
        .send()
        .await
        .expect("chapter manager")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Chapters:"), "{body}");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "{body}"
    );
    assert!(
        body.contains("Chapter 1: The Cartographer&#39;s Apprentice"),
        "{body}"
    );
    assert!(
        body.contains("/write/chapter/edit?work_id=12345&amp;chapter_id=78902"),
        "edit links carry the AO3 chapter id: {body}"
    );
    assert!(
        body.contains("/write/chapter/new?work_id=12345"),
        "add-chapter link: {body}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn posting_a_chapter_confirms_then_posts_once_and_refreshes_the_cache() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /works/12345/navigate",
            Response::ok(navigate_fixture()),
        ),
        (
            "POST /works/12345/chapters",
            Response::redirect("/works/12345/chapters/90999"),
        ),
        (
            "GET /works/12345?view_full_work=true",
            Response::ok(work_fixture()),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let editor = client
        .get(format!("{root}/write/chapter/new?work_id=12345"))
        .send()
        .await
        .expect("editor")
        .text()
        .await
        .expect("body");
    assert!(editor.contains("Post New Chapter"), "{editor}");
    assert!(
        editor.contains("PLACEHOLDER: The Map and the Mirror"),
        "{editor}"
    );
    assert!(
        editor.contains("name=\"position\" value=\"3\""),
        "next chapter number is suggested: {editor}"
    );

    let fields = chapter_fields("post");
    let preview = post_encoded(&root, "/write/chapter/preview", &fields)
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm Chapter Post"), "{preview}");
    assert!(
        preview.contains("Chapter Three: The Long Way Home"),
        "{preview}"
    );
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "POST"),
        "preview never writes"
    );

    let mut confirmed = chapter_fields("post");
    confirmed.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/chapter", &confirmed)
        .send()
        .await
        .expect("post chapter")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Chapter posted on AO3."), "{body}");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "reader opens after the cache refresh: {body}"
    );

    let posts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/works/12345/chapters")
        .collect();
    assert_eq!(posts.len(), 1, "exactly one chapter create");
    for needle in [
        "chapter%5Btitle%5D=Chapter+Three",
        "chapter%5Bcontent%5D=The+apprentice",
        "chapter%5Bwip_length%5D=3",
        "post_without_preview_button=Post",
        "authenticity_token=action-token",
    ] {
        assert!(
            posts[0].body.contains(needle),
            "missing {needle}: {}",
            posts[0].body
        );
    }
    let audit = store.recent_audit(5).await.expect("audit");
    let row = audit
        .iter()
        .find(|entry| entry.action == "post_chapter")
        .expect("audit row");
    assert_eq!(row.outcome, "chapter-posted");
    assert_eq!(row.evidence.as_deref(), Some("chapter 90999"));
    assert!(store.get_work("12345").await.expect("cache").is_some());
    assert_eq!(
        upstream
            .requests()
            .into_iter()
            .filter(|request| request.path == "/works/12345?view_full_work=true")
            .count(),
        1,
        "the cache is refreshed once from AO3"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn saving_a_chapter_draft_uses_the_draft_button_and_audits_it() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /works/12345/navigate",
            Response::ok(navigate_fixture()),
        ),
        (
            "POST /works/12345/chapters",
            Response::redirect("/works/12345/chapters/90998/preview"),
        ),
        (
            "GET /works/12345?view_full_work=true",
            Response::ok(work_fixture()),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let mut fields = chapter_fields("draft");
    fields.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/chapter", &fields)
        .send()
        .await
        .expect("draft chapter")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Chapter saved as a draft on AO3."), "{body}");
    let posts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/works/12345/chapters")
        .collect();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].body.contains("save_button=Save+as+Draft"),
        "{}",
        posts[0].body
    );
    assert!(!posts[0].body.contains("post_without_preview_button"));
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit.iter().any(|entry| entry.outcome == "chapter-draft"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn editing_a_chapter_prefills_and_puts_once() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /works/12345/chapters/78902/edit",
            Response::ok(edit_fixture()),
        ),
        (
            "PUT /works/12345/chapters/78902",
            Response::redirect("/works/12345/chapters/78902"),
        ),
        (
            "GET /works/12345?view_full_work=true",
            Response::ok(work_fixture()),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let editor = client
        .get(format!(
            "{root}/write/chapter/edit?work_id=12345&chapter_id=78902"
        ))
        .send()
        .await
        .expect("editor")
        .text()
        .await
        .expect("body");
    assert!(editor.contains("Edit Chapter"), "{editor}");
    assert!(
        editor.contains("value=\"A Map of Small Things\""),
        "current title prefilled: {editor}"
    );
    assert!(
        editor.contains("PLACEHOLDER PROSE (chapter two)"),
        "current chapter text prefilled: {editor}"
    );

    let mut fields = chapter_fields("update");
    fields.retain(|(key, _)| *key != "action");
    fields.push(("chapter_id", "78902".to_string()));
    fields.push(("action", "update".to_string()));
    fields.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/chapter", &fields)
        .send()
        .await
        .expect("update chapter")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Chapter updated on AO3."), "{body}");
    let puts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "PUT" && request.path == "/works/12345/chapters/78902")
        .collect();
    assert_eq!(puts.len(), 1, "exactly one chapter update");
    assert!(
        puts[0].body.contains("chapter%5Btitle%5D=Chapter+Three"),
        "{}",
        puts[0].body
    );
    assert!(
        puts[0].body.contains("update_button=Update"),
        "{}",
        puts[0].body
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit.iter().any(|entry| entry.outcome == "chapter-updated"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn chapter_validation_blocks_short_content_before_any_write() {
    let upstream = Server::start(session_routes(vec![(
        "GET /works/12345/navigate",
        Response::ok(navigate_fixture()),
    )]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let mut fields = chapter_fields("post");
    fields.retain(|(key, _)| *key != "content");
    fields.push(("content", "short".to_string()));
    let body = post_encoded(&root, "/write/chapter/preview", &fields)
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(body.contains("at least 10 characters"), "{body}");
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "POST"),
        "invalid chapters never reach AO3"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn ao3_rejecting_a_chapter_is_surfaced_and_audited_without_retry() {
    let upstream = Server::start(session_routes(vec![
        (
            "GET /works/12345/navigate",
            Response::ok(navigate_fixture()),
        ),
        (
            "POST /works/12345/chapters",
            Response::ok(
                r#"<div id="chapter-form"><form></form></div>
                   <div id="error" class="error"><h4>Sorry! We couldn't save this chapter because:</h4>
                   <ul><li>Chapter Text is too short.</li></ul></div>"#,
            ),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let mut fields = chapter_fields("post");
    fields.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/chapter", &fields)
        .send()
        .await
        .expect("chapter")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Chapter Text is too short."), "{body}");
    assert!(
        body.contains("The apprentice walked home the long way"),
        "the composed text survives the failure: {body}"
    );
    assert_eq!(
        upstream
            .requests()
            .into_iter()
            .filter(|request| request.method == "POST" && request.path == "/works/12345/chapters")
            .count(),
        1,
        "a rejected chapter is not retried"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "post_chapter" && entry.outcome == "failed"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn chapter_writes_require_the_write_switch() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;

    let response = client
        .get(format!("{root}/write/chapter/new?work_id=12345"))
        .send()
        .await
        .expect("editor");
    assert_eq!(response.status(), 403);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
