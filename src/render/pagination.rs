//! AO3-style numbered pagination, shared by search, browse and tag pages.

/// Renders `Previous … 1 2 [3] 4 … 472 Next`. `path` is the local route and
/// `extra` is an already-encoded query string without a leading `&`.
pub fn pagination(page: i64, per_page: i64, total: i64, path: &str, extra: &str) -> String {
    let pages = (total + per_page - 1) / per_page;
    if pages <= 1 {
        return String::new();
    }
    let page = page.clamp(1, pages);

    let href = |target: i64| {
        let query = if extra.is_empty() {
            format!("page={target}")
        } else {
            format!("{extra}&page={target}")
        };
        format!("{path}?{query}")
    };

    let mut out = String::from("<ol class=\"pagination actions\">");
    if page > 1 {
        out.push_str(&format!(
            "<li><a href=\"{}\" rel=\"prev\">← Previous</a></li>",
            href(page - 1)
        ));
    } else {
        out.push_str("<li><span class=\"disabled\">← Previous</span></li>");
    }

    let mut last = 0;
    for candidate in page_numbers(page, pages) {
        if candidate == 0 {
            out.push_str("<li><span class=\"gap\">…</span></li>");
            continue;
        }
        if candidate == page {
            out.push_str(&format!(
                "<li class=\"current\"><span>{candidate}</span></li>"
            ));
        } else {
            out.push_str(&format!(
                "<li><a href=\"{}\">{candidate}</a></li>",
                href(candidate)
            ));
        }
        last = candidate;
    }
    if last < pages && page < pages {
        out.push_str(&format!(
            "<li><a href=\"{}\" rel=\"next\">Next →</a></li>",
            href(page + 1)
        ));
    } else if page >= pages {
        out.push_str("<li><span class=\"disabled\">Next →</span></li>");
    } else {
        out.push_str(&format!(
            "<li><a href=\"{}\" rel=\"next\">Next →</a></li>",
            href(page + 1)
        ));
    }
    out.push_str("</ol>");
    out
}

/// Page numbers with `0` marking an ellipsis.
fn page_numbers(page: i64, pages: i64) -> Vec<i64> {
    let mut numbers = Vec::new();
    let start = (page - 2).max(1);
    let end = (page + 2).min(pages);
    if start > 1 {
        numbers.push(1);
        if start > 2 {
            numbers.push(0);
        }
    }
    numbers.extend(start..=end);
    if end < pages {
        if end + 1 < pages {
            numbers.push(0);
        }
        numbers.push(pages);
    }
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_pages_render_prev_and_next() {
        let html = pagination(1, 20, 40, "/search", "q=x");
        assert!(html.contains("href=\"/search?q=x&page=2\""), "{html}");
        assert!(html.contains("class=\"current\""));
    }

    #[test]
    fn long_lists_collapse_with_ellipses() {
        let numbers = page_numbers(5, 472);
        assert_eq!(numbers, vec![1, 0, 3, 4, 5, 6, 7, 0, 472]);
        assert!(pagination(5, 20, 9_440, "/tags/Test/works", "").contains("472"));
    }

    #[test]
    fn single_page_renders_nothing() {
        assert!(pagination(1, 20, 3, "/works", "").is_empty());
    }
}
