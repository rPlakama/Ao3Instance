//! Parser for an AO3 work page (`/works/:id` and `/works/:id/chapters/:id`).
//!
//! Every selector below is derived from the pinned otwarchive clone (`452391b`,
//! read-only copy in `otwarchive/`), not guessed:
//!
//! | field | clone evidence |
//! | --- | --- |
//! | title | `app/views/works/_preface.html.erb:2` — `<h2 class="title heading">` |
//! | authors | `_preface.html.erb:17` + `app/helpers/byline_helper.rb:72-83` — `<h3 class="byline heading">` with `rel="author"` links |
//! | summary | `_preface.html.erb:23-31` — `div.summary > blockquote.userstuff` |
//! | tag groups | `app/views/works/_meta.html.erb:6-28`, `app/helpers/tag_type_helper.rb:9-19` — `dd.<group>.tags` anchors `a.tag` (`app/helpers/tags_helper.rb:4,93-96`) |
//! | language | `_meta.html.erb:31-38` — `dd.language` |
//! | stats | `_meta.html.erb:51-56` + `app/helpers/works_helper.rb:4-31` — nested `dl.stats` with `dd.words` / `dd.comments` / `dd.kudos` |
//! | chapters (entire work) | `app/views/works/show.html.erb:44-55` + `app/views/chapters/_chapter.html.erb` — `#chapters` holding one `div.chapter` per chapter |
//! | chapter (single) | `app/views/works/show.html.erb:49-53` — `#chapters > div.userstuff` for a one-chapter work |
//! | chapter title/body/notes | `app/views/chapters/_chapter.html.erb:24-27,66-69,46-60` |
//! | chapter count | `app/views/works/_work_header_navigation.html.erb:29-35` — `#chapter_index select option`, one per chapter |
//! | chapter AO3 id | `app/views/chapters/_chapter.html.erb:23` — `h3.title a` → `[chapter.work, chapter]` = `/works/<work>/chapters/<chapter_id>` |
//! | comment form | `app/views/comments/_commentable.html.erb:131-137` wrapping `app/views/comments/_comment_form.html.erb:5` |
//! | work id | `/works/<id>` links/forms; there is no canonical meta tag (`app/views/layouts/application.html.erb`) |
//!
//! Contract:
//! - A missing title is drift: [`Error::UpstreamChanged`] carrying `work title missing`.
//! - Every other field is optional. A page with a title but no `#chapters`
//!   container yields an empty chapter list instead of an error; the reader
//!   reports "no chapter content found" and does not fabricate text.
//! - `body_html`, `notes_html` and `summary` are the unsanitized inner HTML of
//!   AO3's `userstuff` blocks. Sanitization belongs to the render layer; this
//!   parser must not silently drop what the creator wrote.

use std::sync::OnceLock;

use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::{
    collapse_whitespace, extract_digits_after, parse_number, parse_selector as css,
};
use crate::error::{Error, Result};

/// One parsed AO3 work page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkPage {
    /// Numeric work id recovered from the page's own `/works/<id>` paths.
    /// Empty when the page carries none; never an error.
    pub ao3_id: String,
    pub title: String,
    /// Creators from the byline, in display order. `["Anonymous"]` for an
    /// anonymous work with no visible pseud.
    pub authors: Vec<String>,
    /// Inner HTML of the summary's `blockquote.userstuff` (unsanitized).
    pub summary: Option<String>,
    pub fandoms: Vec<String>,
    pub relationships: Vec<String>,
    pub characters: Vec<String>,
    pub additional_tags: Vec<String>,
    pub warnings: Vec<String>,
    pub categories: Vec<String>,
    /// Rating tag name, e.g. `Teen And Up Audiences`.
    pub rating: Option<String>,
    pub language: Option<String>,
    /// AO3 `Published:` date, ISO `YYYY-MM-DD` when present.
    pub published_at: Option<String>,
    /// `Some(true)` when AO3 marks the work Completed; `Some(false)` when it
    /// shows `Updated:` instead; `None` when the page does not say.
    pub complete: Option<bool>,
    pub word_count: Option<u64>,
    pub kudos: Option<u64>,
    pub comment_count: Option<u64>,
    pub hits: Option<u64>,
    pub bookmarks: Option<u64>,
    /// Total chapter count as AO3 displays it: from the chapter index, else the
    /// chapter list in the page, else the number of parsed chapters.
    pub chapter_count: usize,
    /// Chapters present on this page: all of them under `?view_full_work=true`,
    /// otherwise the single rendered chapter.
    pub chapters: Vec<Chapter>,
    /// The comment form is on the page, i.e. comments are open.
    pub comment_form_present: bool,
}

