use super::works::fts_query;
use super::*;
use crate::ao3::work::{Chapter, WorkPage};
use crate::db;
use crate::diag::{HttpDirection, HttpErrorEvent, Transport};
use crate::store::{BrowseSort, ReadingStatus, WorkFilter};

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

#[tokio::test]
async fn solver_events_and_clearance_round_trip() {
    let (store, dir) = test_store("solver-events").await;

    store
        .record_solver_event(
            "solved",
            "/works/1?view_full_work=true",
            Some(200),
            Some(900),
            200,
        )
        .await
        .unwrap();
    store
        .record_solver_event("captcha", "/works/2?view_full_work=true", None, None, 200)
        .await
        .unwrap();

    let events = store.recent_solver_events(10).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].outcome, "captcha");
    assert_eq!(events[0].status, None);
    assert_eq!(events[1].outcome, "solved");
    assert_eq!(events[1].latency_ms, Some(900));

    assert_eq!(store.solves_since(0).await.unwrap(), 1);
    assert!(store.last_solved_at().await.unwrap().is_some());

    store
        .save_solver_clearance("deadbeef", "Mozilla/5.0", 42)
        .await
        .unwrap();
    let clearance = store
        .load_solver_clearance()
        .await
        .unwrap()
        .expect("clearance row");
    assert_eq!(clearance.ciphertext, "deadbeef");
    assert_eq!(clearance.user_agent, "Mozilla/5.0");

    store.clear_solver_clearance().await.unwrap();
    assert!(store.load_solver_clearance().await.unwrap().is_none());

    drop(std::fs::remove_dir_all(&dir));
}

fn page_titled(id: &str, title: &str, fandom: &str, rating: &str) -> WorkPage {
    let mut page = page();
    page.ao3_id = id.into();
    page.title = title.into();
    page.fandoms = vec![fandom.into()];
    page.rating = Some(rating.into());
    page
}

