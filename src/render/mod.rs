//! Server-rendered HTML: pages, escaping and sanitization.
//!
//! Author content is untrusted. It is sanitized with `ammonia` (tag/attribute
//! allowlist, http/https URLs, forced `noopener noreferrer`) and served inside a
//! sandboxed iframe with its own CSP — never injected into the app document
//! (`docs/security-standards.md` §8, T1).
//!
//! Segment renderers live in submodules; the shared skeleton and date helpers
//! live in `chrome`. `escape` and `sanitize` stay here because every renderer
//! needs them.

use std::collections::HashSet;

mod audit;
mod chrome;
mod home;
mod search;
mod session;
mod work;

pub use audit::{audit_page, confirm_page, error_page};
pub use home::home_page;
pub use search::search_page;
pub use session::session_page;
pub use work::{ReadContext, chapter_document, read_page};

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
}
