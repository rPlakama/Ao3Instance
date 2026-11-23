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

use crate::ao3::parse::{SpaceMode, percent_encode};

/// Embedded stylesheet: `nix build` stays a self-contained single binary.
pub const AO3_CSS: &str = include_str!("assets/ao3.css");
pub const READER_JS: &str = include_str!("assets/reader.js");

mod about;
mod audit;
mod blurb;
mod browse;
mod chrome;
mod dashboard;
mod diagnostics;
mod fandoms;
mod history;
mod home;
mod icons;
mod lists;
mod not_cached;
mod pagination;
mod search;
mod session;
mod tag_search;
mod work;

pub use about::about_page;
pub use audit::{audit_page, challenge_error_page, confirm_page, error_page};
pub use browse::{browse_page, filter_from};
pub use chrome::{NavSection, PageContext, SolverSummary};
pub use dashboard::dashboard_page;
pub use diagnostics::{DiagnosticsView, diagnostics_page};
pub use fandoms::fandoms_page;
pub use history::history_page;
pub use home::home_page;
pub use lists::{bookmarks_page, reading_page, subscriptions_page};
pub use not_cached::not_cached_page;
pub use pagination::pagination;
pub use search::{
    filter_from as search_filter_from, live_search_form_page, live_search_results_page,
    search_form_page, search_results_page,
};
pub use search::{live_tag_page, search_challenge_page};
pub use session::{logout_confirm_page, session_page};
pub use tag_search::tag_search_page;
pub use work::{ReadContext, chapter_document, read_page};

/// Overall percentage from a within-chapter position.
pub(crate) fn overall_percent(chapter: i64, pct: i64, chapter_count: i64) -> i64 {
    let total = chapter_count.max(1);
    let chapter = chapter.clamp(1, total);
    let pct = pct.clamp(0, 100) as f64 / 100.0;
    (((chapter - 1) as f64 + pct) / total as f64 * 100.0)
        .round()
        .clamp(0.0, 100.0) as i64
}

/// AO3-shaped local tag URL: `/tags/<encoded>/works`.
pub(crate) fn tag_href(tag: &str) -> String {
    format!("/tags/{}/works", percent_encode(tag, SpaceMode::Percent20))
}

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
