//! Work and reader page modelled directly on `otwarchive/app/views/works/show.html.erb`,
//! `_work_header_navigation.html.erb`, `_meta.html.erb`, `_preface.html.erb`,
//! `chapters/_chapter.html.erb`, and `comments/_commentable.html.erb`.
//!
//! Author HTML always passes through [`sanitize`] (`ammonia` allowlist,
//! `http`/`https` schemes only, forced `noopener noreferrer`) before rendering
//! inline inside `#chapters`, matching AO3's own `sanitize_field` model so
//! every built-in and imported public AO3 site skin styles prose seamlessly.

mod chapter;
mod meta;
mod nav;

use crate::render::blurb::author_links;
use crate::render::chrome::{PageContext, layout_with_class};
use crate::render::components::flash_notice;
use crate::render::{escape, sanitize};
use crate::store::{StoredChapter, StoredWork};

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

    body.push_str(&nav::feedback_section(&id, chapters, position, ctx));
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
            },
            StoredChapter {
                idx: 2,
                title: Some("Two".into()),
                body_html: "<p>Chapter two.</p>".into(),
                notes_html: None,
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
}
