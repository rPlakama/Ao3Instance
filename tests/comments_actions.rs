#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Replies, edits and deletes for cached comments: one confirmed request each,
//! AO3's own comment rules mirrored (own comments only; no edits once replied
//! to), cache refresh afterwards, audit rows.

mod support;

use ao3instance::ao3::parse::{SpaceMode, percent_encode};
use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

fn comments_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/work_comments_own.html").expect("comments fixture")
}

fn pseuds_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/pseuds.html").expect("pseuds fixture")
}

fn comment_load_path() -> &'static str {
    "GET /works/12345/chapters/78901?show_comments=true"
}

fn base_routes(refresh_responses: usize) -> Vec<(String, Vec<Response>)> {
    let mut routes: Vec<(String, Vec<Response>)> = session_routes(vec![])
        .into_iter()
        .map(|(key, response)| (key, vec![response]))
        .collect();
    routes.push((
        "GET /users/someone/pseuds".to_string(),
        vec![Response::ok(pseuds_fixture())],
    ));
    routes.push((
        comment_load_path().to_string(),
        (0..refresh_responses)
            .map(|_| Response::ok(comments_fixture()))
            .collect(),
    ));
    routes
}

fn post_encoded(path: &str, fields: &[(&str, &str)]) -> reqwest::RequestBuilder {
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
        .post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
}

