use super::works::fts_query;
use super::*;
use crate::ao3::work::{Chapter, WorkPage};
use crate::db;
use crate::diag::{HttpDirection, HttpErrorEvent, Transport};

fn page() -> WorkPage {
    WorkPage {
        ao3_id: "123".into(),
        title: "A Test Work".into(),
        authors: vec!["tester".into()],
        summary: Some("summary".into()),
        fandoms: vec!["Fandom".into()],
        relationships: vec![],
        characters: vec![],
        additional_tags: vec!["Fluff".into()],
        warnings: vec!["No Archive Warnings Apply".into()],
        categories: vec!["F/M".into()],
        rating: Some("General Audiences".into()),
        language: Some("English".into()),
        word_count: Some(1234),
        kudos: Some(7),
        comment_count: Some(2),
        chapter_count: 2,
        chapters: vec![
            Chapter {
                index: 1,
                title: Some("One".into()),
                body_html: "<p>one</p>".into(),
                notes_html: None,
            },
            Chapter {
                index: 2,
                title: Some("Two".into()),
                body_html: "<p>two</p>".into(),
                notes_html: Some("<p>note</p>".into()),
            },
        ],
        comment_form_present: true,
    }
}

async fn test_store(tag: &str) -> (Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ao3i-store-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = db::open(&dir.join("test.db")).await.unwrap();
    (Store::new(pool), dir)
}

