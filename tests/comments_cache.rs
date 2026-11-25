#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Cached AO3 comment threads: one explicit POST fetches the right commentable,
//! stores it, and every later read renders from SQLite. A failed refresh never
//! destroys the previous copy.

mod support;

use ao3instance::ao3::work::parse_work_page;
use ao3instance::store::chapter_scope;
use support::harness::{http, seed_work, start};
use support::{Response, Server};

fn comments_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_comments.html").expect("comments fixture")
}

fn post_comments(root: &str, path: &str, body: &str) -> reqwest::RequestBuilder {
    http()
        .post(format!("{root}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.to_string())
}

#[tokio::test]
async fn chapter_comments_are_loaded_once_cached_and_rendered_offline() {
    let upstream = Server::start(vec![(
        "GET /works/12345/chapters/78901?show_comments=true",
        Response::ok(comments_fixture()),
    )]);
    let (root, store, dir, handle) = start(&upstream, false).await;
    let client = http();
    seed_work(&store).await;

    // Before any fetch: the reader offers a one-click load form, not a thread.
    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(
        reader.contains("action=\"/read/12345/comments\""),
        "load form on the cached work page: {reader}"
    );
    assert!(
        reader.contains("Comments for this work are not cached yet."),
        "{reader}"
    );
    assert!(!reader.contains("id=\"comment_9001\""), "{reader}");

    // One explicit POST fetches the chapter commentable AO3 shows for chapter 1.
    let loaded = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("load comments");
    assert_eq!(
        loaded.status(),
        200,
        "follows the redirect back to the reader"
    );
    let body = loaded.text().await.expect("body");

    assert!(body.contains("Comments loaded from AO3"), "{body}");
    assert!(body.contains("id=\"comment_9001\""), "{body}");
    assert!(body.contains("id=\"comment_9002\""), "nested reply: {body}");
    assert!(
        body.contains("id=\"comment_9004\""),
        "reply to a deleted comment is hoisted, not lost: {body}"
    );
    assert!(body.contains("id=\"comment_9005\""), "{body}");
    assert!(
        !body.contains("id=\"comment_9003\""),
        "deleted comment: {body}"
    );
    assert!(
        body.contains("First comment with <em>markup</em>."),
        "creator markup survives: {body}"
    );
    assert!(
        !body.contains("bad()") && !body.contains("<script>"),
        "author HTML is sanitized: {body}"
    );
    assert!(
        body.contains("href=\"/works?author=reader_one\""),
        "commenter becomes a local author filter: {body}"
    );
    assert!(body.contains("guest_name"), "guest byline: {body}");
    assert!(
        body.contains("Last Edited 03 Nov 2026 08:00 UTC"),
        "edited marker: {body}"
    );
    assert!(body.contains("4 cached comments"), "{body}");
    assert!(
        body.contains("Hide Comments"),
        "action flips to hide: {body}"
    );
    assert!(
        body.contains(
            "https://archiveofourown.org/works/12345/chapters/78901?show_comments=true&page=2"
        ),
        "paginated thread links to AO3 for the rest: {body}"
    );

    let fetches: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET" && request.path.contains("show_comments"))
        .collect();
    assert_eq!(fetches.len(), 1, "exactly one upstream fetch: {fetches:?}");
    assert_eq!(
        fetches[0].path,
        "/works/12345/chapters/78901?show_comments=true"
    );

    // The cache is real: rows are scoped to the chapter, with AO3's total.
    let scope = chapter_scope("78901");
    let stored = store.get_comments("12345", &scope).await.expect("cache");
    assert_eq!(stored.len(), 4);
    let set = store
        .get_comment_set("12345", &scope)
        .await
        .expect("meta")
        .expect("fetched");
    assert_eq!(set.total, Some(4));
    assert!(set.has_more);

    // Showing the cached thread never touches upstream again.
    let before = upstream.requests().len();
    let shown = client
        .get(format!("{root}/read/12345?chapter=1&show_comments=1"))
        .send()
        .await
        .expect("cached thread")
        .text()
        .await
        .expect("body");
    assert!(shown.contains("id=\"comment_9001\""), "{shown}");
    assert_eq!(
        upstream.requests().len(),
        before,
        "GET reads the cache, never upstream"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn entire_work_and_one_shot_readers_use_the_work_commentable() {
    let mut single = parse_work_page(
        &std::fs::read_to_string("tests/fixtures/work_single.html").expect("single fixture"),
    )
    .expect("single fixture parses");
    single.ao3_id = "654321".into();
    let upstream = Server::start(vec![
        (
            "GET /works/12345?view_full_work=true&show_comments=true",
            Response::ok(comments_fixture()),
        ),
        (
            "GET /works/654321?view_full_work=true&show_comments=true",
            Response::ok(comments_fixture()),
        ),
    ]);
    let (root, store, dir, handle) = start(&upstream, false).await;
    seed_work(&store).await;
    store
        .upsert_work(&single, "hash-single")
        .await
        .expect("seed");

    // Entire Work mode comments on the work itself.
    let loaded = post_comments(&root, "/read/12345/comments", "chapter=1&full_work=true")
        .send()
        .await
        .expect("load work comments");
    assert_eq!(loaded.status(), 200);
    let body = loaded.text().await.expect("body");
    assert!(body.contains("id=\"comment_9001\""), "{body}");
    assert!(
        body.contains("view_full_work=true"),
        "entire-work reader keeps its mode in the hide link: {body}"
    );
    let work_set = store
        .get_comment_set("12345", ao3instance::store::WORK_SCOPE)
        .await
        .expect("meta")
        .expect("fetched");
    assert_eq!(work_set.total, Some(4));
    assert!(
        store
            .get_comments("12345", &chapter_scope("78901"))
            .await
            .expect("chapter cache")
            .is_empty(),
        "the work scope did not write the chapter scope"
    );

    // A one-chapter work has no chapter commentable: AO3 comments on the work.
    let single_loaded = post_comments(&root, "/read/654321/comments", "chapter=1")
        .send()
        .await
        .expect("load one-shot comments");
    assert_eq!(single_loaded.status(), 200);
    assert!(
        store
            .get_comment_set("654321", ao3instance::store::WORK_SCOPE)
            .await
            .expect("meta")
            .is_some(),
        "one-chapter works cache under the work scope"
    );
    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_cached_thread_and_explains_itself() {
    let upstream = Server::start_sequence(vec![(
        "GET /works/12345/chapters/78901?show_comments=true",
        vec![
            Response::ok(comments_fixture()),
            Response::new(500),
            Response::new(403),
        ],
    )]);
    let (root, store, dir, handle) = start(&upstream, false).await;
    seed_work(&store).await;

    let first = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("prime cache");
    assert_eq!(first.status(), 200);
    let primed = first.text().await.expect("body");
    assert!(primed.contains("id=\"comment_9001\""), "{primed}");

    // Refresh against a 500: the reader reports it and keeps the old thread.
    let failed = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("failed refresh");
    assert_eq!(failed.status(), 200);
    let body = failed.text().await.expect("body");
    assert!(
        body.contains("Could not load comments from AO3"),
        "honest notice: {body}"
    );
    assert!(
        body.contains("id=\"comment_9001\""),
        "cached thread survives: {body}"
    );
    assert_eq!(
        store
            .get_comments("12345", &chapter_scope("78901"))
            .await
            .expect("cache")
            .len(),
        4,
        "failed refresh never rewrites the cache"
    );

    // A blocked edge is explained distinctly but also keeps the cache.
    let blocked = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("blocked refresh");
    let blocked_body = blocked.text().await.expect("body");
    assert!(
        blocked_body.contains("blocked the comments request"),
        "{blocked_body}"
    );
    assert!(
        blocked_body.contains("id=\"comment_9001\""),
        "{blocked_body}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn an_empty_ao3_thread_is_cached_and_still_refreshable() {
    let empty = r##"<div id="feedback" class="feedback">
        <div id="comments_placeholder"><ol class="thread"></ol></div>
    </div>"##;
    let upstream = Server::start(vec![(
        "GET /works/12345/chapters/78901?show_comments=true",
        Response::ok(empty),
    )]);
    let (root, store, dir, handle) = start(&upstream, false).await;
    seed_work(&store).await;

    let loaded = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("load empty comments");
    assert_eq!(loaded.status(), 200);
    let body = loaded.text().await.expect("body");
    assert!(
        body.contains("No comments on AO3 for this work yet."),
        "{body}"
    );
    assert!(
        body.contains(">Refresh comments</button>"),
        "even a zero-comment cache can be refreshed: {body}"
    );

    let set = store
        .get_comment_set("12345", &chapter_scope("78901"))
        .await
        .expect("meta")
        .expect("fetched marker recorded for an empty thread");
    assert_eq!(set.total, None);
    assert!(
        store
            .get_comments("12345", &chapter_scope("78901"))
            .await
            .expect("cache")
            .is_empty()
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn an_adult_or_login_gate_is_explained_without_losing_cache() {
    let gate = r#"<html><body><div id="adult">
        <p>This work could have adult content. If you proceed, you have consented.</p>
        </div></body></html>"#;
    let upstream = Server::start_sequence(vec![(
        "GET /works/12345/chapters/78901?show_comments=true",
        vec![Response::ok(comments_fixture()), Response::ok(gate)],
    )]);
    let (root, store, dir, handle) = start(&upstream, false).await;
    seed_work(&store).await;

    let primed = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("prime cache");
    assert!(
        primed
            .text()
            .await
            .expect("body")
            .contains("id=\"comment_9001\"")
    );

    let gated = post_comments(&root, "/read/12345/comments", "chapter=1")
        .send()
        .await
        .expect("gated refresh");
    let body = gated.text().await.expect("body");
    assert!(
        body.contains("AO3 wants a signed-in session or enabled adult content"),
        "adult gate is explained: {body}"
    );
    assert!(
        body.contains("id=\"comment_9001\""),
        "the cached thread survives a gate: {body}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn loading_comments_for_an_uncached_work_is_a_clean_404() {
    let upstream = Server::empty();
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let response = post_comments(&root, "/read/999/comments", "chapter=1")
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 404);
    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