/// One rendered chapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// Chapter position as AO3 displays it (`chapter-<position>` id, or the
    /// "Chapter N" header). 1-based.
    pub index: usize,
    /// Text after the `Chapter N:` header, when the creator supplied a title.
    pub title: Option<String>,
    /// AO3 chapter record id from the chapter header link
    /// (`/works/<work_id>/chapters/<chapter_id>`). `None` for the bare
    /// single-chapter layout, which has no chapter wrapper at all.
    pub ao3_id: Option<String>,
    /// Inner HTML of the chapter's `div.userstuff` block (unsanitized), with
    /// AO3's landmark heading removed.
    pub body_html: String,
    /// Inner HTML of the chapter notes (`blockquote.userstuff`). Top-of-chapter
    /// notes win; chapter endnotes are used only when there are no top notes
    /// (the public type has no separate endnotes field yet).
    pub notes_html: Option<String>,
}

/// Parses a work page.
///
/// Returns [`Error::UpstreamChanged`] when the page has no work title — that
/// means we are not looking at a work page any more (login wall, interstital,
/// layout change). Missing optional fields are `None`/empty, never errors.
pub fn parse_work_page(html: &str) -> Result<WorkPage> {
    let document = Html::parse_document(html);
    let selectors = selectors()?;

    let title = extract_title(&document, selectors)?;
    let meta = first_in_document(&document, &selectors.meta);
    let chapters = extract_chapters(&document, selectors);
    let chapter_count = extract_chapter_count(&document, selectors, &chapters);
    let comment_form_present = first_in_document(&document, &selectors.comment_form).is_some();

    Ok(WorkPage {
        ao3_id: extract_ao3_id(&document, selectors),
        title,
        authors: extract_authors(&document, selectors),
        summary: extract_summary(&document, selectors),
        fandoms: tag_names(meta, &selectors.fandom_tags),
        relationships: tag_names(meta, &selectors.relationship_tags),
        characters: tag_names(meta, &selectors.character_tags),
        additional_tags: tag_names(meta, &selectors.freeform_tags),
        warnings: tag_names(meta, &selectors.warning_tags),
        categories: tag_names(meta, &selectors.category_tags),
        rating: tag_names(meta, &selectors.rating_tags).into_iter().next(),
        language: meta_value(meta, &selectors.language),
        published_at: meta_value(meta, &selectors.published)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        complete: meta_value(meta, &selectors.status_label).map(|label| {
            let label = label.to_ascii_lowercase();
            label.contains("completed")
        }),
        word_count: meta_value(meta, &selectors.words).and_then(|value| parse_number(&value)),
        kudos: meta_value(meta, &selectors.kudos).and_then(|value| parse_number(&value)),
        comment_count: meta_value(meta, &selectors.comments).and_then(|value| parse_number(&value)),
        hits: meta_value(meta, &selectors.hits).and_then(|value| parse_number(&value)),
        bookmarks: meta_value(meta, &selectors.bookmarks).and_then(|value| parse_number(&value)),
        chapter_count,
        chapters,
        comment_form_present,
    })
}

