#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Session-state honesty on the reader: signed-out, writes-disabled and
//! unusable-session states must read differently, and an unusable stored
//! session must never be persisted over or shown as a plain sign-out.

mod support;

use ao3instance::ao3::jar::Jar;
use ao3instance::config::Config;
use ao3instance::crypto::SecretKey;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::{config_for, http, seed_work, session_routes, sign_in, start};
use support::{Response, Server};

async fn serve_custom(config: Config, store: Store) -> (String, tokio::task::JoinHandle<()>) {
    let app = server::router(&config, store).await.expect("router");
    support::serve(app).await
}

#[tokio::test]
async fn unusable_stored_session_is_cleared_and_explained_not_a_sign_out() {
    let dir = support::temp_dir("session-recovery");
    let config = config_for(&dir, "http://127.0.0.1:1", false);
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    // A jar that lost its AO3 cookies (exactly what a wiped jar leaves behind).
    let key = SecretKey::load_or_create(&config.key_file).expect("key");
    let stale = Jar::new().encrypt(&key).expect("encrypt an incomplete jar");
    store
        .save_session("someone", &stale)
        .await
        .expect("seed stale session");
    seed_work(&store).await;
    let (root, handle) = serve_custom(config, store.clone()).await;
    let client = http();

    // Startup must clear the unusable row instead of restoring it forever.
    assert!(
        store.load_session().await.expect("load session").is_none(),
        "an unauthenticatable stored jar is cleared"
    );

    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(
        reader.contains("Your saved AO3 session is incomplete or expired"),
        "page-level recovery banner: {reader}"
    );
    assert!(
        reader.contains("Re-import AO3 session"),
        "the action bar offers the fix instead of a sign-in link: {reader}"
    );
    assert!(
        reader.contains("cannot comment as you"),
        "comment area explains the unusable session: {reader}"
    );
    assert!(
        !reader.contains("Sign in to AO3 on the"),
        "a remembered-but-unusable session is not reported as signed out: {reader}"
    );

    // The adult opt-in used to persist the wiped jar over a good one; it must
    // now refuse and leave the store untouched.
    let adult = client
        .post(format!("{root}/session/adult"))
        .send()
        .await
        .expect("adult toggle");
    let adult_body = adult.text().await.expect("body");
    assert!(
        adult_body.contains("re-import your cookies before enabling adult content"),
        "{adult_body}"
    );
    assert!(
        store.load_session().await.expect("load session").is_none(),
        "refusing the toggle never writes a session row"
    );

    let session_page = client
        .get(format!("{root}/session"))
        .send()
        .await
        .expect("session page")
        .text()
        .await
        .expect("body");
    assert!(
        session_page.contains("Writes: disabled"),
        "the session page states the operator write switch: {session_page}"
    );
    assert!(
        session_page.contains("can no longer authenticate"),
        "the session page explains the cleared jar: {session_page}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn signed_in_with_writes_disabled_is_explained_as_such() {
    let upstream = Server::start(session_routes(vec![]));
    let (root, store, dir, handle) = start(&upstream, false).await;
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
    assert!(
        reader.contains("Writes are disabled on this AO3Instance"),
        "signed-in reader explains the operator switch: {reader}"
    );
    assert!(
        reader.contains("Writes disabled</a>"),
        "action bar shows the write state: {reader}"
    );
    assert!(
        !reader.contains("Sign in to AO3 on the"),
        "a signed-in account is never told to sign in: {reader}"
    );
    assert!(
        !reader.contains("action=\"/write/comment/preview\""),
        "writes stay off: {reader}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn signed_out_reader_keeps_the_sign_in_prompt() {
    let upstream = Server::start(Vec::<(String, Response)>::new());
    let (root, store, dir, handle) = start(&upstream, false).await;
    seed_work(&store).await;

    let reader = http()
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(
        reader.contains("Sign in to AO3 on the"),
        "signed-out reader still prompts sign-in: {reader}"
    );
    assert!(
        !reader.contains("Writes are disabled on this AO3Instance"),
        "{reader}"
    );
    assert!(
        !reader.contains("Your saved AO3 session is incomplete"),
        "{reader}"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
