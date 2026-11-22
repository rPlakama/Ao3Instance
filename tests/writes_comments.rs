#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! W6: comments end to end — pseud discovery, confirmation, one request, typed
//! failure classification, audit.

mod support;

use ao3instance::ao3::parse::{SpaceMode, percent_encode};
use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

fn pseuds_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/pseuds.html").expect("pseuds fixture")
}

fn comment_routes(comment_responses: Vec<Response>) -> Vec<(String, Vec<Response>)> {
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
    routes.push(("POST /works/12345/comments".to_string(), comment_responses));
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

#[tokio::test]
async fn comment_is_discovered_confirmed_posted_and_audited() {
    let upstream = Server::start_sequence(comment_routes(vec![
        Response::new(302).with_header("location", "/works/12345#comment_777"),
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
    assert!(reader.contains("Comment as"), "AO3 comment box label");
    assert!(reader.contains("Comment on this work"), "{reader}");

    // Preview discovers the default pseud and shows the exact text.
    let preview = post_encoded(
        &format!("{root}/write/comment/preview"),
        &[("target_id", "12345"), ("content", "Hello world")],
    )
    .send()
    .await
    .expect("preview");
    assert_eq!(preview.status(), 200);
    let preview_body = preview.text().await.expect("body");
    assert!(preview_body.contains("Confirm comment"), "{preview_body}");
    assert!(preview_body.contains("Main Pseud"), "{preview_body}");
    assert!(preview_body.contains("Hello world"), "{preview_body}");

    let pseuds_request = upstream
        .requests()
        .into_iter()
        .find(|request| request.path == "/users/someone/pseuds")
        .expect("pseud discovery request");
    assert_eq!(pseuds_request.method, "GET");

    // Execute.
    let posted = post_encoded(
        &format!("{root}/write/comment"),
        &[
            ("target_id", "12345"),
            ("content", "Hello world"),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("comment");
    assert_eq!(posted.status(), 200);
    let body = posted.text().await.expect("body");
    assert!(body.contains("Comment posted on AO3."), "{body}");

    let posts: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path == "/works/12345/comments")
        .collect();
    assert_eq!(posts.len(), 1, "exactly one comment request");
    assert!(posts[0].body.contains("comment%5Bpseud_id%5D=111"));
    assert!(
        posts[0]
            .body
            .contains("comment%5Bcomment_content%5D=Hello+world")
    );
    assert!(posts[0].body.contains("authenticity_token=action-token"));

    let saved = store.load_session().await.expect("session").expect("saved");
    assert_eq!(saved.pseud_id, Some(111), "pseud remembered for next time");
    assert_eq!(saved.pseud_name.as_deref(), Some("Main Pseud"));

    let audit = store.recent_audit(5).await.expect("audit");
    assert_eq!(audit[0].action, "comment");
    assert_eq!(audit[0].outcome, "commented");
    assert!(audit[0].evidence_or_empty().contains("777"));

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn duplicate_comment_is_typed_and_audited_once() {
    let duplicate = r#"<html><body><div class="error">You've already left this comment here. (It may not appear right away for performance reasons.)</div><p>Couldn't save comment!</p></body></html>"#;
    let upstream = Server::start_sequence(comment_routes(vec![
        Response::ok(duplicate).with_header("content-type", "text/html; charset=utf-8"),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let response = post_encoded(
        &format!("{root}/write/comment"),
        &[
            ("target_id", "12345"),
            ("content", "Hello world"),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("comment");
    let body = response.text().await.expect("body");
    assert!(body.contains("AO3 rejected the comment"), "{body}");
    let posts = upstream
        .requests()
        .into_iter()
        .filter(|request| request.path == "/works/12345/comments")
        .count();
    assert_eq!(posts, 1, "one attempt, never retried");
    let audit = store.recent_audit(5).await.expect("audit");
    assert_eq!(audit[0].outcome, "failed");
    assert!(audit[0].evidence_or_empty().contains("already left"));

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn spam_and_rate_limits_have_their_own_notices() {
    let spam = r#"<html><body><p>This comment looks like spam to our system, sorry! Please try again.</p></body></html>"#;
    let upstream = Server::start_sequence(comment_routes(vec![
        Response::ok(spam).with_header("content-type", "text/html; charset=utf-8"),
        Response::new(429),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let spam_response = post_encoded(
        &format!("{root}/write/comment"),
        &[
            ("target_id", "12345"),
            ("content", "buy stuff"),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("spam attempt");
    let spam_body = spam_response.text().await.expect("body");
    assert!(spam_body.contains("spam filter"), "{spam_body}");

    let rate_response = post_encoded(
        &format!("{root}/write/comment"),
        &[
            ("target_id", "12345"),
            ("content", "second try"),
            ("confirm", "yes"),
        ],
    )
    .send()
    .await
    .expect("rate attempt");
    let rate_body = rate_response.text().await.expect("body");
    assert!(rate_body.contains("3 per 15 minutes"), "{rate_body}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn overlong_or_unconfirmed_comments_never_reach_ao3() {
    let upstream = Server::start_sequence(comment_routes(vec![
        Response::new(302).with_header("location", "/works/12345#comment_1"),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;

    let long = "x".repeat(ao3instance::ao3::actions::COMMENT_MAX_CHARS + 1);
    let overlong = post_encoded(
        &format!("{root}/write/comment/preview"),
        &[("target_id", "12345"), ("content", long.as_str())],
    )
    .send()
    .await
    .expect("overlong preview");
    assert_eq!(overlong.status(), 400);

    let unconfirmed = post_encoded(
        &format!("{root}/write/comment"),
        &[("target_id", "12345"), ("content", "hello")],
    )
    .send()
    .await
    .expect("unconfirmed");
    assert_eq!(unconfirmed.status(), 400);

    assert!(
        upstream
            .requests()
            .iter()
            .all(|request| request.path != "/works/12345/comments"),
        "no comment request may be sent for invalid input"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