/// Parsed once per call; a bad literal is a parser bug, surfaced as
/// [`Error::UpstreamChanged`] rather than a panic (the integration tests fail
/// first if one is ever mistyped).
struct Selectors {
    title: Selector,
    byline: Selector,
    byline_link: Selector,
    meta: Selector,
    work_preface: Selector,
    summary_block: Selector,
    rating_tags: Selector,
    warning_tags: Selector,
    category_tags: Selector,
    fandom_tags: Selector,
    relationship_tags: Selector,
    character_tags: Selector,
    freeform_tags: Selector,
    language: Selector,
    published: Selector,
    status_label: Selector,
    words: Selector,
    comments: Selector,
    kudos: Selector,
    hits: Selector,
    bookmarks: Selector,
    chapters: Selector,
    chapter_units: Selector,
    chapter_body: Selector,
    chapter_title: Selector,
    chapter_title_link: Selector,
    chapter_notes: Selector,
    chapter_endnotes: Selector,
    chapter_index_options: Selector,
    landmark_heading: Selector,
    comment_form: Selector,
    work_paths: Selector,
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
        title: css("h2.title")?,
        byline: css("h3.byline")?,
        byline_link: css("a")?,
        meta: css("dl.meta")?,
        work_preface: css("div.preface")?,
        summary_block: css("div.summary blockquote")?,
        rating_tags: css("dd.rating.tags a.tag")?,
        warning_tags: css("dd.warning.tags a.tag")?,
        category_tags: css("dd.category.tags a.tag")?,
        fandom_tags: css("dd.fandom.tags a.tag")?,
        relationship_tags: css("dd.relationship.tags a.tag")?,
        character_tags: css("dd.character.tags a.tag")?,
        freeform_tags: css("dd.freeform.tags a.tag")?,
        language: css("dd.language")?,
        published: css("dd.published")?,
        status_label: css("dt.status")?,
        words: css("dd.words")?,
        comments: css("dd.comments")?,
        kudos: css("dd.kudos")?,
        hits: css("dd.hits")?,
        bookmarks: css("dd.bookmarks")?,
        chapters: css("#chapters")?,
        chapter_units: css("#chapters > div.chapter")?,
        chapter_body: css("div.userstuff")?,
        chapter_title: css("h3.title")?,
        chapter_title_link: css("a")?,
        chapter_notes: css("div#notes blockquote")?,
        chapter_endnotes: css("div.end.notes blockquote")?,
        chapter_index_options: css("#chapter_index option")?,
        landmark_heading: css("h3.landmark")?,
        // `comments/_commentable.html.erb`: the wrapper is always `#add_comment`;
        // the form ids are kept as a fallback in case the wrapper class moves.
        comment_form: css(
            r#"#add_comment, div[id^="comment_form_for_"], form[id^="comment_for_"]"#,
        )?,
        work_paths: css("a[href], form[action]")?,
    })
}

fn first<'a>(element: ElementRef<'a>, selector: &Selector) -> Option<ElementRef<'a>> {
    element.select(selector).next()
}

fn first_in_document<'a>(document: &'a Html, selector: &Selector) -> Option<ElementRef<'a>> {
    document.select(selector).next()
}

fn extract_title(document: &Html, selectors: &Selectors) -> Result<String> {
    let title = first_in_document(document, &selectors.title)
        .map(|element| collapsed_text(element))
        .unwrap_or_default();
    if title.is_empty() {
        return Err(Error::UpstreamChanged("work title missing".to_string()));
    }
    Ok(title)
}

fn extract_authors(document: &Html, selectors: &Selectors) -> Vec<String> {
    let Some(byline) = first_in_document(document, &selectors.byline) else {
        return Vec::new();
    };
    let linked: Vec<String> = byline
        .select(&selectors.byline_link)
        .map(|link| collapsed_text(link))
        .filter(|name| !name.is_empty())
        .collect();
    if !linked.is_empty() {
        return linked;
    }
    // Anonymous works render "Anonymous" as plain text instead of links.
    let anonymous = collapsed_text(byline);
    if anonymous.is_empty() {
        Vec::new()
    } else {
        vec![anonymous]
    }
}

fn extract_summary(document: &Html, selectors: &Selectors) -> Option<String> {
    // Scope to the work preface: chapter prefaces also render summaries, and a
    // work without a summary must stay `None` even if a chapter has one.
    let preface = first_in_document(document, &selectors.work_preface)?;
    let summary = first(preface, &selectors.summary_block)?;
    non_empty_inner_html(summary)
}

fn tag_names(container: Option<ElementRef<'_>>, selector: &Selector) -> Vec<String> {
    let Some(container) = container else {
        return Vec::new();
    };
    container
        .select(selector)
        .map(|tag| collapsed_text(tag))
        .filter(|name| !name.is_empty())
        .collect()
}

fn meta_value(container: Option<ElementRef<'_>>, selector: &Selector) -> Option<String> {
    let value = collapsed_text(first(container?, selector)?);
    if value.is_empty() { None } else { Some(value) }
}

fn extract_ao3_id(document: &Html, selectors: &Selectors) -> String {
    let mut chapter_link = None;
    let mut sub_path = None;
    let mut plain_path = None;
    for element in document.select(&selectors.work_paths) {
        let Some(target) = element.attr("href").or_else(|| element.attr("action")) else {
            continue;
        };
        for (id, rest) in work_id_paths(target) {
            if rest == "/chapters" || rest.starts_with("/chapters/") {
                chapter_link.get_or_insert(id);
            } else if rest.starts_with('/') {
                sub_path.get_or_insert(id);
            } else {
                plain_path.get_or_insert(id);
            }
        }
    }
    chapter_link
        .or(sub_path)
        .or(plain_path)
        .unwrap_or_default()
        .to_string()
}

