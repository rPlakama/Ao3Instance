#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Phase 3/4 frontend baseline: every route renders the AO3-style chrome while
//! the upstream is unreachable, and the embedded stylesheet is served correctly.

mod support;

use ao3instance::config::Config;
use ao3instance::store::Store;
use ao3instance::{db, server};
use support::harness::http;

async fn serve(dir: &std::path::Path) -> (String, Store, tokio::task::JoinHandle<()>) {
    let config = Config::from_pairs([
        ("AO3I_BASE_URL", "http://127.0.0.1:1"),
        ("AO3I_DATA_DIR", dir.to_str().expect("utf8 dir")),
        ("XDG_CONFIG_HOME", dir.to_str().expect("utf8 dir")),
        ("AO3I_MIN_SPACING_MS", "0"),
    ])
    .expect("config");
    let pool = db::open(&config.db_file).await.expect("db");
    let store = Store::new(pool);
    let app = server::router(&config, store.clone())
        .await
        .expect("router");
    let (root, handle) = support::serve(app).await;
    (root, store, handle)
}

#[tokio::test]
async fn every_offline_segment_renders_the_shared_chrome() {
    let dir = support::temp_dir("frontend-walk");
    let (root, _store, handle) = serve(&dir).await;
    let client = http();

    for path in [
        "/",
        "/search?local=1&q=hello",
        "/works/search?local=1&q=hello",
        "/users/login",
        "/session",
        "/audit",
        "/diagnostics",
        "/works",
        "/fandoms",
        "/dashboard",
        "/subscriptions",
        "/bookmarks",
        "/reading",
        "/tags/Test/works",
        "/about",
    ] {
        let response = client
            .get(format!("{root}{path}"))
            .send()
            .await
            .unwrap_or_else(|error| panic!("GET {path}: {error}"));
        assert_eq!(response.status(), 200, "GET {path}");
        let body = response.text().await.expect("body");
        assert!(body.contains("id=\"header\""), "{path}: header");
        assert!(body.contains("id=\"footer\""), "{path}: footer");
        assert!(body.contains("class=\"primary navigation\""), "{path}: nav");
        assert!(body.contains("/assets/ao3.css"), "{path}: stylesheet link");
        assert!(
            !body.contains("archiveofourown.org/works/"),
            "{path}: offline pages must not link out from the chrome"
        );
    }

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn stylesheet_is_embedded_and_cache_friendly() {
    let dir = support::temp_dir("frontend-css");
    let (root, _store, handle) = serve(&dir).await;

    let response = http()
        .get(format!("{root}/assets/ao3.css"))
        .send()
        .await
        .expect("css");
    assert_eq!(response.status(), 200);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/css"), "{content_type}");
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=3600")
    );
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    let css = response.text().await.expect("css body");
    assert!(css.contains("--ao3-red: #900"), "AO3 palette");
    assert!(css.contains("ul.primary.navigation"), "primary nav");
    assert!(css.contains("work.blurb.group"), "blurb component");
    assert!(
        css.contains("@media (prefers-color-scheme: dark)"),
        "reversi"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn aliases_are_offline_and_uncached_works_stay_a_miss() {
    let dir = support::temp_dir("frontend-aliases");
    let (root, _store, handle) = serve(&dir).await;
    let client = http();

    let login = client
        .get(format!("{root}/users/login"))
        .send()
        .await
        .expect("login alias");
    assert_eq!(login.status(), 200);
    assert!(login.text().await.expect("body").contains("AO3 session"));

    let session = client
        .get(format!("{root}/session"))
        .send()
        .await
        .expect("session");
    assert_eq!(session.status(), 200);
    assert!(
        session
            .text()
            .await
            .expect("body")
            .contains("id=\"cookie-import\"")
    );

    let logout = client
        .get(format!("{root}/users/logout/confirm"))
        .send()
        .await
        .expect("logout confirm");
    assert_eq!(logout.status(), 200);
    let logout = logout.text().await.expect("body");
    assert!(logout.contains("Log out?"), "{logout}");
    assert!(logout.contains("action=\"/session/logout\""), "{logout}");

    let missing = client
        .get(format!("{root}/works/999999"))
        .send()
        .await
        .expect("uncached alias");
    assert_eq!(missing.status(), 404, "GET must never fetch upstream");
    assert!(
        missing
            .text()
            .await
            .expect("body")
            .contains("Not cached yet")
    );

    // Opening an uncached work with the upstream refused is a typed 502, quickly.
    let started = std::time::Instant::now();
    let open = client
        .post(format!("{root}/open"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("id=999999")
        .send()
        .await
        .expect("open");
    assert_eq!(open.status(), 502);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "offline failure must not hang"
    );

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn seeded_cache_renders_browse_fandoms_and_dashboard() {
    let dir = support::temp_dir("frontend-seeded");
    let (root, store, handle) = serve(&dir).await;
    support::harness::seed_work(&store).await;
    let client = http();

    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader");
    let reader = reader.text().await.expect("body");
    assert!(reader.contains("Entire Work"), "{reader}");
    assert!(!reader.contains("Chapter by Chapter"), "toggle is one-way");

    let full = client
        .get(format!("{root}/read/12345?view_full_work=true"))
        .send()
        .await
        .expect("full work");
    let full = full.text().await.expect("body");
    assert!(full.contains("Chapter by Chapter"), "{full}");
    assert!(
        full.matches("iframe class=\"chapter\"").count() >= 2,
        "every cached chapter is stacked: {full}"
    );

    let cached_alias = client
        .get(format!("{root}/works/12345"))
        .send()
        .await
        .expect("cached work alias");
    assert_eq!(cached_alias.status(), 200);
    assert!(
        cached_alias
            .text()
            .await
            .expect("body")
            .contains("PLACEHOLDER: The Map and the Mirror")
    );

    let chapter_alias = client
        .get(format!("{root}/works/12345/chapters/2"))
        .send()
        .await
        .expect("chapter alias");
    assert_eq!(
        chapter_alias.status(),
        200,
        "redirect followed by the client"
    );
    assert!(chapter_alias.text().await.expect("body").contains("iframe"));

    let search = client
        .get(format!("{root}/search?local=1&q=Mirror"))
        .send()
        .await
        .expect("search");
    let search = search.text().await.expect("body");
    assert!(search.contains("work blurb group"), "{search}");
    assert!(search.contains("required-tags"), "{search}");
    assert!(search.contains("Search Results"), "{search}");

    let local_form = client
        .get(format!("{root}/search?local=1"))
        .send()
        .await
        .expect("local form");
    let local_form = local_form.text().await.expect("body");
    assert!(local_form.contains("name=\"rating\""), "{local_form}");
    assert!(local_form.contains("name=\"fandom\""), "{local_form}");

    let filtered = client
        .get(format!("{root}/search?local=1&q=Mirror&rating=Explicit"))
        .send()
        .await
        .expect("filtered search");
    assert!(
        filtered
            .text()
            .await
            .expect("body")
            .contains("PLACEHOLDER: The Map and the Mirror")
    );

    let excluded = client
        .get(format!("{root}/search?local=1&q=Mirror&rating=Mature"))
        .send()
        .await
        .expect("excluded search");
    assert!(
        excluded
            .text()
            .await
            .expect("body")
            .contains("No results found")
    );

    let home = client.get(&root).send().await.expect("home");
    let home = home.text().await.expect("body");
    assert!(home.contains("id=\"login\""), "signed-out login dropdown");

    let browse = client
        .get(format!("{root}/works"))
        .send()
        .await
        .expect("browse");
    assert_eq!(browse.status(), 200);
    let body = browse.text().await.expect("body");
    assert!(
        body.contains("PLACEHOLDER: The Map and the Mirror"),
        "{body}"
    );
    assert!(body.contains("id=\"work-filters\""), "filter sidebar");
    assert!(body.contains("Test Fandom (Not Real)"), "fandom facet");

    let filtered = client
        .get(format!("{root}/works?rating=Explicit"))
        .send()
        .await
        .expect("filtered");
    assert_eq!(filtered.status(), 200);
    assert!(
        filtered
            .text()
            .await
            .expect("body")
            .contains("PLACEHOLDER: The Map and the Mirror")
    );

    let empty = client
        .get(format!("{root}/works?rating=Not%20A%20Rating"))
        .send()
        .await
        .expect("empty filter");
    assert!(
        empty
            .text()
            .await
            .expect("body")
            .contains("No cached work matches")
    );

    let fandoms = client
        .get(format!("{root}/fandoms"))
        .send()
        .await
        .expect("fandoms");
    assert!(
        fandoms
            .text()
            .await
            .expect("body")
            .contains("Test Fandom (Not Real)")
    );

    let tag_works = client
        .get(format!(
            "{root}/tags/Test%20Fandom%20%28Not%20Real%29/works"
        ))
        .send()
        .await
        .expect("tag works");
    let tag_works = tag_works.text().await.expect("body");
    assert!(
        tag_works.contains("1 Work in Test Fandom (Not Real)"),
        "{tag_works}"
    );
    assert!(tag_works.contains("Search this tag on AO3"), "{tag_works}");
    assert!(tag_works.contains("navigation actions"), "{tag_works}");

    let tag_search = client
        .get(format!("{root}/tags/search?q=Fandom"))
        .send()
        .await
        .expect("tag search");
    let tag_search = tag_search.text().await.expect("body");
    assert!(
        tag_search.contains("Test Fandom (Not Real)"),
        "{tag_search}"
    );
    assert!(
        tag_search.contains("/works/search?include_tags="),
        "{tag_search}"
    );

    let live_form = client
        .get(format!("{root}/works/search"))
        .send()
        .await
        .expect("live form");
    let live_form = live_form.text().await.expect("body");
    assert!(live_form.contains("Work Tags"), "{live_form}");
    assert!(live_form.contains("name=\"include_tags\""), "{live_form}");
    assert!(live_form.contains("method=\"post\""), "{live_form}");

    let dashboard = client
        .get(format!("{root}/dashboard"))
        .send()
        .await
        .expect("dashboard");
    let body = dashboard.text().await.expect("body");
    assert!(body.contains("Cached works:"), "{body}");
    assert!(body.contains(">1<"), "one cached work in stats: {body}");

    for path in ["/subscriptions", "/bookmarks", "/reading"] {
        let response = client
            .get(format!("{root}{path}"))
            .send()
            .await
            .expect("list page");
        assert_eq!(response.status(), 200, "{path}");
    }

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}

#[tokio::test]
async fn skin_toggle_persists_and_required_tags_render() {
    let dir = support::temp_dir("frontend-skin");
    let (root, store, handle) = serve(&dir).await;
    support::harness::seed_work(&store).await;
    let client = http();

    let home = client.get(&root).send().await.expect("home");
    assert!(
        home.text()
            .await
            .expect("body")
            .contains("data-skin=\"default\"")
    );

    let toggled = client
        .post(format!("{root}/skin"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("skin=reversi")
        .send()
        .await
        .expect("skin post");
    assert_eq!(toggled.status(), 200);
    let home = client.get(&root).send().await.expect("home again");
    assert!(
        home.text()
            .await
            .expect("body")
            .contains("data-skin=\"reversi\"")
    );

    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader in reversi");
    let reader = reader.text().await.expect("body");
    assert!(
        reader.contains("&skin=reversi"),
        "iframe carries the skin: {reader}"
    );

    let chapter = client
        .get(format!("{root}/read/12345/chapter/1?skin=reversi"))
        .send()
        .await
        .expect("chapter");
    let chapter = chapter.text().await.expect("body");
    assert!(chapter.contains("data-skin=\"reversi\""), "{chapter}");
    assert!(chapter.contains("color:#eee"), "light text for dark iframe");

    let bad = client
        .post(format!("{root}/skin"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("skin=neon")
        .send()
        .await
        .expect("bad skin");
    assert_eq!(bad.status(), 400);

    let browse = client
        .get(format!("{root}/works"))
        .send()
        .await
        .expect("browse");
    let body = browse.text().await.expect("body");
    assert!(body.contains("class=\"required-tags\""), "{body}");
    assert!(body.contains("rating-e"), "explicit rating badge: {body}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
