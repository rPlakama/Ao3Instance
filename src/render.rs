//! Server-rendered HTML: pages, escaping and sanitization.
//!
//! Author content is untrusted. It is sanitized with `ammonia` (tag/attribute
//! allowlist, http/https URLs, forced `noopener noreferrer`) and served inside a
//! sandboxed iframe with its own CSP — never injected into the app document
//! (`docs/security-standards.md` §8, T1).

use std::collections::HashSet;

use crate::store::{StoredChapter, StoredWork, WorkSummary};

/// Minimal HTML text escaping for values interpolated into our own templates.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Sanitizes AO3-authored HTML for the reader iframe.
pub fn sanitize(html: &str) -> String {
    let schemes: HashSet<&'static str> = HashSet::from(["http", "https"]);
    ammonia::Builder::default()
        .link_rel(Some("noopener noreferrer"))
        .url_schemes(schemes)
        .clean(html)
        .to_string()
}

const STYLE: &str = r#"
:root { color-scheme: light dark; --fg:#1c1b1a; --bg:#fbfaf7; --muted:#6b6762; --line:#e2ddd5; --accent:#8c2f39; }
@media (prefers-color-scheme: dark) { :root { --fg:#e9e4dc; --bg:#171615; --muted:#a39c93; --line:#33302c; --accent:#e07b86; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:16px/1.6 "Iowan Old Style", Georgia, serif; }
header.site { display:flex; align-items:baseline; gap:1rem; padding:.75rem 1.5rem; border-bottom:1px solid var(--line); }
header.site h1 { font-size:1.05rem; margin:0; letter-spacing:.02em; }
header.site a { color:var(--accent); text-decoration:none; }
main { max-width: 52rem; margin:0 auto; padding:1.5rem; }
form.open { display:flex; gap:.5rem; margin:1rem 0 2rem; }
form.open input { flex:1; padding:.6rem .7rem; font:inherit; color:inherit; background:transparent; border:1px solid var(--line); border-radius:6px; }
form.open button { padding:.6rem 1rem; font:inherit; border:1px solid var(--accent); background:var(--accent); color:#fff; border-radius:6px; cursor:pointer; }
ul.works { list-style:none; padding:0; }
ul.works li { padding:.7rem 0; border-bottom:1px solid var(--line); }
ul.works a { color:var(--fg); text-decoration:none; font-weight:600; }
.meta, .muted { color:var(--muted); font-size:.9rem; }
.error { border:1px solid #b3423c; color:#b3423c; padding:.7rem 1rem; border-radius:6px; margin:1rem 0; }
.tags { display:flex; flex-wrap:wrap; gap:.35rem; margin:.6rem 0; }
.tag { font-size:.78rem; padding:.15rem .5rem; border:1px solid var(--line); border-radius:999px; color:var(--muted); }
.tag.warning { border-color:#b3423c; color:#b3423c; }
h2.title { margin:.2rem 0 0; font-size:1.9rem; line-height:1.25; }
.byline { margin:.35rem 0 .8rem; color:var(--muted); }
.toolbar { display:flex; flex-wrap:wrap; gap:.5rem; align-items:center; margin:1rem 0; padding:.5rem 0; border-top:1px solid var(--line); border-bottom:1px solid var(--line); }
.toolbar a, .toolbar span { font-size:.85rem; color:var(--muted); text-decoration:none; }
.toolbar a:hover { color:var(--accent); }
iframe.chapter { width:100%; border:0; background:transparent; min-height:70vh; }
footer.site { padding:1rem 1.5rem; border-top:1px solid var(--line); color:var(--muted); font-size:.8rem; }
"#;

fn layout(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><style>{STYLE}</style></head>\n<body>\
         <header class=\"site\"><h1><a href=\"/\">AO3Instance</a></h1>\
         <span class=\"muted\">local cache · reader</span></header>\
         <main>{body}</main>\
         <footer class=\"site\">Fetched from archiveofourown.org for personal offline reading. \
         Content belongs to its authors.</footer></body></html>"
    )
}

pub fn home_page(works: &[WorkSummary], error: Option<&str>) -> String {
    let mut body = String::from(
        "<form class=\"open\" method=\"post\" action=\"/open\">\
         <input name=\"id\" placeholder=\"AO3 work id or URL (e.g. 12345678)\" autofocus required>\
         <button type=\"submit\">Open work</button></form>",
    );
    if let Some(error) = error {
        body.push_str(&format!("<div class=\"error\">{}</div>", escape(error)));
    }
    if works.is_empty() {
        body.push_str(
            "<p class=\"muted\">Nothing cached yet. Paste a work id or URL above; \
             the work page is fetched once and kept for offline reading.</p>",
        );
    } else {
        body.push_str("<h2>Cached works</h2><ul class=\"works\">");
        for work in works {
            let authors = if work.authors.is_empty() {
                "Anonymous".to_string()
            } else {
                work.authors.join(", ")
            };
            body.push_str(&format!(
                "<li><a href=\"/read/{id}\">{title}</a>\
                 <div class=\"meta\">{authors} · {chapters} chapter(s)</div></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                authors = escape(&authors),
                chapters = work.chapter_count,
            ));
        }
        body.push_str("</ul>");
    }
    layout("AO3Instance", &body)
}

fn tag_chips(tags: &[String], class: &str) -> String {
    let mut out = String::new();
    for tag in tags {
        out.push_str(&format!(
            "<span class=\"tag {class}\">{}</span>",
            escape(tag)
        ));
    }
    out
}

pub fn read_page(work: &StoredWork, chapters: &[StoredChapter], current: i64) -> String {
    let authors = if work.authors.is_empty() {
        "Anonymous".to_string()
    } else {
        work.authors.join(", ")
    };
    let mut body = String::new();
    body.push_str(&format!(
        "<h2 class=\"title\">{}</h2><div class=\"byline\">by {}</div>",
        escape(&work.title),
        escape(&authors)
    ));

    let mut facts: Vec<String> = Vec::new();
    if let Some(rating) = &work.rating {
        facts.push(escape(rating));
    }
    if let Some(language) = &work.language {
        facts.push(escape(language));
    }
    if let Some(words) = work.word_count {
        facts.push(format!("{words} words"));
    }
    if let Some(kudos) = work.kudos {
        facts.push(format!("{kudos} kudos"));
    }
    if !facts.is_empty() {
        body.push_str(&format!("<div class=\"meta\">{}</div>", facts.join(" · ")));
    }

    let mut tags = String::new();
    tags.push_str(&tag_chips(&work.tags.warnings, "warning"));
    tags.push_str(&tag_chips(&work.tags.fandoms, ""));
    tags.push_str(&tag_chips(&work.tags.relationships, ""));
    tags.push_str(&tag_chips(&work.tags.characters, ""));
    tags.push_str(&tag_chips(&work.tags.additional, ""));
    tags.push_str(&tag_chips(&work.tags.categories, ""));
    if !tags.is_empty() {
        body.push_str(&format!("<div class=\"tags\">{tags}</div>"));
    }

    if let Some(summary) = &work.summary
        && !summary.trim().is_empty()
    {
        body.push_str(&format!(
            "<details><summary class=\"muted\">Summary</summary><div>{}</div></details>",
            sanitize(summary)
        ));
    }

    let chapter = chapters.iter().find(|chapter| chapter.idx == current);
    let total = chapters.len();
    let position = chapters
        .iter()
        .position(|chapter| chapter.idx == current)
        .unwrap_or(0);

    let mut toolbar = String::from("<div class=\"toolbar\">");
    toolbar.push_str(&format!(
        "<span>Chapter {} of {}</span>",
        if chapter.is_some() { position + 1 } else { 0 },
        total
    ));
    if position > 0 {
        let previous = chapters[position - 1].idx;
        toolbar.push_str(&format!(
            "<a href=\"/read/{}?chapter={previous}\">← previous</a>",
            escape(&work.ao3_id)
        ));
    }
    if position + 1 < total {
        let next = chapters[position + 1].idx;
        toolbar.push_str(&format!(
            "<a href=\"/read/{}?chapter={next}\">next →</a>",
            escape(&work.ao3_id)
        ));
    }
    toolbar.push_str(&format!(
        "<a href=\"https://archiveofourown.org/works/{}\" rel=\"noopener noreferrer\">\
         open on AO3</a>",
        escape(&work.ao3_id)
    ));
    toolbar.push_str("</div>");
    body.push_str(&toolbar);

    match chapter {
        Some(chapter) => {
            body.push_str(&format!(
                "<iframe class=\"chapter\" sandbox referrerpolicy=\"no-referrer\" \
                 title=\"Chapter {} of {}\" src=\"/read/{}/chapter/{}?v={}\"></iframe>",
                position + 1,
                total,
                escape(&work.ao3_id),
                chapter.idx,
                work.fetched_at
            ));
        }
        None => body.push_str(
            "<p class=\"muted\">No chapter content was stored for this work. \
             Re-open it to refresh (AO3's chapter markup may have changed).</p>",
        ),
    }

    layout(&format!("{} — AO3Instance", work.title), &body)
}

/// Standalone document for the chapter iframe. Served with its own CSP.
pub fn chapter_document(work_title: &str, chapter: &StoredChapter) -> String {
    let mut body = String::new();
    if let Some(title) = &chapter.title
        && !title.trim().is_empty()
    {
        body.push_str(&format!(
            "<h3 class=\"chapter-title\">{}</h3>",
            escape(title)
        ));
    }
    body.push_str(&sanitize(&chapter.body_html));
    if let Some(notes) = &chapter.notes_html
        && !notes.trim().is_empty()
    {
        body.push_str(&format!(
            "<hr class=\"notes-sep\"><div class=\"notes\"><strong>Chapter notes</strong>{}</div>",
            sanitize(notes)
        ));
    }
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{} — chapter</title><style>\
         body{{margin:0;padding:0 1.25rem;color:inherit;background:transparent;\
         font:16px/1.7 Georgia,serif;}} \
         a{{color:#8c2f39;}} img{{max-width:100%;height:auto;}} \
         .chapter-title{{font-size:1.1rem;font-weight:600;margin:.2rem 0 1rem;}} \
         .notes-sep{{border:0;border-top:1px solid #ccc4;margin:1.5rem 0 1rem;}} \
         .notes{{font-size:.92rem;color:#8a857e;}} \
         blockquote{{border-left:3px solid #ccc4;margin:1rem 0;padding-left:1rem;color:#8a857e;}}</style>\
         </head><body>{body}</body></html>",
        escape(work_title)
    )
}

pub fn error_page(status: u16, message: &str) -> String {
    layout(
        "AO3Instance — error",
        &format!(
            "<h2>Something went wrong</h2><div class=\"error\">HTTP {status}: {}</div>\
             <p><a href=\"/\">Back to the start</a></p>",
            escape(message)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_the_dangerous_characters() {
        assert_eq!(
            escape("<script>alert('x') & \"y\"</script>"),
            "&lt;script&gt;alert(&#39;x&#39;) &amp; &quot;y&quot;&lt;/script&gt;"
        );
    }

    #[test]
    fn sanitizer_strips_scripts_handlers_and_javascript_urls() {
        let dirty = r#"<p>hi<script>alert(1)</script></p><img src=x onerror=alert(1)><a href="javascript:alert(1)">x</a>"#;
        let clean = sanitize(dirty);
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("javascript:"));
        assert!(clean.contains("<p>"));
    }

    #[test]
    fn sanitizer_keeps_formatting_and_links() {
        let clean = sanitize(r#"<p><em>ok</em> <a href="https://example.org">link</a></p>"#);
        assert!(clean.contains("<em>"));
        assert!(clean.contains("https://example.org"));
    }

    #[test]
    fn chapter_document_includes_sanitized_body_and_notes() {
        let chapter = StoredChapter {
            idx: 1,
            title: Some("Chapter <One>".into()),
            body_html: "<p>body</p><script>bad()</script>".into(),
            notes_html: Some("<p>note</p>".into()),
        };
        let doc = chapter_document("w", &chapter);
        assert!(doc.contains("&lt;One&gt;"));
        assert!(doc.contains("<p>body</p>"));
        assert!(!doc.contains("bad()"));
        assert!(doc.contains("Chapter notes"));
    }
}