#[tokio::test]
async fn browse_and_local_lists_read_only_from_the_cache() {
    let (store, dir) = test_store("lists").await;
    store
        .upsert_work(
            &page_titled("123", "A Test Work", "Fandom", "General Audiences"),
            "h1",
        )
        .await
        .unwrap();
    let mut second = page_titled("456", "Second Work", "Other", "Mature");
    second.authors = vec!["SecondAuthor".into()];
    store.upsert_work(&second, "h2").await.unwrap();
    store.save_progress("123", 2).await.unwrap();
    store.save_progress("456", 1).await.unwrap();
    store
        .set_reading_state("reader", "123", "for_later")
        .await
        .unwrap();
    store.mark_bookmark("reader", "456", Some(9)).await.unwrap();
    store
        .upsert_subscription("reader", "Work", "123", Some(77), "active")
        .await
        .unwrap();

    let fandoms = store.list_fandoms().await.unwrap();
    assert_eq!(
        fandoms,
        vec![("Fandom".to_string(), 1), ("Other".to_string(), 1)]
    );

    let mut filter = WorkFilter::default();
    let browse = store.browse_works("reader", &filter).await.unwrap();
    assert_eq!(browse.total, 2);
    assert_eq!(
        browse.works[0].ao3_id,
        "456",
        "newest tie broken by id; got {:?}",
        browse
            .works
            .iter()
            .map(|work| &work.ao3_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(browse.page, 1);

    filter.fandom = Some("other".into());
    let filtered = store.browse_works("reader", &filter).await.unwrap();
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.works[0].ao3_id, "456");

    filter.fandom = None;
    filter.author = Some("secondauthor".into());
    let by_author = store.browse_works("reader", &filter).await.unwrap();
    assert_eq!(by_author.total, 1, "author matching is case-insensitive");
    assert_eq!(by_author.works[0].ao3_id, "456");
    filter.author = Some("missing".into());
    assert_eq!(
        store.browse_works("reader", &filter).await.unwrap().total,
        0
    );
    filter.author = None;

    filter.fandom = None;
    filter.reading_status = ReadingStatus::Bookmarked;
    let bookmarked = store.browse_works("reader", &filter).await.unwrap();
    assert_eq!(bookmarked.total, 1);
    assert_eq!(bookmarked.works[0].ao3_id, "456");

    filter.reading_status = ReadingStatus::InProgress;
    let in_progress = store.browse_works("reader", &filter).await.unwrap();
    assert_eq!(in_progress.total, 2, "both works have saved progress");
    assert!(in_progress.works.iter().any(|work| work.ao3_id == "123"));

    filter.reading_status = ReadingStatus::All;
    filter.sort = BrowseSort::TitleAsc;
    let sorted = store.browse_works("reader", &filter).await.unwrap();
    assert_eq!(sorted.works[0].ao3_id, "123", "A Test Work sorts first");

    let subscriptions = store.list_subscriptions("reader").await.unwrap();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].title.as_deref(), Some("A Test Work"));

    let bookmarks = store.list_bookmarks("reader").await.unwrap();
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].title.as_deref(), Some("Second Work"));

    let reading = store.list_reading_history("reader").await.unwrap();
    assert_eq!(reading.len(), 2);
    let with_progress = reading.iter().find(|entry| entry.work_id == "123").unwrap();
    assert_eq!(with_progress.progress_chapter, Some(2));
    assert_eq!(with_progress.reading_state.as_deref(), Some("for_later"));

    let stats = store.cache_stats().await.unwrap();
    assert_eq!(stats.works, 2);
    assert_eq!(stats.chapters, 4);
    assert_eq!(stats.subscriptions, 1);
    assert_eq!(stats.bookmarks, 1);
    assert_eq!(stats.kudos, 0);

    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn search_pages_report_totals_and_offsets() {
    let (store, dir) = test_store("search-pages").await;
    for index in 1..=3 {
        let mut work = page();
        work.ao3_id = index.to_string();
        work.title = format!("Shared Title {index}");
        store
            .upsert_work(&work, &format!("hash-{index}"))
            .await
            .unwrap();
    }

    let first = store.search_works_page("Shared", 1, 2).await.unwrap();
    assert_eq!(first.total, 3);
    assert_eq!(first.works.len(), 2);
    assert_eq!(first.page, 1);
    assert_eq!(first.per_page, 2);

    let second = store.search_works_page("Shared", 2, 2).await.unwrap();
    assert_eq!(second.total, 3);
    assert_eq!(second.works.len(), 1);
    let first_ids: Vec<&str> = first
        .works
        .iter()
        .map(|work| work.ao3_id.as_str())
        .collect();
    assert!(
        !first_ids.contains(&second.works[0].ao3_id.as_str()),
        "pages must not overlap"
    );

    let none = store.search_works_page("NoSuchToken", 1, 20).await.unwrap();
    assert_eq!(none.total, 0);
    assert!(none.works.is_empty());

    let blank = store.search_works_page("   ", 1, 20).await.unwrap();
    assert_eq!(blank.total, 0);

    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn filtered_search_narrows_and_can_reorder() {
    let (store, dir) = test_store("search-filtered").await;
    let cases = [
        ("1", "Explicit", "Fandom", 100),
        ("2", "Mature", "Other", 300),
        ("3", "Explicit", "Other", 200),
    ];
    for (id, rating, fandom, words) in cases {
        let mut work = page();
        work.ao3_id = id.into();
        work.title = format!("Filtered Title {id}");
        work.rating = Some(rating.into());
        work.fandoms = vec![fandom.into()];
        work.word_count = Some(words);
        store.upsert_work(&work, id).await.unwrap();
    }

    let rating = SearchFilter {
        rating: Some("explicit".into()),
        ..SearchFilter::default()
    };
    let explicit = store
        .search_works_filtered("Filtered", &rating, 1, 20)
        .await
        .unwrap();
    assert_eq!(explicit.total, 2);

    let both = SearchFilter {
        rating: Some("Explicit".into()),
        fandom: Some("other".into()),
        ..SearchFilter::default()
    };
    let narrowed = store
        .search_works_filtered("Filtered", &both, 1, 20)
        .await
        .unwrap();
    assert_eq!(narrowed.total, 1);
    assert_eq!(narrowed.works[0].ao3_id, "3");

    let by_words = SearchFilter {
        sort: Some("words".into()),
        direction: Some("desc".into()),
        ..SearchFilter::default()
    };
    let sorted = store
        .search_works_filtered("Filtered", &by_words, 1, 20)
        .await
        .unwrap();
    let ids: Vec<&str> = sorted
        .works
        .iter()
        .map(|work| work.ao3_id.as_str())
        .collect();
    assert_eq!(ids, vec!["2", "3", "1"]);

    let by_title = SearchFilter {
        title: Some("title 2".into()),
        ..SearchFilter::default()
    };
    let titled = store
        .search_works_filtered("Filtered", &by_title, 1, 20)
        .await
        .unwrap();
    assert_eq!(titled.total, 1);
    assert_eq!(titled.works[0].ao3_id, "2");

    let words_range = SearchFilter {
        words_min: Some(250),
        words_max: Some(350),
        ..SearchFilter::default()
    };
    let ranged = store
        .search_works_filtered("Filtered", &words_range, 1, 20)
        .await
        .unwrap();
    assert_eq!(ranged.total, 1);
    assert_eq!(ranged.works[0].ao3_id, "2");

    let single = SearchFilter {
        single_chapter: true,
        ..SearchFilter::default()
    };
    let singles = store
        .search_works_filtered("Filtered", &single, 1, 20)
        .await
        .unwrap();
    assert_eq!(singles.total, 0, "fixture works have two chapters");

    let filter_only = SearchFilter {
        fandom: Some("Other".into()),
        ..SearchFilter::default()
    };
    let filtered_only = store
        .search_works_filtered("", &filter_only, 1, 20)
        .await
        .unwrap();
    assert_eq!(
        filtered_only.total, 2,
        "blank query + filter searches the cache"
    );

    let blank = store
        .search_works_filtered("   ", &SearchFilter::default(), 1, 20)
        .await
        .unwrap();
    assert_eq!(blank.total, 0);

    drop(std::fs::remove_dir_all(&dir));
}
