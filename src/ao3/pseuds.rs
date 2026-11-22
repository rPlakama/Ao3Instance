//! Parser for `/users/:login/pseuds`.
//!
//! Posting a comment requires `comment[pseud_id]`, and AO3 validates that the
//! pseud belongs to the account (`comments_controller.rb:79-87`). The default
//! pseud is marked with `<span class="current">Default Pseud</span>` inside its
//! `li.user.pseud` block; markup traced to
//! `app/views/pseuds/index.html.erb` + `_pseud_blurb.html.erb` + `_pseud_module.html.erb`.

use scraper::{Html, Selector};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pseud {
    pub id: i64,
    pub name: String,
}

/// All pseuds on the page, in display order.
pub fn parse_pseuds(html: &str) -> Vec<Pseud> {
    let doc = Html::parse_document(html);
    let Ok(block_selector) = Selector::parse("li.user.pseud") else {
        return Vec::new();
    };
    let Ok(anchor_selector) = Selector::parse("h4 a[href*='/pseuds/']") else {
        return Vec::new();
    };

    let mut pseuds = Vec::new();
    for block in doc.select(&block_selector) {
        for anchor in block.select(&anchor_selector) {
            let Some(href) = anchor.value().attr("href") else {
                continue;
            };
            let Some(id) = pseud_id_from_href(href) else {
                continue;
            };
            let name = anchor.text().collect::<String>().trim().to_string();
            if name.is_empty() {
                continue;
            }
            pseuds.push(Pseud { id, name });
        }
    }

    // Fallback: some layouts render the anchors outside a recognised block.
    if pseuds.is_empty()
        && let Ok(any_anchor) = Selector::parse("a[href*='/pseuds/']")
    {
        for anchor in doc.select(&any_anchor) {
            let Some(id) = anchor.value().attr("href").and_then(pseud_id_from_href) else {
                continue;
            };
            let name = anchor.text().collect::<String>().trim().to_string();
            if !name.is_empty() {
                pseuds.push(Pseud { id, name });
            }
        }
    }
    pseuds
}

/// The default pseud, or the first one when AO3 does not mark a default.
pub fn default_pseud(html: &str) -> Option<Pseud> {
    let doc = Html::parse_document(html);
    let Ok(block_selector) = Selector::parse("li.user.pseud") else {
        return None;
    };
    let Ok(anchor_selector) = Selector::parse("h4 a[href*='/pseuds/']") else {
        return None;
    };
    let Ok(current_selector) = Selector::parse("span.current") else {
        return None;
    };

    if let Some(block) = doc
        .select(&block_selector)
        .find(|block| block.select(&current_selector).next().is_some())
    {
        for anchor in block.select(&anchor_selector) {
            if let Some(id) = anchor.value().attr("href").and_then(pseud_id_from_href) {
                let name = anchor.text().collect::<String>().trim().to_string();
                if !name.is_empty() {
                    return Some(Pseud { id, name });
                }
            }
        }
    }

    parse_pseuds(html).into_iter().next()
}

/// Extracts the numeric id from a `/users/<login>/pseuds/<id>` href.
pub fn pseud_id_from_href(href: &str) -> Option<i64> {
    let marker = "/pseuds/";
    let start = href.find(marker)? + marker.len();
    let digits: String = href[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Validates that a page looks like the pseuds index (drift detection).
pub fn ensure_pseuds_page(html: &str) -> Result<()> {
    if html.contains("class=\"pseud index") || html.contains(">/pseuds/") {
        Ok(())
    } else {
        Err(Error::UpstreamChanged(
            "pseuds page did not look like the AO3 pseud index".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        std::fs::read_to_string("tests/fixtures/pseuds.html").expect("pseuds fixture")
    }

    #[test]
    fn parses_all_pseuds_in_order() {
        let pseuds = parse_pseuds(&fixture());
        assert_eq!(
            pseuds,
            vec![
                Pseud {
                    id: 111,
                    name: "Main Pseud".into()
                },
                Pseud {
                    id: 222,
                    name: "Second Pseud".into()
                }
            ]
        );
    }

    #[test]
    fn default_pseud_prefers_the_marked_one() {
        assert_eq!(
            default_pseud(&fixture()),
            Some(Pseud {
                id: 111,
                name: "Main Pseud".into()
            })
        );
    }

    #[test]
    fn default_pseud_falls_back_to_first() {
        let html =
            r#"<ul><li class="user pseud"><h4><a href="/users/x/pseuds/9">Only</a></h4></li></ul>"#;
        assert_eq!(
            default_pseud(html),
            Some(Pseud {
                id: 9,
                name: "Only".into()
            })
        );
    }

    #[test]
    fn empty_or_drifted_pages_yield_nothing() {
        assert!(default_pseud("<html></html>").is_none());
        assert!(parse_pseuds("<html></html>").is_empty());
        assert!(ensure_pseuds_page("<html></html>").is_err());
    }

    #[test]
    fn href_ids_are_parsed_strictly() {
        assert_eq!(pseud_id_from_href("/users/a/pseuds/123"), Some(123));
        assert_eq!(pseud_id_from_href("/users/a/pseuds/123?x=1"), Some(123));
        assert_eq!(pseud_id_from_href("/users/a/pseuds/abc"), None);
        assert_eq!(pseud_id_from_href("/works/1"), None);
    }
}
