//! Shared HTML and numeric extraction primitives for AO3 page parsers.

use scraper::{ElementRef, Selector};

use crate::error::{Error, Result};

/// Collapses runs of Unicode whitespace into single ASCII spaces and trims ends.
pub(crate) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collects text nodes from an HTML element and collapses whitespace.
pub(crate) fn collapse_text(element: &ElementRef<'_>) -> String {
    let joined = element.text().collect::<Vec<_>>().join(" ");
    collapse_whitespace(&joined)
}

/// Parses a displayed AO3 number (ignoring commas/spaces such as `1,024`).
pub(crate) fn parse_number(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

/// Extracts the leading ASCII digits immediately following `marker` in `input`.
pub(crate) fn extract_digits_after<'a>(input: &'a str, marker: &str) -> Option<&'a str> {
    let start = input.find(marker)? + marker.len();
    let rest = &input[start..];
    let len = rest.bytes().take_while(u8::is_ascii_digit).count();
    (len > 0).then_some(&rest[..len])
}

/// Parses a CSS selector, surfacing any bug as [`Error::UpstreamChanged`].
pub(crate) fn parse_selector(selector: &str) -> Result<Selector> {
    Selector::parse(selector)
        .map_err(|_| Error::UpstreamChanged(format!("parser selector is invalid: {selector}")))
}

#[cfg(test)]
mod tests {
    use scraper::Html;

    use super::*;

    #[test]
    fn collapses_whitespace_and_element_text() {
        assert_eq!(collapse_whitespace("  hello \n\t world  "), "hello world");
        let doc = Html::parse_fragment("<div><span>Hello</span>\n  <b>world</b></div>");
        let sel = parse_selector("div").expect("div selector");
        let el = doc.select(&sel).next().expect("div");
        assert_eq!(collapse_text(&el), "Hello world");
    }

    #[test]
    fn parses_delimited_numbers() {
        assert_eq!(parse_number("4,321"), Some(4_321));
        assert_eq!(parse_number(" 1,024 "), Some(1_024));
        assert_eq!(parse_number("—"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn extracts_digits_after_marker() {
        assert_eq!(
            extract_digits_after("/users/a/pseuds/123?x=1", "/pseuds/"),
            Some("123")
        );
        assert_eq!(
            extract_digits_after("https://archiveofourown.org/works/42", "/works/"),
            Some("42")
        );
        assert_eq!(extract_digits_after("/works/abc", "/works/"), None);
        assert_eq!(extract_digits_after("/series/9", "/works/"), None);
    }
}
