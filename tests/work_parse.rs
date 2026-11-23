#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test target: panics are the assertion mechanism"
)]

//! Work-page parser tests: synthetic fixtures derived from the pinned otwarchive
//! clone (`452391b`) plus malformed pages that must fail visibly.

use ao3instance::ao3::work::{WorkPage, parse_work_page};
use ao3instance::error::Error;

const FULL_WORK: &str = include_str!("fixtures/work_full.html");
const SINGLE_CHAPTER: &str = include_str!("fixtures/work_single.html");

fn parse(html: &str) -> WorkPage {
    parse_work_page(html).expect("work page should parse")
}

#[test]
fn full_work_page_reads_identity_tags_and_stats() {
    let page = parse(FULL_WORK);

    assert_eq!(page.ao3_id, "12345");
    assert_eq!(page.title, "PLACEHOLDER: The Map and the Mirror");
    assert_eq!(page.authors, ["test_author_one", "second_pseud"]);

    let summary = page.summary.as_deref().expect("summary");
    assert!(summary.contains("PLACEHOLDER SUMMARY"), "{summary}");
    assert!(
        summary.contains("<p>"),
        "summary keeps userstuff markup: {summary}"
    );

    assert_eq!(page.fandoms, ["Original Work", "Test Fandom (Not Real)"]);
    assert_eq!(
        page.relationships,
        ["Character A/Character B", "Character A & Character B"]
    );
    assert_eq!(page.characters, ["Character A", "Character B"]);
    assert_eq!(
        page.additional_tags,
        [
            "Alternate Universe - Placeholder",
            "POV Alternating",
            "Fluff and Angst"
        ]
    );
    assert_eq!(
        page.warnings,
        ["Graphic Depictions Of Violence", "Major Character Death"]
    );
    assert_eq!(page.categories, ["F/M", "Gen"]);
    assert_eq!(page.rating.as_deref(), Some("Explicit"));
    assert_eq!(page.language.as_deref(), Some("English"));
    assert_eq!(page.published_at.as_deref(), Some("2025-01-02"));
    assert_eq!(page.complete, Some(true));
    assert_eq!(page.word_count, Some(4_321));
    assert_eq!(page.kudos, Some(1_024));
    assert_eq!(page.comment_count, Some(57));
    assert_eq!(page.hits, Some(9_876));
    assert_eq!(page.bookmarks, Some(12));
}

#[test]
fn full_work_page_reads_chapter_structure_and_comment_form() {
    let page = parse(FULL_WORK);

    assert_eq!(page.chapter_count, 2);
    assert_eq!(page.chapters.len(), 2);

    let first = &page.chapters[0];
    assert_eq!(first.index, 1);
    assert_eq!(
        first.title.as_deref(),
        Some("The Cartographer's Apprentice")
    );
    assert!(
        first.body_html.contains("PLACEHOLDER PROSE (chapter one)"),
        "{}",
        first.body_html
    );
    assert!(
        first.body_html.contains("<em>"),
        "creator markup survives: {}",
        first.body_html
    );
    assert!(
        !first.body_html.contains("Chapter Text"),
        "landmark heading is wrapper chrome: {}",
        first.body_html
    );
    assert!(
        !first.body_html.contains("CHAPTER ONE NOTE"),
        "notes are not part of the body: {}",
        first.body_html
    );
    let first_notes = first.notes_html.as_deref().expect("first chapter notes");
    assert!(
        first_notes.contains("PLACEHOLDER CHAPTER ONE NOTE"),
        "{first_notes}"
    );

    let second = &page.chapters[1];
    assert_eq!(second.index, 2);
    assert_eq!(second.title.as_deref(), Some("A Map of Small Things"));
    assert!(
        second.body_html.contains("PLACEHOLDER PROSE (chapter two)"),
        "{}",
        second.body_html
    );
    assert!(
        !second.body_html.contains("ENDNOTE"),
        "endnotes are not part of the body: {}",
        second.body_html
    );
    let second_notes = second.notes_html.as_deref().expect("second chapter notes");
    assert!(
        second_notes.contains("PLACEHOLDER CHAPTER TWO NOTE"),
        "{second_notes}"
    );
    assert!(
        !second_notes.contains("ENDNOTE"),
        "top-of-chapter notes win over endnotes: {second_notes}"
    );

    assert!(page.comment_form_present);
}

#[test]
fn single_chapter_page_leaves_optional_fields_empty() {
    let page = parse(SINGLE_CHAPTER);

    assert_eq!(page.ao3_id, "654321");
    assert_eq!(page.title, "PLACEHOLDER SINGLE TITLE");
    assert_eq!(page.authors, ["single_author"]);

    assert_eq!(page.summary, None);
    assert_eq!(page.fandoms, ["Original Work"]);
    assert_eq!(page.rating.as_deref(), Some("Not Rated"));
    assert_eq!(page.warnings, ["No Archive Warnings Apply"]);
    assert_eq!(page.categories, ["Gen"]);
    assert_eq!(page.characters, ["Character C"]);
    assert!(page.relationships.is_empty());
    assert!(page.additional_tags.is_empty());
    assert_eq!(page.language.as_deref(), Some("English"));
    assert_eq!(page.word_count, Some(812));
    // AO3 omits zero counts, so an absent stat is `None`, not `Some(0)`.
    assert_eq!(page.kudos, None);
    assert_eq!(page.comment_count, None);

    assert_eq!(page.chapter_count, 1);
    assert_eq!(page.chapters.len(), 1);
    let chapter = &page.chapters[0];
    assert_eq!(chapter.index, 1);
    assert_eq!(chapter.title, None);
    assert!(
        chapter
            .body_html
            .contains("PLACEHOLDER PROSE (single chapter)"),
        "{}",
        chapter.body_html
    );
    assert!(
        !chapter.body_html.contains("Work Text:"),
        "landmark heading is wrapper chrome: {}",
        chapter.body_html
    );
    assert_eq!(chapter.notes_html, None);
    assert!(!page.comment_form_present);
}

#[test]
fn missing_title_is_visible_upstream_drift() {
    for html in [
        "<html></html>",
        "",
        "<html><body><p>not a work page</p></body></html>",
    ] {
        match parse_work_page(html) {
            Err(Error::UpstreamChanged(message)) => assert_eq!(message, "work title missing"),
            other => panic!("expected UpstreamChanged, got {other:?}"),
        }
    }
}

#[test]
fn title_without_chapter_container_yields_no_chapters() {
    // A real title but no `#chapters` container: the parser must say "no chapter
    // content" by returning an empty list, not invent a chapter and not error.
    let html = r#"<html><body>
        <div class="preface group">
          <h2 class="title heading">A Title With No Body</h2>
          <h3 class="byline heading"><a href="/users/nobody/pseuds/nobody" rel="author">nobody</a></h3>
        </div>
      </body></html>"#;

    let page = parse(html);
    assert_eq!(page.title, "A Title With No Body");
    assert_eq!(page.authors, ["nobody"]);
    assert!(page.chapters.is_empty());
    assert_eq!(page.chapter_count, 0);
    assert!(!page.comment_form_present);
}