/// Every `/works/<id><rest>` occurrence in a link target. Chapter links win when
/// choosing the page's own work: notes can link other works ("inspired by"), and
/// a plain `/works/<id>` from a note must not be mistaken for this page's id.
fn work_id_paths(target: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some(position) = target[offset..].find("/works/") {
        let after = offset + position + "/works/".len();
        let rest = &target[after..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            found.push((&rest[..digits], &rest[digits..]));
        }
        offset = after;
    }
    found
}

fn extract_chapters(document: &Html, selectors: &Selectors) -> Vec<Chapter> {
    let Some(container) = first_in_document(document, &selectors.chapters) else {
        // No chapter container: the reader shows "no chapter content found".
        return Vec::new();
    };
    let units: Vec<ElementRef<'_>> = document.select(&selectors.chapter_units).collect();
    if units.is_empty() {
        // One-chapter work: `#chapters` holds the bare `div.userstuff` block.
        return first(container, &selectors.chapter_body)
            .map(|body| Chapter {
                index: 1,
                title: None,
                ao3_id: None,
                body_html: body_inner_html(body, selectors),
                notes_html: None,
            })
            .into_iter()
            .collect();
    }
    units
        .into_iter()
        .enumerate()
        .map(|(position, unit)| parse_chapter(unit, position, selectors))
        .collect()
}

fn parse_chapter(unit: ElementRef<'_>, position: usize, selectors: &Selectors) -> Chapter {
    Chapter {
        index: chapter_index(unit, position, selectors),
        title: chapter_title(unit, selectors),
        ao3_id: chapter_ao3_id(unit, selectors),
        body_html: first(unit, &selectors.chapter_body)
            .map(|body| body_inner_html(body, selectors))
            .unwrap_or_default(),
        notes_html: first(unit, &selectors.chapter_notes)
            .or_else(|| first(unit, &selectors.chapter_endnotes))
            .and_then(non_empty_inner_html),
    }
}

fn chapter_index(unit: ElementRef<'_>, position: usize, selectors: &Selectors) -> usize {
    // `chapters/_chapter.html.erb:2` emits `id="chapter-<position>"`.
    if let Some(number) = unit
        .attr("id")
        .and_then(|id| id.strip_prefix("chapter-"))
        .and_then(|position| position.parse::<usize>().ok())
        && number > 0
    {
        return number;
    }
    // Fall back to the displayed "Chapter N" header text.
    if let Some(header) = first(unit, &selectors.chapter_title)
        && let Some(number) = first(header, &selectors.chapter_title_link)
            .and_then(|link| displayed_chapter_number(&collapsed_text(link)))
    {
        return number;
    }
    position + 1
}

/// `chapter.chapter_header` is `config/locales/models/en.yml:24` — "Chapter N".
fn displayed_chapter_number(header: &str) -> Option<usize> {
    header.strip_prefix("Chapter ")?.trim().parse().ok()
}

