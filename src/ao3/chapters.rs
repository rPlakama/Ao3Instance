//! Parsers for AO3's chapter manager surfaces and chapter validation.
//!
//! Selector provenance (pinned otwarchive clone, `452391b`):
//! | surface | evidence |
//! | --- | --- |
//! | chapter index | `app/views/works/navigate.html.erb:1-7` — `ol.chapter.index.group > li > a` (chapter path) + `span.datetime` |
//! | work title | `navigate.html.erb:1` — `h2.heading` first `/works/<id>` link |
//! | chapter editor | `app/views/chapters/_chapter_form.html.erb` — `chapter[title]`, `chapter[position]`, `chapter[wip_length]`, `chapter[summary]`, `chapter[notes]`, `chapter[endnotes]`, `chapter[content]` (`#content`) |
//!
//! Contract: a page without the expected marker is not the chapter surface
//! (login wall, permission problem, layout drift) and yields
//! [`Error::UpstreamChanged`]; callers show an honest error instead of an editor
//! that cannot save.

use std::sync::OnceLock;

use scraper::{ElementRef, Html, Selector};

use crate::ao3::compose::{CONTENT_MAX, CONTENT_MIN, NOTES_MAX, SUMMARY_MAX, TITLE_MAX};
use crate::ao3::html::{collapse_text, parse_selector as css};
use crate::error::{Error, Result};

/// One chapter in AO3's chapter index (`/works/:id/navigate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterEntry {
    pub chapter_id: String,
    pub title: String,
    pub posted_at: Option<String>,
}

/// The parsed chapter index, including the work title for editor headings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChapterIndex {
    pub work_title: String,
    pub chapters: Vec<ChapterEntry>,
}

/// The editable fields of one chapter, as AO3 renders them in the editor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChapterFields {
    pub title: String,
    pub position: String,
    /// Expected total chapter count (`chapter[wip_length]`, "of N").
    pub wip_length: String,
    pub summary: String,
    pub notes: String,
    pub endnotes: String,
    pub content: String,
}

/// Parses `/works/:id/navigate` (chapter index).
pub fn parse_chapter_index(html: &str) -> Result<ChapterIndex> {
    let document = Html::parse_document(html);
    let selectors = selectors()?;
    let list = document
        .select(&selectors.chapter_index)
        .next()
        .ok_or_else(|| {
            Error::UpstreamChanged(
                "AO3 did not show a chapter index (not the work's owner, or the page changed)"
                    .to_string(),
            )
        })?;
    let work_title = document
        .select(&selectors.work_title_link)
        .next()
        .map(|link| collapse_text(&link))
        .unwrap_or_default();
    let mut chapters = Vec::new();
    for item in list.select(&selectors.index_items) {
        let Some(link) = item.select(&selectors.chapter_link).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let Some(chapter_id) = chapter_id_from_path(href) else {
            continue;
        };
        let posted_at = item
            .select(&selectors.datetime)
            .next()
            .map(|node| collapse_text(&node))
            .map(|text| text.trim_matches(['(', ')']).to_string())
            .filter(|text| !text.is_empty());
        let raw_title = collapse_text(&link);
        chapters.push(ChapterEntry {
            chapter_id,
            title: strip_position_prefix(&raw_title),
            posted_at,
        });
    }
    Ok(ChapterIndex {
        work_title,
        chapters,
    })
}

/// Parses `GET /works/:w/chapters/:id/edit` into editable fields.
pub fn parse_chapter_editor(html: &str) -> Result<ChapterFields> {
    let document = Html::parse_document(html);
    let selectors = selectors()?;
    let form = document
        .select(&selectors.chapter_form)
        .next()
        .ok_or_else(|| {
            Error::UpstreamChanged(
                "AO3 did not show a chapter form (not the chapter's author, or the page changed)"
                    .to_string(),
            )
        })?;
    Ok(ChapterFields {
        title: input_value(form, &selectors.title),
        position: input_value(form, &selectors.position),
        wip_length: input_value(form, &selectors.wip_length),
        summary: textarea_value(form, &selectors.summary),
        notes: textarea_value(form, &selectors.notes),
        endnotes: textarea_value(form, &selectors.endnotes),
        content: textarea_value(form, &selectors.content),
    })
}

/// Local validation mirroring `app/models/chapter.rb:14-22` and the work-level
/// tag rules. Kept deliberately tolerant: AO3 remains the final judge.
pub fn validate_chapter(fields: &ChapterFields) -> Vec<String> {
    let mut errors = Vec::new();
    if fields.title.chars().count() > TITLE_MAX {
        errors.push(format!(
            "Chapter Title must be at most {TITLE_MAX} characters."
        ));
    }
    if fields.summary.chars().count() > SUMMARY_MAX {
        errors.push(format!(
            "Chapter Summary must be at most {SUMMARY_MAX} characters."
        ));
    }
    if fields.notes.chars().count() > NOTES_MAX {
        errors.push(format!(
            "Chapter Notes must be at most {NOTES_MAX} characters."
        ));
    }
    if fields.endnotes.chars().count() > NOTES_MAX {
        errors.push(format!("End Notes must be at most {NOTES_MAX} characters."));
    }
    let content_len = fields.content.trim().chars().count();
    if content_len < CONTENT_MIN {
        errors.push(format!(
            "Chapter Text must be at least {CONTENT_MIN} characters long."
        ));
    } else if fields.content.chars().count() > CONTENT_MAX {
        errors.push(format!(
            "Chapter Text must be at most {CONTENT_MAX} characters long."
        ));
    }
    if !fields.position.trim().is_empty() && fields.position.trim().parse::<i64>().is_err() {
        errors.push("Chapter Number must be a number.".to_string());
    }
    // AO3 renders "?" for "total unknown" and accepts it on save.
    let wip = fields.wip_length.trim();
    if !wip.is_empty() && wip != "?" && wip.parse::<i64>().is_err() {
        errors.push("Total chapters must be a number or \"?\".".to_string());
    }
    errors
}

