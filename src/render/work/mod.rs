//! Work and reader page modelled directly on `otwarchive/app/views/works/show.html.erb`,
//! `_work_header_navigation.html.erb`, `_meta.html.erb`, `_preface.html.erb`,
//! `chapters/_chapter.html.erb`, and `comments/_commentable.html.erb`.
//!
//! Author HTML always passes through [`sanitize`] (`ammonia` allowlist,
//! `http`/`https` schemes only, forced `noopener noreferrer`) before rendering
//! inline inside `#chapters`, matching AO3's own `sanitize_field` model so
//! every built-in and imported public AO3 site skin styles prose seamlessly.

mod chapter;
mod comments;
mod meta;
mod nav;

use crate::ao3::comments::Comment;
use crate::render::blurb::author_links;
use crate::render::chrome::{PageContext, layout_with_class};
use crate::render::components::flash_notice;
use crate::render::{escape, sanitize};
use crate::store::{CommentSetInfo, StoredChapter, StoredWork};

pub use chapter::chapter_document;

/// Everything the reader needs that is not the work itself.
pub struct ReadContext<'a> {
    pub current: i64,
    pub progress: Option<crate::store::Progress>,
    /// Exact scroll percentage (0.0..=100.0) to restore inside the current chapter.
    pub resume_pct: Option<f64>,
    pub notice: Option<&'a str>,
    pub subscribed: bool,
    pub can_write: bool,
    pub kudos_sent: bool,
    pub bookmarked: bool,
    pub reading_state: Option<&'a str>,
    /// `?view_full_work=true`: render every cached chapter stacked.
    pub full_work: bool,
    /// Pseud name for the comment box, once this session discovered one.
    pub comment_as: Option<&'a str>,
    /// Cached AO3 comment thread for this reader position (`show_comments`).
    pub comments: &'a [Comment],
    /// Whether this commentable was ever fetched, and when.
    pub comment_set: Option<&'a CommentSetInfo>,
    /// `?show_comments=1`: render the cached thread instead of the action link.
    pub show_comments: bool,
    /// Fixed notice belonging to the comments section (fetch outcome).
    pub comment_notice: Option<&'a str>,
    /// AO3 page that carries this commentable's thread (for the "more comments"
    /// link when the cached copy is only the first page).
    pub comment_page_path: Option<&'a str>,
}