fn chapter_title(unit: ElementRef<'_>, selectors: &Selectors) -> Option<String> {
    let header = first(unit, &selectors.chapter_title)?;
    let full = collapsed_text(header);
    let header_text = first(header, &selectors.chapter_title_link)
        .map(|link| collapsed_text(link))
        .unwrap_or_default();
    let remainder = full.strip_prefix(&header_text).unwrap_or(&full).trim();
    let title = remainder.strip_prefix(':').unwrap_or(remainder).trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

/// AO3 chapter record id from the chapter header link.
///
/// `chapters/_chapter.html.erb:23` renders
/// `link_to chapter.chapter_header, [chapter.work, chapter]`, i.e.
/// `/works/<work_id>/chapters/<chapter_id>`; the chapter index `<select>` also
/// carries the same ids but only exists for multi-chapter works.
fn chapter_ao3_id(unit: ElementRef<'_>, selectors: &Selectors) -> Option<String> {
    let header = first(unit, &selectors.chapter_title)?;
    let link = first(header, &selectors.chapter_title_link)?;
    let href = link.value().attr("href")?;
    extract_digits_after(href, "/chapters/").map(str::to_string)
}

/// Inner HTML of a `userstuff` block without AO3's screen-reader landmark
/// heading (`<h3 class="landmark heading">Chapter Text</h3>`), which is wrapper
/// chrome rather than creator content.
fn body_inner_html(element: ElementRef<'_>, selectors: &Selectors) -> String {
    let inner_html = element.inner_html();
    let inner = inner_html.trim();
    if let Some(landmark) = first(element, &selectors.landmark_heading) {
        let landmark_html = landmark.html();
        if let Some(body) = inner.strip_prefix(&landmark_html) {
            return body.trim().to_string();
        }
    }
    inner.to_string()
}

fn non_empty_inner_html(element: ElementRef<'_>) -> Option<String> {
    let inner_html = element.inner_html();
    let inner = inner_html.trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

fn extract_chapter_count(document: &Html, selectors: &Selectors, chapters: &[Chapter]) -> usize {
    let indexed = document
        .select(&selectors.chapter_index_options)
        .filter(|option| option.attr("value").is_some_and(|value| !value.is_empty()))
        .count();
    if indexed > 0 {
        return indexed;
    }
    // In an entire-work page the rendered chapter list is the count.
    let listed = document.select(&selectors.chapter_units).count();
    if listed > 0 { listed } else { chapters.len() }
}

fn collapsed_text(element: ElementRef<'_>) -> String {
    collapse_whitespace(&element.text().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_numbers_ignore_thousands_separators() {
        assert_eq!(parse_number("4,321"), Some(4321));
        assert_eq!(parse_number(" 1,024 "), Some(1024));
        assert_eq!(parse_number("—"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn work_id_paths_split_id_from_the_rest_of_the_path() {
        assert_eq!(
            work_id_paths("/works/123/chapters/456"),
            vec![("123", "/chapters/456")]
        );
        assert_eq!(
            work_id_paths("https://archiveofourown.org/works/42?view_full_work=true"),
            vec![("42", "?view_full_work=true")]
        );
        assert!(work_id_paths("/series/9").is_empty());
        assert!(work_id_paths("/works/not-a-number").is_empty());
    }

    #[test]
    fn chapter_header_number_comes_from_the_displayed_header() {
        assert_eq!(displayed_chapter_number("Chapter 12"), Some(12));
        assert_eq!(displayed_chapter_number("Chapter"), None);
        assert_eq!(displayed_chapter_number("Kapitel 3"), None);
    }

    #[test]
    fn chapter_title_drops_the_header_and_body_drops_the_landmark() {
        let html = r#"<div id="chapters">
            <div class="chapter" id="chapter-2">
              <div class="chapter preface group">
                <h3 class="title"><a href="/works/1/chapters/2">Chapter 2</a>: A Map of Small Things</h3>
                <div id="notes" class="notes module">
                  <h3 class="heading">Notes:</h3>
                  <blockquote class="userstuff"><p>Top notes.</p></blockquote>
                </div>
              </div>
              <div class="userstuff module" role="article">
                <h3 class="landmark heading" id="work">Chapter Text</h3>
                <p>Body.</p>
              </div>
              <div class="chapter preface group">
                <div class="end notes module" id="chapter_2_endnotes">
                  <h3 class="heading">Notes:</h3>
                  <blockquote class="userstuff"><p>End notes.</p></blockquote>
                </div>
              </div>
            </div>
          </div>"#;
        let document = Html::parse_document(html);
        let selectors = selectors().expect("selectors");
        let unit = first_in_document(&document, &selectors.chapter_units).expect("chapter");

        let chapter = parse_chapter(unit, 0, selectors);
        assert_eq!(chapter.index, 2);
        assert_eq!(chapter.title.as_deref(), Some("A Map of Small Things"));
        assert_eq!(chapter.body_html, "<p>Body.</p>");
        assert_eq!(
            chapter.notes_html.as_deref(),
            Some("<p>Top notes.</p>"),
            "top-of-chapter notes win over endnotes"
        );
    }

    #[test]
    fn chapter_without_top_notes_falls_back_to_endnotes() {
        let html = r#"<div id="chapters">
            <div class="chapter" id="chapter-1">
              <div class="userstuff module" role="article">
                <h3 class="landmark heading" id="work">Chapter Text</h3>
                <p>Body.</p>
              </div>
              <div class="chapter preface group">
                <div class="end notes module" id="chapter_1_endnotes">
                  <h3 class="heading">Notes:</h3>
                  <blockquote class="userstuff"><p>End notes only.</p></blockquote>
                </div>
              </div>
            </div>
          </div>"#;
        let document = Html::parse_document(html);
        let selectors = selectors().expect("selectors");
        let unit = first_in_document(&document, &selectors.chapter_units).expect("chapter");

        let chapter = parse_chapter(unit, 0, selectors);
        assert_eq!(
            chapter.notes_html.as_deref(),
            Some("<p>End notes only.</p>")
        );
    }
}