async fn load_comments(root: &str) {
    http()
        .post(format!("{root}/read/12345/comments"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("chapter=1")
        .send()
        .await
        .expect("load comments");
}

#[tokio::test]
async fn reader_offers_reply_and_only_safe_comment_actions() {
    let upstream = Server::start_sequence(base_routes(1));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    load_comments(&root).await;

    let reader = client
        .get(format!("{root}/read/12345?chapter=1&show_comments=1"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(
        reader.contains("/write/comment/reply?work_id=12345&amp;comment_id=9001"),
        "reply link on another person's comment: {reader}"
    );
    assert!(
        reader.contains("/write/comment/edit?work_id=12345&amp;comment_id=9006"),
        "edit link on my leaf comment: {reader}"
    );
    assert!(
        reader.contains("/write/comment/delete?work_id=12345&amp;comment_id=9006"),
        "delete link on my leaf comment: {reader}"
    );
    assert!(
        !reader.contains("/write/comment/edit?work_id=12345&amp;comment_id=9001"),
        "no edit link on someone else's comment: {reader}"
    );
    assert!(
        !reader.contains("/write/comment/edit?work_id=12345&amp;comment_id=9007"),
        "no edit link once a comment has replies: {reader}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn reply_confirms_posts_once_and_refreshes_the_thread() {
    let mut routes = base_routes(2);
    routes.push((
        "POST /comments/9001/comments".to_string(),
        vec![Response::redirect("/works/12345#comment_9010")],
    ));
    let upstream = Server::start_sequence(routes);
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    load_comments(&root).await;

    let editor = client
        .get(format!(
            "{root}/write/comment/reply?work_id=12345&comment_id=9001"
        ))
        .send()
        .await
        .expect("reply editor")
        .text()
        .await
        .expect("body");
    assert!(editor.contains("Reply to Comment"), "{editor}");
    assert!(editor.contains("Reply"), "{editor}");

    let preview = post_encoded(
        &format!("{root}/write/comment/reply/preview"),
        &[
            ("work_id", "12345"),
            ("comment_id", "9001"),
            ("content", "A considered reply."),
        ],
    )
    .send()
    .await
    .expect("preview")
    .text()
    .await
    .expect("body");
    assert!(preview.contains("Confirm Reply"), "{preview}");

    let body = post_encoded(
        &format!("{root}/write/comment/reply"),
        &[
            ("work_id", "12345"),
            ("comment_id", "9001"),
            ("content", "A considered reply."),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("reply")
    .text()
    .await
    .expect("body");
    assert!(body.contains("Comment posted on AO3."), "{body}");
    let posts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/comments/9001/comments")
        .collect();
    assert_eq!(posts.len(), 1, "exactly one reply request");
    assert!(
        posts[0]
            .body
            .contains("comment%5Bcomment_content%5D=A+considered+reply."),
        "{}",
        posts[0].body
    );
    assert!(
        posts[0].body.contains("comment%5Bpseud_id%5D="),
        "{}",
        posts[0].body
    );
    assert!(
        upstream
            .requests()
            .iter()
            .filter(|request| request.path == comment_load_path().trim_start_matches("GET "))
            .count()
            >= 2,
        "the thread is refreshed after the reply"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    let row = audit
        .iter()
        .find(|entry| entry.action == "reply_comment")
        .expect("reply audit row");
    assert_eq!(row.outcome, "replied");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn editing_own_comment_prefills_and_patches_once() {
    let mut routes = base_routes(2);
    routes.push((
        "PATCH /comments/9006".to_string(),
        vec![Response::redirect("/works/12345#comment_9006")],
    ));
    let upstream = Server::start_sequence(routes);
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    load_comments(&root).await;

    let editor = client
        .get(format!(
            "{root}/write/comment/edit?work_id=12345&comment_id=9006"
        ))
        .send()
        .await
        .expect("edit editor")
        .text()
        .await
        .expect("body");
    assert!(editor.contains("Edit Comment"), "{editor}");
    assert!(editor.contains("My own leaf comment."), "{editor}");

    let preview = post_encoded(
        &format!("{root}/write/comment/edit/preview"),
        &[
            ("work_id", "12345"),
            ("comment_id", "9006"),
            ("content", "My edited leaf comment."),
        ],
    )
    .send()
    .await
    .expect("preview")
    .text()
    .await
    .expect("body");
    assert!(preview.contains("Confirm Comment Edit"), "{preview}");

    let body = post_encoded(
        &format!("{root}/write/comment/edit"),
        &[
            ("work_id", "12345"),
            ("comment_id", "9006"),
            ("content", "My edited leaf comment."),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("edit")
    .text()
    .await
    .expect("body");
    assert!(body.contains("Comment updated on AO3."), "{body}");
    let patches: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "PATCH" && request.path == "/comments/9006")
        .collect();
    assert_eq!(patches.len(), 1);
    assert!(
        patches[0]
            .body
            .contains("comment%5Bcomment_content%5D=My+edited+leaf+comment."),
        "{}",
        patches[0].body
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "edit_comment" && entry.outcome == "comment-updated"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn deleting_own_comment_warns_then_deletes_once() {
    let mut routes = base_routes(2);
    routes.push((
        "DELETE /comments/9006".to_string(),
        vec![Response::redirect("/works/12345#comment_9006")],
    ));
    let upstream = Server::start_sequence(routes);
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    load_comments(&root).await;

    let confirm = client
        .get(format!(
            "{root}/write/comment/delete?work_id=12345&comment_id=9006"
        ))
        .send()
        .await
        .expect("delete page")
        .text()
        .await
        .expect("body");
    assert!(confirm.contains("Delete Comment"), "{confirm}");
    assert!(confirm.contains("cannot be undone"), "{confirm}");

    let body = post_encoded(
        &format!("{root}/write/comment/delete"),
        &[
            ("work_id", "12345"),
            ("comment_id", "9006"),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("delete")
    .text()
    .await
    .expect("body");
    assert!(body.contains("Comment deleted on AO3."), "{body}");
    assert_eq!(
        upstream
            .requests()
            .into_iter()
            .filter(|request| request.method == "DELETE" && request.path == "/comments/9006")
            .count(),
        1
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "delete_comment" && entry.outcome == "comment-deleted"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn other_peoples_comments_cannot_be_edited_or_deleted() {
    let upstream = Server::start_sequence(base_routes(1));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    load_comments(&root).await;

    for path in [
        "/write/comment/edit?work_id=12345&comment_id=9001",
        "/write/comment/delete?work_id=12345&comment_id=9001",
    ] {
        let response = client
            .get(format!("{root}{path}"))
            .send()
            .await
            .expect("guarded page");
        assert_eq!(response.status(), 403, "{path} must be forbidden");
    }
    // My own comment that already has a reply cannot be edited (AO3's rule).
    let replied = client
        .get(format!(
            "{root}/write/comment/edit?work_id=12345&comment_id=9007"
        ))
        .send()
        .await
        .expect("replied comment");
    assert_eq!(replied.status(), 400);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn comment_actions_need_the_write_switch() {
    let upstream = Server::start_sequence(base_routes(1));
    let (root, store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    load_comments(&root).await;

    let response = client
        .get(format!(
            "{root}/write/comment/reply?work_id=12345&comment_id=9001"
        ))
        .send()
        .await
        .expect("reply page");
    assert_eq!(response.status(), 403);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
