#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Bookmark editing and deletion: prefilled from AO3's own edit page, one
//! confirmed request each, ledger/cache maintenance and audit rows.

mod support;

use support::harness::{http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

const BOOKMARKS_HTML: &str = r#"<html><body>
  <h2 class="heading">1 Bookmark</h2>
  <ol class="bookmark index group">
    <li id="bookmark_501" class="bookmark blurb group">
      <h4 class="heading"><a href="/works/7701">The Clockwork Ferry</a></h4>
      <p class="datetime">Bookmarked 01 Nov 2026</p>
      <ul class="meta tags"><li><a class="tag" href="/tags/Fluff">Fluff</a></li></ul>
    </li>
  </ol>
</body></html>"#;

fn edit_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/ao3_bookmark_edit.html").expect("edit fixture")
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

#[tokio::test]
async fn bookmark_list_offers_editing_and_deletion_for_own_bookmarks() {
    let upstream = Server::start(session_routes(vec![(
        "GET /users/someone/bookmarks",
        Response::ok(BOOKMARKS_HTML),
    )]));
    let (root, _store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let body = client
        .get(format!("{root}/bookmarks"))
        .send()
        .await
        .expect("bookmarks")
        .text()
        .await
        .expect("body");
    assert!(
        body.contains("/write/bookmark/edit?bookmark_id=501"),
        "edit link on own bookmark: {body}"
    );
    assert!(
        body.contains("/write/bookmark/delete?bookmark_id=501"),
        "delete link on own bookmark: {body}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn editing_a_bookmark_prefills_confirms_and_patches_once() {
    let upstream = Server::start(session_routes(vec![
        ("GET /users/someone/bookmarks", Response::ok(BOOKMARKS_HTML)),
        ("GET /bookmarks/501/edit", Response::ok(edit_fixture())),
        ("PATCH /bookmarks/501", Response::redirect("/bookmarks/501")),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;

    let editor = client
        .get(format!(
            "{root}/write/bookmark/edit?bookmark_id=501&work_id=7701"
        ))
        .send()
        .await
        .expect("editor")
        .text()
        .await
        .expect("body");
    assert!(editor.contains("Edit Bookmark"), "{editor}");
    assert!(editor.contains("The Clockwork Ferry"), "{editor}");
    assert!(
        editor.contains("PLACEHOLDER BOOKMARK NOTE"),
        "prefilled notes: {editor}"
    );
    assert!(editor.contains("Fluff, Comfort Reading"), "{editor}");
    assert!(
        editor.contains("value=\"1\" checked"),
        "private/rec flags: {editor}"
    );

    let fields = vec![
        ("bookmark_id", "501".to_string()),
        ("work_id", "7701".to_string()),
        ("notes", "Edited bookmark note.".to_string()),
        ("tags", "Fluff, Comfort Reading".to_string()),
        ("collections", String::new()),
        ("private", "1".to_string()),
    ];
    let preview = post_encoded(&root, "/write/bookmark/edit/preview", &fields)
        .send()
        .await
        .expect("preview")
        .text()
        .await
        .expect("body");
    assert!(preview.contains("Confirm Bookmark Edit"), "{preview}");
    assert!(preview.contains("Edited bookmark note."), "{preview}");
    assert!(
        !upstream
            .requests()
            .iter()
            .any(|request| request.method == "PATCH"),
        "preview never writes"
    );

    let mut confirmed = fields;
    confirmed.push(("confirm", "yes".to_string()));
    let body = post_encoded(&root, "/write/bookmark/edit", &confirmed)
        .send()
        .await
        .expect("edit")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Bookmark updated on AO3."), "{body}");
    let patches: Vec<_> = upstream
        .requests()
        .into_iter()
        .filter(|request| request.method == "PATCH" && request.path == "/bookmarks/501")
        .collect();
    assert_eq!(patches.len(), 1, "exactly one bookmark update");
    let sent = &patches[0].body;
    for needle in [
        "bookmark%5Bbookmarker_notes%5D=Edited+bookmark+note.",
        "bookmark%5Btag_string%5D=Fluff%2C+Comfort+Reading",
        "bookmark%5Bprivate%5D=1",
        "bookmark%5Brec%5D=0",
        "authenticity_token=",
    ] {
        assert!(sent.contains(needle), "missing {needle}: {sent}");
    }
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "edit_bookmark" && entry.outcome == "bookmark-updated"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn deleting_a_bookmark_warns_then_deletes_once_and_drops_the_ledger() {
    let upstream = Server::start(session_routes(vec![
        ("GET /users/someone/bookmarks", Response::ok(BOOKMARKS_HTML)),
        (
            "DELETE /bookmarks/501",
            Response::redirect("/users/someone/bookmarks"),
        ),
    ]));
    let (root, store, dir, handle) = start(&upstream, true).await;
    let client = http();
    sign_in(&client, &root).await;
    seed_work(&store).await;
    store
        .mark_bookmark("someone", "7701", Some(501))
        .await
        .expect("bookmark ledger");
    assert!(store.has_bookmark("someone", "7701").await.expect("has"));

    let confirm = client
        .get(format!(
            "{root}/write/bookmark/delete?bookmark_id=501&work_id=7701"
        ))
        .send()
        .await
        .expect("delete page")
        .text()
        .await
        .expect("body");
    assert!(confirm.contains("Delete Bookmark"), "{confirm}");
    assert!(confirm.contains("cannot be undone"), "{confirm}");

    let body = post_encoded(
        &root,
        "/write/bookmark/delete",
        &[
            ("bookmark_id", "501".to_string()),
            ("work_id", "7701".to_string()),
            ("confirm", "yes".to_string()),
        ],
    )
    .send()
    .await
    .expect("delete")
    .text()
    .await
    .expect("body");
    assert!(body.contains("Bookmark deleted on AO3."), "{body}");
    assert_eq!(
        upstream
            .requests()
            .into_iter()
            .filter(|request| request.method == "DELETE" && request.path == "/bookmarks/501")
            .count(),
        1
    );
    assert!(
        !store.has_bookmark("someone", "7701").await.expect("ledger"),
        "local receipt is removed"
    );
    let audit = store.recent_audit(5).await.expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "delete_bookmark" && entry.outcome == "bookmark-deleted"),
        "{audit:?}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn bookmark_editing_requires_the_write_switch() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, _store, dir, handle) = start(&upstream, false).await;
    let client = http();
    sign_in(&client, &root).await;

    let response = client
        .get(format!(
            "{root}/write/bookmark/edit?bookmark_id=501&work_id=7701"
        ))
        .send()
        .await
        .expect("editor");
    assert_eq!(response.status(), 403);

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
