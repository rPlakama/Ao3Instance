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
        ("AO3I_FLARESOLVERR", "off"),
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
        "/history",
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
        full.matches("class=\"chapter\" id=\"chapter-").count() >= 2,
        "every cached chapter is stacked inline: {full}"
    );
    assert!(
        !full.contains("<iframe"),
        "entire work renders chapters inline without iframes: {full}"
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
    let chapter_alias_body = chapter_alias.text().await.expect("body");
    assert!(
        chapter_alias_body.contains("id=\"chapter-2\""),
        "{chapter_alias_body}"
    );
    assert!(
        chapter_alias_body.contains("class=\"userstuff module\""),
        "{chapter_alias_body}"
    );

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
    assert!(
        home.contains("class=\"menu\""),
        "AO3 dropdown menus: {home}"
    );
    assert!(home.contains("Anime &amp; Manga"), "{home}");
    assert!(home.contains("fluff sort:kudos"), "search tip: {home}");
    assert!(
        home.contains("action=\"/works/search\""),
        "header search goes to AO3 search: {home}"
    );

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
        tag_search.contains("/tags/Test%20Fandom%20%28Not%20Real%29/works"),
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
        reader.contains("data-skin=\"reversi\""),
        "reader page carries active skin natively for inline chapters: {reader}"
    );

    for skin in ["reversi", "auto", "sepia", "midnight"] {
        let chapter = client
            .get(format!("{root}/read/12345/chapter/1?skin={skin}"))
            .send()
            .await
            .expect("chapter");
        let chapter = chapter.text().await.expect("body");
        assert!(
            chapter.contains(&format!("data-skin=\"{skin}\"")),
            "chapter endpoint preserves skin {skin}: {chapter}"
        );
    }

    let auto = client
        .post(format!("{root}/skin"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("skin=auto")
        .send()
        .await
        .expect("auto skin");
    assert_eq!(auto.status(), 200);
    let home = client.get(&root).send().await.expect("home auto");
    assert!(
        home.text()
            .await
            .expect("body")
            .contains("data-skin=\"auto\"")
    );
    assert_eq!(
        store.get_setting("display.skin").await.expect("setting"),
        Some("auto".to_string())
    );

    let session = client
        .get(format!("{root}/session"))
        .send()
        .await
        .expect("session");
    let session = session.text().await.expect("body");
    assert!(session.contains("id=\"appearance\""), "{session}");
    assert!(session.contains("value=\"auto\" checked"), "{session}");
    assert!(session.contains("Theme settings"), "{session}");

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

#[tokio::test]
async fn ao3_html_structure_matches_otwarchive_regions_and_patterns() {
    let dir = support::temp_dir("frontend-ao3-structure");
    let (root, store, handle) = serve(&dir).await;
    support::harness::seed_work(&store).await;
    let client = http();

    // Home splash: outer/inner wrappers, skiplinks, AO3 splash modules, no dashboard sidebar.
    let home = client
        .get(&root)
        .send()
        .await
        .expect("home")
        .text()
        .await
        .expect("body");
    assert!(home.contains("id=\"outer\" class=\"wrapper\""), "{home}");
    assert!(
        home.contains("/assets/ao3.css?v="),
        "cache-busted stylesheet: {home}"
    );
    assert!(home.contains("id=\"skiplinks\""), "{home}");
    assert!(home.contains("id=\"inner\" class=\"wrapper\""), "{home}");
    assert!(home.contains("class=\"splash\""), "{home}");
    assert!(home.contains("latest news module"), "{home}");
    assert!(home.contains("random readings module"), "{home}");
    assert!(home.contains("latest messages module"), "{home}");
    assert!(home.contains("Customize"), "AO3 footer customize column");
    assert!(
        !home.contains("id=\"dashboard\""),
        "home omits dashboard sidebar"
    );

    // Work reader: AO3 show layout (work navigation actions, meta wrapper, workskin, chapters, feedback).
    let reader = client
        .get(format!("{root}/read/12345"))
        .send()
        .await
        .expect("reader")
        .text()
        .await
        .expect("body");
    assert!(
        reader.contains("class=\"work navigation actions\""),
        "{reader}"
    );
    assert!(reader.contains("class=\"work meta group\""), "{reader}");
    assert!(reader.contains("id=\"workskin\""), "{reader}");
    assert!(reader.contains("id=\"chapters\""), "{reader}");
    assert!(
        reader.contains("id=\"feedback\" class=\"feedback\""),
        "{reader}"
    );
    // AO3 kudos module and commentable landmarks (design parity).
    assert!(reader.contains("id=\"kudos\""), "{reader}");
    assert!(
        reader.contains("<a id=\"comments\">Comments</a>"),
        "{reader}"
    );
    assert!(
        reader.contains("1,024 people left kudos on this work!"),
        "{reader}"
    );

    // Fandoms directory: alphabet jump bar + AO3 letter listbox panels.
    let fandoms = client
        .get(format!("{root}/fandoms"))
        .send()
        .await
        .expect("fandoms")
        .text()
        .await
        .expect("body");
    assert!(fandoms.contains("class=\"alphabet actions\""), "{fandoms}");
    assert!(fandoms.contains("letter listbox group"), "{fandoms}");

    // Dashboard: shows AO3 #dashboard.region.own sidebar + .user.home + .listbox.group.
    let dashboard = client
        .get(format!("{root}/dashboard"))
        .send()
        .await
        .expect("dashboard")
        .text()
        .await
        .expect("body");
    assert!(
        dashboard.contains("id=\"dashboard\" class=\"region own\""),
        "{dashboard}"
    );
    assert!(dashboard.contains("class=\"user home\""), "{dashboard}");
    assert!(dashboard.contains("class=\"listbox group\""), "{dashboard}");

    // Search form: AO3 verbose search fieldsets + dl/dt/dd rows.
    let search = client
        .get(format!("{root}/works/search"))
        .send()
        .await
        .expect("search form")
        .text()
        .await
        .expect("body");
    assert!(search.contains("verbose search work-search"), "{search}");
    assert!(search.contains("<legend>Work Info</legend>"), "{search}");
    assert!(search.contains("<dl><dt>"), "{search}");

    handle.abort();
    drop(std::fs::remove_dir_all(dir));
}