pub fn read_page(
    page: &PageContext,
    work: &StoredWork,
    chapters: &[StoredChapter],
    ctx: &ReadContext<'_>,
) -> String {
    let id = escape(&work.ao3_id);
    let authors = author_links(work);
    let chapter = chapters.iter().find(|chapter| chapter.idx == ctx.current);
    let total = chapters.len();
    let position = chapters
        .iter()
        .position(|chapter| chapter.idx == ctx.current)
        .unwrap_or(0);
    let chapter_total = i64::try_from(work.chapter_count)
        .unwrap_or(i64::MAX)
        .max(i64::try_from(total).unwrap_or(i64::MAX));
    let inside_pct = ctx
        .progress
        .filter(|saved| saved.chapter_idx == ctx.current)
        .map_or(0, |saved| saved.pct);
    let overall = crate::render::overall_percent(
        i64::try_from(position).unwrap_or(0) + 1,
        inside_pct,
        chapter_total,
    );

    let mut body = String::from(
        "<ul class=\"landmark skip\">\
         <li><a id=\"top\">&nbsp;</a></li>\
         <li><a href=\"#workskin\">Skip header</a></li></ul>",
    );
    body.push_str(&flash_notice(ctx.notice));
    if let Some(saved) = ctx.progress
        && saved.chapter_idx != ctx.current
        && !ctx.full_work
    {
        body.push_str(&format!(
            "<div class=\"continue\">You were at chapter {chapter}. \
             <a href=\"/read/{id}?chapter={chapter}\">Resume</a></div>",
            chapter = saved.chapter_idx
        ));
    }

    body.push_str("<div class=\"work\">");
    body.push_str(&nav::top_navigation(
        &id, chapters, position, inside_pct, ctx,
    ));
    body.push_str(&meta::work_meta_box(
        &id,
        work,
        ctx.current,
        chapter_total,
        overall,
    ));

    // Work skin + preface + inline chapters (otwarchive/app/views/works/show.html.erb)
    body.push_str("<div id=\"work-skin\" class=\"wrapper\"><div id=\"workskin\">");
    body.push_str(&format!(
        "<div class=\"preface group\">\
         <h2 class=\"title heading\">{}</h2>\
         <h3 class=\"byline heading\">{}</h3>",
        escape(&work.title),
        authors
    ));
    if (ctx.full_work || position == 0)
        && let Some(summary) = &work.summary
        && !summary.trim().is_empty()
    {
        body.push_str(&format!(
            "<div class=\"summary module\"><h3 class=\"heading\">Summary:</h3>\
             <blockquote class=\"userstuff\">{}</blockquote></div>",
            sanitize(summary)
        ));
    }
    body.push_str("</div>"); // .preface.group

    body.push_str("<div id=\"chapters\" role=\"article\">");
    if ctx.full_work && !chapters.is_empty() {
        for (index, candidate) in chapters.iter().enumerate() {
            let resume = ctx.resume_pct.filter(|_| candidate.idx == ctx.current);
            body.push_str(&chapter::render_chapter_inline(
                &id,
                candidate,
                index + 1,
                total,
                chapter_total,
                resume,
                index == 0,
            ));
        }
    } else {
        match chapter {
            Some(candidate) => {
                let resume = ctx.resume_pct.filter(|_| candidate.idx == ctx.current);
                body.push_str(&chapter::render_chapter_inline(
                    &id,
                    candidate,
                    position + 1,
                    total,
                    chapter_total,
                    resume,
                    true,
                ));
            }
            None => body.push_str(
                "<p class=\"muted\">No chapter content was stored for this work. \
                 Re-open it to refresh (AO3's chapter markup may have changed).</p>",
            ),
        }
    }
    body.push_str("</div>"); // #chapters
    body.push_str("</div></div>"); // #workskin, #work-skin
    body.push_str("</div>"); // .work

    body.push_str(&nav::feedback_section(work, chapters, position, ctx));
    body.push_str(&format!(
        "<script src=\"/assets/reader.js?v={:016x}\" defer></script>",
        crate::render::READER_JS_HASH
    ));
    layout_with_class(page, "works-show", &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::NavSection;
    use crate::store::TagGroups;

    fn sample_work() -> StoredWork {
        StoredWork {
            ao3_id: "42".into(),
            title: "A Test Work".into(),
            authors: vec!["Author".into()],
            summary: Some("<p>Summary text.</p>".into()),
            tags: TagGroups {
                fandoms: vec!["Fandom A".into(), "Fandom B".into()],
                relationships: vec!["A/B".into()],
                characters: vec!["A".into()],
                additional: vec!["Fluff".into()],
                warnings: vec!["No Archive Warnings Apply".into()],
                categories: vec!["Gen".into()],
            },
            rating: Some("General Audiences".into()),
            language: Some("English".into()),
            published_at: Some("2025-01-15".into()),
            complete: Some(true),
            word_count: Some(1500),
            kudos: Some(12),
            comment_count: Some(3),
            hits: Some(100),
            bookmarks: Some(4),
            chapter_count: 1,
            content_hash: "h".into(),
            fetched_at: 1_700_000_000,
        }
    }

    #[test]
    fn read_page_renders_sanitized_chapter_inline_with_ao3_structure() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello <em>world</em></p><script>bad()</script>".into(),
            notes_html: Some("<p>Author note.</p>".into()),
            ao3_chapter_id: Some("78901".into()),
        }];
        let html = read_page(
            &page,
            &work,
            &chapters,
            &ReadContext {
                current: 1,
                progress: Some(crate::store::Progress {
                    chapter_idx: 1,
                    pct: 42,
                }),
                resume_pct: Some(42.75),
                notice: None,
                subscribed: false,
                can_write: false,
                kudos_sent: false,
                bookmarked: false,
                reading_state: None,
                full_work: false,
                comment_as: None,
                comments: &[],
                comment_set: None,
                show_comments: false,
                comment_notice: None,
                comment_page_path: None,
            },
        );
        assert!(!html.contains("<iframe"), "chapters render inline: {html}");
        assert!(html.contains("<p>Hello <em>world</em></p>"), "{html}");
        assert!(!html.contains("bad()"), "author scripts stripped: {html}");
        assert!(html.contains("data-resume=\"43\""), "{html}");
        assert!(html.contains("data-resume-exact=\"42.75\""), "{html}");
        assert!(html.contains("data-resume-target=\"1\""), "{html}");
        assert!(
            html.contains("name=\"pct\" value=\"42.75\""),
            "save-place preserves exact pct: {html}"
        );
        assert!(html.contains("Fandoms:"), "pluralised tag label: {html}");
        assert!(html.contains("Published:"), "published stat: {html}");
        assert!(html.contains("2025-01-15"), "{html}");
        assert!(html.contains("42% read"), "{html}");
        assert!(
            !html.contains("Entire Work"),
            "single-chapter work hides multi-chapter nav: {html}"
        );
        assert!(
            !html.contains("Chapter 1</a>"),
            "untitled single-chapter work omits redundant Chapter 1 header: {html}"
        );
    }

    #[test]
    fn full_work_marks_later_saved_chapter_as_resume_target_even_at_zero_percent() {
        let page = PageContext::new("Test", NavSection::Home);
        let mut work = sample_work();
        work.chapter_count = 2;
        let chapters = vec![
            StoredChapter {
                idx: 1,
                title: Some("One".into()),
                body_html: "<p>Chapter one.</p>".into(),
                notes_html: None,
                ao3_chapter_id: Some("78901".into()),
            },
            StoredChapter {
                idx: 2,
                title: Some("Two".into()),
                body_html: "<p>Chapter two.</p>".into(),
                notes_html: None,
                ao3_chapter_id: Some("78902".into()),
            },
        ];
        let html = read_page(
            &page,
            &work,
            &chapters,
            &ReadContext {
                current: 2,
                progress: Some(crate::store::Progress {
                    chapter_idx: 2,
                    pct: 0,
                }),
                resume_pct: Some(0.0),
                notice: None,
                subscribed: false,
                can_write: false,
                kudos_sent: false,
                bookmarked: false,
                reading_state: None,
                full_work: true,
                comment_as: None,
                comments: &[],
                comment_set: None,
                show_comments: false,
                comment_notice: None,
                comment_page_path: None,
            },
        );
        assert!(
            html.contains(
                "id=\"chapter-1\" data-work-id=\"42\" data-chapter=\"1\" data-total-chapters=\"2\" data-resume=\"0\" data-resume-exact=\"0.00\">"
            ),
            "chapter 1 is not the resume target: {html}"
        );
        assert!(
            html.contains(
                "id=\"chapter-2\" data-work-id=\"42\" data-chapter=\"2\" data-total-chapters=\"2\" data-resume=\"0\" data-resume-exact=\"0.00\" data-resume-target=\"1\">"
            ),
            "chapter 2 is marked as the resume target: {html}"
        );
    }

    #[test]
    fn chapter_document_includes_sanitized_body_notes_and_all_skins() {
        let chapter = StoredChapter {
            idx: 1,
            title: Some("Chapter <One>".into()),
            body_html: "<p>body</p><script>bad()</script>".into(),
            notes_html: Some("<p>note</p>".into()),
            ao3_chapter_id: Some("78901".into()),
        };
        let doc = chapter_document("Work <One>", &chapter, "default");
        assert!(doc.contains("Work &lt;One&gt;"));
        assert!(!doc.contains("<h3 class=\"chapter-title\""));
        assert!(doc.contains("<p>body</p>"));
        assert!(!doc.contains("bad()"));
        assert!(doc.contains("Chapter notes"));
        assert!(doc.contains("data-skin=\"default\""));

        for skin in [
            "auto", "reversi", "snow", "frost", "cloud", "sepia", "midnight",
        ] {
            let themed = chapter_document("w", &chapter, skin);
            assert!(
                themed.contains(&format!("data-skin=\"{skin}\"")),
                "skin {skin}: {themed}"
            );
        }
        assert_eq!(
            chapter_document("w", &chapter, "neon"),
            chapter_document("w", &chapter, "default"),
            "unknown skins fall back to the light document"
        );
    }

    fn ctx_with<'a>(comments: &'a [Comment]) -> ReadContext<'a> {
        ReadContext {
            current: 1,
            progress: None,
            resume_pct: None,
            notice: None,
            subscribed: false,
            can_write: true,
            kudos_sent: false,
            bookmarked: false,
            reading_state: None,
            full_work: false,
            comment_as: Some("Main Pseud"),
            comments,
            comment_set: None,
            show_comments: false,
            comment_notice: None,
            comment_page_path: None,
        }
    }

    #[test]
    fn read_page_renders_ao3_kudos_and_direct_post_forms() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello</p>".into(),
            notes_html: None,
            ao3_chapter_id: None,
        }];
        let html = read_page(&page, &work, &chapters, &ctx_with(&[]));

        // AO3's kudos action is a direct POST form carrying the confirmation
        // token; there is no preview hop from the work page.
        assert!(html.contains("action=\"/write/kudos\""), "{html}");
        assert!(
            html.contains("id=\"kudo_submit\">Kudos ♥</button>"),
            "{html}"
        );
        assert!(
            html.contains("<input type=\"hidden\" name=\"confirm\" value=\"yes\">"),
            "{html}"
        );
        // Subscription and unsubscribe are direct POST forms too.
        assert!(html.contains("action=\"/write/subscribe\""), "{html}");
        assert!(html.contains(">Subscribe</button>"), "{html}");
        assert!(!html.contains("/write/preview?kind=kudos"), "{html}");
        assert!(!html.contains("/write/preview?kind=work"), "{html}");
        // AO3's kudos receipt module.
        assert!(
            html.contains("12 people left kudos on this work!"),
            "{html}"
        );
        // AO3's comment form (not a collapsible details box anymore).
        assert!(html.contains("action=\"/write/comment/preview\""), "{html}");
        assert!(
            html.contains("Comment as <span class=\"byline\">Main Pseud</span>"),
            "{html}"
        );
        assert!(html.contains("Preview Comment</button>"), "{html}");
        assert!(!html.contains("<details class=\"comment\">"), "{html}");
        // Uncached comments offer the one-click fetch form.
        assert!(html.contains("action=\"/read/42/comments\""), "{html}");
        assert!(
            html.contains("Comments for this work are not cached yet."),
            "{html}"
        );
    }

    #[test]
    fn read_page_renders_cached_comment_thread_sanitized_and_nested() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello</p>".into(),
            notes_html: None,
            ao3_chapter_id: None,
        }];
        let comments = vec![
            Comment {
                seq: 0,
                comment_id: "9001".into(),
                parent_id: None,
                depth: 0,
                author: Some("reader_one".into()),
                author_login: Some("reader_one".into()),
                posted_at: Some("01 Nov 2026 10:00 UTC".into()),
                edited_at: None,
                body_html: "<p>Nice <em>work</em>.</p><script>bad()</script>".into(),
            },
            Comment {
                seq: 1,
                comment_id: "9002".into(),
                parent_id: Some("9001".into()),
                depth: 1,
                author: Some("guest_name".into()),
                author_login: None,
                posted_at: Some("02 Nov 2026 09:00 UTC".into()),
                edited_at: Some("Last Edited 03 Nov 2026 08:00 UTC".into()),
                body_html: "<p>A reply with <a href=\"javascript:bad()\">link</a>.</p>".into(),
            },
        ];
        let mut ctx = ctx_with(&comments);
        ctx.show_comments = true;
        ctx.comment_set = Some(&CommentSetInfo {
            total: Some(2),
            fetched_at: 1_700_000_000,
            has_more: true,
        });
        ctx.comment_page_path = Some("/works/12345/chapters/78901?show_comments=true");
        let html = read_page(&page, &work, &chapters, &ctx);

        assert!(html.contains("<ol class=\"thread\">"), "{html}");
        assert!(html.contains("id=\"comment_9001\""), "{html}");
        assert!(html.contains("id=\"comment_9002\""), "{html}");
        assert!(
            html.contains("href=\"/works?author=reader_one\""),
            "registered pseuds link: {html}"
        );
        assert!(
            !html.contains(">guest_name</a>"),
            "guest names stay plain text: {html}"
        );
        assert!(
            html.contains("Nice <em>work</em>."),
            "creator markup survives: {html}"
        );
        assert!(!html.contains("bad()"), "scripts are stripped: {html}");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("Last Edited 03 Nov 2026 08:00 UTC"), "{html}");
        assert!(html.contains("Hide Comments"), "{html}");
        assert!(html.contains("2 cached comments"), "{html}");
        assert!(
            html.contains(">Refresh comments</button>"),
            "cached threads offer an explicit refresh control: {html}"
        );
        assert!(
            html.contains(
                "https://archiveofourown.org/works/12345/chapters/78901?show_comments=true&page=2"
            ),
            "paginated thread links to AO3: {html}"
        );
    }

    #[test]
    fn read_page_offers_refresh_for_a_cached_empty_thread() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello</p>".into(),
            notes_html: None,
            ao3_chapter_id: None,
        }];
        let mut ctx = ctx_with(&[]);
        ctx.show_comments = true;
        ctx.comment_set = Some(&CommentSetInfo {
            total: Some(0),
            fetched_at: 1_700_000_000,
            has_more: false,
        });
        let html = read_page(&page, &work, &chapters, &ctx);

        assert!(
            html.contains("No comments on AO3 for this work yet."),
            "{html}"
        );
        assert!(
            html.contains(">Refresh comments</button>"),
            "a cached zero-comment thread stays refreshable: {html}"
        );
    }

    #[test]
    fn read_page_shows_kudos_receipt_and_unsubscribe_when_given() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello</p>".into(),
            notes_html: None,
            ao3_chapter_id: None,
        }];
        let mut ctx = ctx_with(&[]);
        ctx.kudos_sent = true;
        ctx.subscribed = true;
        let html = read_page(&page, &work, &chapters, &ctx);

        assert!(!html.contains("action=\"/write/kudos\""), "{html}");
        assert!(html.contains("kudos given"), "{html}");
        assert!(
            html.contains("You and 11 others left kudos on this work!"),
            "{html}"
        );
        assert!(html.contains("action=\"/write/unsubscribe\""), "{html}");
        assert!(html.contains(">Unsubscribe</button>"), "{html}");
    }

    #[test]
    fn read_page_explains_signed_out_commenting() {
        let page = PageContext::new("Test", NavSection::Home);
        let work = sample_work();
        let chapters = vec![StoredChapter {
            idx: 1,
            title: None,
            body_html: "<p>Hello</p>".into(),
            notes_html: None,
            ao3_chapter_id: None,
        }];
        let mut ctx = ctx_with(&[]);
        ctx.can_write = false;
        ctx.comment_as = None;
        let html = read_page(&page, &work, &chapters, &ctx);

        assert!(
            html.contains(
                "Sign in to AO3 on the <a href=\"/session\">Session page</a> to comment."
            ),
            "{html}"
        );
        assert!(
            !html.contains("action=\"/write/comment/preview\""),
            "{html}"
        );
        assert!(!html.contains("action=\"/write/kudos\""), "{html}");
    }
}