/// `chapter.display_title` is rendered as `"<position>. <title>"`; strip the
/// numeric prefix so the manager lists the chapter title itself.
fn strip_position_prefix(title: &str) -> String {
    let trimmed = title.trim_start();
    let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return title.trim().to_string();
    }
    let rest = trimmed[digits.len()..].trim_start();
    let Some(rest) = rest.strip_prefix('.') else {
        return title.trim().to_string();
    };
    rest.trim().to_string()
}

pub(crate) fn chapter_id_from_path(href: &str) -> Option<String> {
    let rest = href
        .split('/')
        .skip_while(|segment| *segment != "chapters")
        .nth(1)?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

fn input_value(form: ElementRef<'_>, selector: &Selector) -> String {
    form.select(selector)
        .next()
        .and_then(|node| node.value().attr("value"))
        .unwrap_or_default()
        .to_string()
}

fn textarea_value(form: ElementRef<'_>, selector: &Selector) -> String {
    // Textarea contents are author HTML stored as text: return the decoded text
    // (not serialized inner HTML), or the editor would show `&lt;p&gt;` and post
    // double-escaped markup back to AO3.
    form.select(selector)
        .next()
        .map(|node| node.text().collect::<String>())
        .unwrap_or_default()
}

struct Selectors {
    chapter_index: Selector,
    index_items: Selector,
    chapter_link: Selector,
    datetime: Selector,
    work_title_link: Selector,
    chapter_form: Selector,
    title: Selector,
    position: Selector,
    wip_length: Selector,
    summary: Selector,
    notes: Selector,
    endnotes: Selector,
    content: Selector,
}

fn selectors() -> Result<&'static Selectors> {
    static CACHE: OnceLock<std::result::Result<Selectors, String>> = OnceLock::new();
    let compiled = CACHE.get_or_init(|| build_selectors().map_err(|err| err.to_string()));
    compiled
        .as_ref()
        .map_err(|msg| Error::UpstreamChanged(msg.clone()))
}

fn build_selectors() -> Result<Selectors> {
    Ok(Selectors {
        chapter_index: css("ol.chapter.index.group")?,
        index_items: css("li")?,
        chapter_link: css("a[href*='/chapters/']")?,
        datetime: css("span.datetime")?,
        work_title_link: css("h2.heading a[href*='/works/']")?,
        chapter_form: css("#chapter-form form, form[id^='edit_chapter_'], form.new_chapter")?,
        title: css("input[name='chapter[title]']")?,
        position: css("input[name='chapter[position]']")?,
        wip_length: css("input[name='chapter[wip_length]']")?,
        summary: css("textarea[name='chapter[summary]']")?,
        notes: css("textarea[name='chapter[notes]']")?,
        endnotes: css("textarea[name='chapter[endnotes]']")?,
        content: css("textarea[name='chapter[content]']")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAVIGATE: &str = include_str!("../../tests/fixtures/ao3_navigate.html");
    const EDIT: &str = include_str!("../../tests/fixtures/ao3_chapter_edit.html");

    #[test]
    fn parses_the_chapter_index_with_ids_and_dates() {
        let index = parse_chapter_index(NAVIGATE).expect("chapter index");
        assert_eq!(index.work_title, "PLACEHOLDER: The Map and the Mirror");
        assert_eq!(index.chapters.len(), 2);
        assert_eq!(index.chapters[0].chapter_id, "78901");
        assert_eq!(
            index.chapters[0].title, "Chapter 1: The Cartographer's Apprentice",
            "the display_title numeric prefix is stripped"
        );
        assert_eq!(index.chapters[0].posted_at.as_deref(), Some("2025-01-02"));
        assert_eq!(index.chapters[1].chapter_id, "78902");
    }

    #[test]
    fn parses_the_editor_fields_for_prefilling() {
        let fields = parse_chapter_editor(EDIT).expect("chapter editor");
        assert_eq!(fields.title, "A Map of Small Things");
        assert_eq!(fields.position, "2");
        assert_eq!(fields.wip_length, "2");
        assert_eq!(
            fields.summary,
            "The apprentice redraws the harbour from memory."
        );
        assert!(fields.notes.contains("PLACEHOLDER CHAPTER TWO NOTE"));
        assert!(fields.endnotes.is_empty());
        assert!(fields.content.contains("PLACEHOLDER PROSE (chapter two)"));
        assert!(
            fields.content.contains("<p>") && !fields.content.contains("&lt;p&gt;"),
            "author markup is decoded text, not serialized entities: {}",
            fields.content
        );
    }

    #[test]
    fn a_non_chapter_page_fails_closed() {
        assert!(parse_chapter_index("<html><body>login</body></html>").is_err());
        assert!(parse_chapter_editor("<html><body>login</body></html>").is_err());
    }

    #[test]
    fn chapter_validation_mirrors_the_model_limits() {
        let mut fields = ChapterFields {
            title: "One".into(),
            position: "1".into(),
            wip_length: "3".into(),
            content: "At least ten characters of prose.".into(),
            ..ChapterFields::default()
        };
        assert!(validate_chapter(&fields).is_empty());
        fields.content = "short".into();
        fields.summary = "s".repeat(SUMMARY_MAX + 1);
        fields.position = "first".into();
        let errors = validate_chapter(&fields);
        assert!(errors.iter().any(|error| error.contains("at least 10")));
        assert!(errors.iter().any(|error| error.contains("Chapter Summary")));
        assert!(errors.iter().any(|error| error.contains("Chapter Number")));
    }
}
