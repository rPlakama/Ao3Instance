//! Shared HTML fragments reused across renderers (Rule of Two / `inherit`).

use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::time::format_utc;

use super::{escape, sanitize, tag_href};

/// Renders the 30-minute SQLite page-cache notice banner shared by live AO3 pages.
pub(super) fn cache_banner(
    cache_meta: Option<(bool, i64)>,
    refresh_href: &str,
    live_label: &str,
) -> String {
    let Some((from_cache, fetched_at)) = cache_meta else {
        return String::new();
    };
    let status = if from_cache {
        format!(
            "Cached from archiveofourown.org at {} (30-min cache)",
            escape(&format_utc(fetched_at))
        )
    } else {
        format!("{live_label} (cached for 30 min)")
    };
    format!("<p class=\"meta\">{status} · <a href=\"{refresh_href}\">Refresh now</a></p>")
}

/// Renders an optional `<div class="flash notice">`.
pub(super) fn flash_notice(message: Option<&str>) -> String {
    message.map_or_else(String::new, |msg| {
        format!("<div class=\"flash notice\">{}</div>", escape(msg))
    })
}

/// Renders an optional `<div class="flash error">`.
pub(super) fn flash_error(message: Option<&str>) -> String {
    message.map_or_else(String::new, |msg| {
        format!("<div class=\"flash error\">{}</div>", escape(msg))
    })
}

/// Author links pointing to the offline local browse filter (`/works?author=...`).
pub(super) fn author_links_local(authors: &[String]) -> String {
    if authors.is_empty() {
        return "Anonymous".to_string();
    }
    authors
        .iter()
        .map(|author| {
            format!(
                "<a href=\"/works?author={}\">{}</a>",
                percent_encode(author, SpaceMode::Percent20),
                escape(author)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Author links pointing to live AO3 creator search (`/works/search?creator=...`).
pub(super) fn author_links_live(authors: &[String]) -> String {
    if authors.is_empty() {
        return "Anonymous".to_string();
    }
    authors
        .iter()
        .map(|author| {
            format!(
                "<a href=\"/works/search?creator={}\">{}</a>",
                percent_encode(author, SpaceMode::Percent20),
                escape(author)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Comma-separated AO3 fandom tag links (or `"—"` when empty).
pub(super) fn fandom_links(fandoms: &[String]) -> String {
    if fandoms.is_empty() {
        return "—".to_string();
    }
    fandoms
        .iter()
        .map(|fandom| {
            format!(
                "<a class=\"tag\" href=\"{}\">{}</a>",
                tag_href(fandom),
                escape(fandom)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders `<li class="{class}"><a class="tag">...</a></li>` items for one tag group.
pub(super) fn tag_list_items(tags: &[String], class: &str) -> String {
    let mut out = String::new();
    for tag in tags {
        out.push_str(&format!(
            "<li class=\"{class}\"><a class=\"tag\" href=\"{}\">{}</a></li>",
            tag_href(tag),
            escape(tag)
        ));
    }
    out
}

/// Renders a sanitized `<blockquote class="userstuff summary">` block when non-empty.
pub(super) fn summary_block(summary_html: Option<&str>) -> String {
    summary_html
        .filter(|summary| !summary.trim().is_empty())
        .map_or_else(String::new, |summary| {
            format!(
                "<h6 class=\"landmark heading\">Summary</h6>\
                 <blockquote class=\"userstuff summary\">{}</blockquote>",
                sanitize(summary)
            )
        })
}

/// Renders one `<dt>` / `<dd>` stat pair inside `<dl class="stats">`.
pub(super) fn stat_row(class: &str, label: &str, value_html: &str) -> String {
    format!("<dt class=\"{class}\">{label}:</dt><dd class=\"{class}\">{value_html}</dd>")
}

/// Wraps an AO3-style filter form in the collapsible "Sort and Filter" panel.
///
/// `open` is always rendered so desktop and no-JS readers see the filters;
/// `ui.js` removes it on narrow viewports when no filter is active.
pub(super) fn filter_panel(active: bool, form_markup: &str) -> String {
    format!(
        "<details id=\"work-filters-panel\" class=\"filters-panel\" open data-active=\"{active}\">\
         <summary class=\"filters-toggle\">Sort and Filter</summary>{form_markup}</details>"
    )
}

/// Renders `<option>` elements for a static `(value, label)` slice.
pub(super) fn select_options(options: &[(&str, &str)], current: &str) -> String {
    let mut out = String::new();
    for (value, label) in options {
        out.push_str(&format!(
            "<option value=\"{}\"{}>{label}</option>",
            escape(value),
            if current == *value { " selected" } else { "" }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_banner_formats_cached_and_live_states() {
        assert_eq!(cache_banner(None, "/r", "Live"), "");
        let live = cache_banner(Some((false, 0)), "/x?refresh=1", "Live results");
        assert!(live.contains("Live results (cached for 30 min)"), "{live}");
        assert!(live.contains("href=\"/x?refresh=1\""), "{live}");

        let cached = cache_banner(Some((true, 1_700_000_000)), "/x?refresh=1", "Live");
        assert!(cached.contains("30-min cache"), "{cached}");
    }

    #[test]
    fn filter_panel_wraps_the_form_and_marks_the_active_state() {
        let inactive = filter_panel(false, "<form id=\"work-filters\"></form>");
        assert_eq!(
            inactive,
            "<details id=\"work-filters-panel\" class=\"filters-panel\" open data-active=\"false\">\
             <summary class=\"filters-toggle\">Sort and Filter</summary>\
             <form id=\"work-filters\"></form></details>"
        );

        let active = filter_panel(true, "");
        assert!(
            active.contains(
                "<details id=\"work-filters-panel\" class=\"filters-panel\" open data-active=\"true\">"
            ),
            "{active}"
        );
        assert!(
            active.contains("<summary class=\"filters-toggle\">Sort and Filter</summary>"),
            "{active}"
        );
    }

    #[test]
    fn author_and_flash_helpers_escape_output() {
        assert_eq!(author_links_local(&[]), "Anonymous");
        assert_eq!(author_links_live(&[]), "Anonymous");
        assert!(author_links_local(&["a<b>".into()]).contains("a&lt;b&gt;"));
        assert!(author_links_live(&["a b".into()]).contains("creator=a%20b"));
        assert!(flash_notice(Some("<hi>")).contains("&lt;hi&gt;"));
        assert!(flash_error(Some("<err>")).contains("flash error"));
        assert_eq!(flash_notice(None), "");
    }
}