#[tokio::test]
async fn upsert_round_trip_and_replace() {
    let (store, dir) = test_store("roundtrip").await;
    store.upsert_work(&page(), "hash1").await.unwrap();

    let work = store.get_work("123").await.unwrap().expect("stored");
    assert_eq!(work.title, "A Test Work");
    assert_eq!(work.authors, vec!["tester".to_string()]);
    assert_eq!(work.tags.fandoms, vec!["Fandom".to_string()]);
    assert_eq!(work.tags.additional, vec!["Fluff".to_string()]);
    assert_eq!(work.chapter_count, 2);

    let chapters = store.get_chapters("123").await.unwrap();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[1].body_html, "<p>two</p>");
    assert_eq!(chapters[1].notes_html.as_deref(), Some("<p>note</p>"));

    // Re-fetch with fewer chapters must replace, not append.
    let mut second = page();
    second.chapters.truncate(1);
    second.title = "Renamed".into();
    store.upsert_work(&second, "hash2").await.unwrap();
    let work = store.get_work("123").await.unwrap().expect("stored");
    assert_eq!(work.title, "Renamed");
    assert_eq!(store.get_chapters("123").await.unwrap().len(), 1);

    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn list_returns_recent_first() {
    let (store, dir) = test_store("list").await;
    let mut a = page();
    a.ao3_id = "1".into();
    let mut b = page();
    b.ao3_id = "2".into();
    b.title = "Second".into();
    store.upsert_work(&a, "h1").await.unwrap();
    store.upsert_work(&b, "h2").await.unwrap();

    let works = store.list_works(10).await.unwrap();
    assert_eq!(works.len(), 2);
    assert_eq!(works[0].ao3_id, "2");

    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn session_and_progress_round_trip() {
    let (store, dir) = test_store("session").await;
    assert!(store.load_session().await.unwrap().is_none());

    store.save_session("someone", "ciphertext-1").await.unwrap();
    store.save_session("someone", "ciphertext-2").await.unwrap();
    let saved = store.load_session().await.unwrap().expect("session");
    assert_eq!(saved.account, "someone");
    assert_eq!(saved.jar_enc, "ciphertext-2");
    assert_eq!(saved.pseud_id, None);
    store
        .set_session_pseud("someone", 111, "Main Pseud")
        .await
        .unwrap();
    let saved = store.load_session().await.unwrap().expect("session");
    assert_eq!(saved.pseud_id, Some(111));
    assert_eq!(saved.pseud_name.as_deref(), Some("Main Pseud"));
    store.delete_session().await.unwrap();
    assert!(store.load_session().await.unwrap().is_none());

    store.upsert_work(&page(), "h").await.unwrap();
    assert!(store.get_progress("123").await.unwrap().is_none());
    store.save_progress("123", 2).await.unwrap();
    assert_eq!(store.get_progress("123").await.unwrap(), Some(2));
    let works = store.list_works(10).await.unwrap();
    assert_eq!(works[0].progress_chapter, Some(2));

    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn search_finds_title_author_and_tags() {
    let (store, dir) = test_store("search").await;
    store.upsert_work(&page(), "h1").await.unwrap();
    let mut other = page();
    other.ao3_id = "999".into();
    other.title = "Another Story".into();
    other.authors = vec!["secondwriter".into()];
    other.additional_tags = vec!["Angst".into()];
    store.upsert_work(&other, "h2").await.unwrap();

    let by_title = store.search_works("Test", 10).await.unwrap();
    assert_eq!(by_title.len(), 1);
    assert_eq!(by_title[0].ao3_id, "123");

    let by_author = store.search_works("secondwriter", 10).await.unwrap();
    assert_eq!(by_author[0].ao3_id, "999");

    let by_tag = store.search_works("angst", 10).await.unwrap();
    assert_eq!(by_tag[0].ao3_id, "999");

    // No results, blank query, and FTS syntax characters as data.
    assert!(
        store
            .search_works("nothingmatchesthis", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.search_works("   ", 10).await.unwrap().is_empty());
    assert!(
        store
            .search_works("\"; DROP TABLE works; --", 10)
            .await
            .unwrap()
            .is_empty()
    );

    // Update triggers keep the index in step.
    let mut renamed = page();
    renamed.title = "Renamed Work".into();
    renamed.authors = vec!["writer2".into()];
    store.upsert_work(&renamed, "h3").await.unwrap();
    assert_eq!(store.search_works("Renamed", 10).await.unwrap().len(), 1);
    // Both the old title and the old author are gone from the index.
    assert!(store.search_works("Test", 10).await.unwrap().is_empty());
    assert!(store.search_works("tester", 10).await.unwrap().is_empty());

    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn touch_checked_moves_fetched_at_forward() {
    let (store, dir) = test_store("touch").await;
    store.upsert_work(&page(), "h1").await.unwrap();
    let before = store
        .get_work("123")
        .await
        .unwrap()
        .expect("work")
        .fetched_at;
    sqlx::query("UPDATE works SET fetched_at = 1")
        .execute(store.pool())
        .await
        .unwrap();
    store.touch_checked("123").await.unwrap();
    let after = store
        .get_work("123")
        .await
        .unwrap()
        .expect("work")
        .fetched_at;
    assert!(after > 1 && after >= before);
    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn fts_query_quotes_tokens() {
    assert_eq!(
        fts_query("hello world").as_deref(),
        Some("\"hello\"* \"world\"*")
    );
    assert_eq!(fts_query("a\"b").as_deref(), Some("\"a\"\"b\"*"));
    assert_eq!(fts_query("  "), None);
}

#[tokio::test]
async fn write_ledger_and_subscription_round_trip() {
    let (store, dir) = test_store("writes").await;
    let intent = store
        .record_intent("someone", "subscribe", "Work", "123")
        .await
        .unwrap();
    store
        .finish_intent(intent, "done", "subscribed", Some("item 42"))
        .await
        .unwrap();
    let audit = store.recent_audit(10).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].action, "subscribe");
    assert_eq!(audit[0].target, "Work 123");
    assert_eq!(audit[0].outcome, "subscribed");

    assert!(
        store
            .get_subscription("someone", "Work", "123")
            .await
            .unwrap()
            .is_none()
    );
    store
        .upsert_subscription("someone", "Work", "123", Some(42), "active")
        .await
        .unwrap();
    let row = store
        .get_subscription("someone", "Work", "123")
        .await
        .unwrap()
        .expect("ledger row");
    assert_eq!(row.subscription_id, Some(42));
    assert_eq!(row.state, "active");

    store
        .upsert_subscription("someone", "Work", "123", None, "removed")
        .await
        .unwrap();
    let row = store
        .get_subscription("someone", "Work", "123")
        .await
        .unwrap()
        .expect("ledger row");
    assert_eq!(row.state, "removed");

    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn bookmark_receipts_are_idempotent_and_per_account() {
    let (store, dir) = test_store("bookmarks").await;
    assert!(!store.has_bookmark("someone", "123").await.unwrap());
    store
        .mark_bookmark("someone", "123", Some(900))
        .await
        .unwrap();
    store
        .mark_bookmark("someone", "123", Some(900))
        .await
        .unwrap();
    assert!(store.has_bookmark("someone", "123").await.unwrap());
    assert!(!store.has_bookmark("other", "123").await.unwrap());
    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn kudos_and_reading_state_round_trip() {
    let (store, dir) = test_store("kudos").await;
    assert!(!store.has_kudos("someone", "123").await.unwrap());
    store.mark_kudos("someone", "123").await.unwrap();
    store.mark_kudos("someone", "123").await.unwrap(); // idempotent
    assert!(store.has_kudos("someone", "123").await.unwrap());
    assert!(!store.has_kudos("other", "123").await.unwrap());

    assert!(
        store
            .get_reading_state("someone", "123")
            .await
            .unwrap()
            .is_none()
    );
    store
        .set_reading_state("someone", "123", "for_later")
        .await
        .unwrap();
    assert_eq!(
        store
            .get_reading_state("someone", "123")
            .await
            .unwrap()
            .as_deref(),
        Some("for_later")
    );
    store
        .set_reading_state("someone", "123", "read")
        .await
        .unwrap();
    assert_eq!(
        store
            .get_reading_state("someone", "123")
            .await
            .unwrap()
            .as_deref(),
        Some("read")
    );
    store.pool().close().await;
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn hashes_are_stable_and_distinct() {
    assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
    assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    assert_eq!(sha256_hex(b"abc").len(), 64);
}

#[tokio::test]
async fn http_errors_are_recorded_filtered_and_pruned() {
    let (store, dir) = test_store("http-errors").await;
    for index in 0..5 {
        let direction = if index % 2 == 0 {
            HttpDirection::Upstream
        } else {
            HttpDirection::Inbound
        };
        let event = HttpErrorEvent::new(
            direction,
            Transport::Direct,
            "GET",
            format!("/works/{index}?authenticity_token=secret"),
            "blocked_by_edge",
            "refused; cookie _otwarchive_session=secret-cookie",
        )
        .with_status(403)
        .with_latency(5);
        store.record_http_error(&event, 3).await.unwrap();
    }

    let rows = store.recent_http_errors(10, None).await.unwrap();
    assert_eq!(rows.len(), 3, "retention prunes the oldest rows");
    assert!(rows[0].id > rows[1].id, "newest first");
    assert_eq!(rows[0].path, "/works/4", "query strings are stripped");
    assert_eq!(rows[0].status, Some(403));
    assert_eq!(rows[0].transport, "direct");
    assert!(!rows[0].message.contains("secret-cookie"));
    assert!(!rows[0].message.contains("_otwarchive_session"));

    let upstream = store
        .recent_http_errors(10, Some("upstream"))
        .await
        .unwrap();
    assert!(!upstream.is_empty());
    assert!(upstream.iter().all(|row| row.direction == "upstream"));

    let cleared = store.clear_http_errors().await.unwrap();
    assert_eq!(cleared, 3);
    assert!(store.recent_http_errors(10, None).await.unwrap().is_empty());

    drop(std::fs::remove_dir_all(&dir));
}
