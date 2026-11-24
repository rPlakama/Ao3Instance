//! Public AO3 site skins: the list at `/skins` and the CSS at `/skins/<id>`.

use std::sync::OnceLock;

use scraper::{ElementRef, Html, Selector};

struct SkinSelectors {
    skin_item: Selector,
    heading_link: Selector,
    heading: Selector,
    summary: Selector,
    css_pre: Selector,
}

fn skin_selectors() -> Option<&'static SkinSelectors> {
    static CACHE: OnceLock<Option<SkinSelectors>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            Some(SkinSelectors {
                skin_item: Selector::parse("ul.skin.picture.index.group li.skins").ok()?,
                heading_link: Selector::parse("h4.heading a").ok()?,
                heading: Selector::parse("h4.heading").ok()?,
                summary: Selector::parse("blockquote.summary").ok()?,
                css_pre: Selector::parse("div.css.module blockquote pre").ok()?,
            })
        })
        .as_ref()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSkin {
    pub title: String,
    pub byline: String,
    pub href: String,
    pub description: String,
}

pub fn parse_skin_list(html: &str) -> Vec<PublicSkin> {
    let Some(sel) = skin_selectors() else {
        return Vec::new();
    };
    let document = Html::parse_document(html);
    document
        .select(&sel.skin_item)
        .filter_map(|node| {
            let link = node.select(&sel.heading_link).next()?;
            let title = text(&link);
            if title.is_empty() {
                return None;
            }
            let href = link.value().attr("href")?.to_string();
            let byline = node
                .select(&sel.heading)
                .next()
                .map(|heading| text(&heading))
                .unwrap_or_default();
            let description = node
                .select(&sel.summary)
                .next()
                .map(|node| text(&node))
                .unwrap_or_default();
            Some(PublicSkin {
                title,
                byline,
                href,
                description,
            })
        })
        .collect()
}

pub fn parse_skin_css(html: &str) -> Option<String> {
    let sel = skin_selectors()?;
    let document = Html::parse_document(html);
    let css = document
        .select(&sel.css_pre)
        .map(|node| node.text().collect::<Vec<_>>().join("\n"))
        .collect::<Vec<_>>()
        .join("\n");
    (!css.trim().is_empty()).then_some(css)
}

fn text(element: &ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_skin_list_and_css() {
        let list = include_str!("../../tests/fixtures/ao3_skins.html");
        let skins = parse_skin_list(list);
        assert_eq!(skins.len(), 1);
        assert_eq!(skins[0].title, "Chained Melody");
        assert_eq!(skins[0].href, "/skins/942");
        assert!(skins[0].byline.contains("tester"));
        assert!(skins[0].description.contains("Chained"));

        let page = include_str!("../../tests/fixtures/ao3_skin.html");
        let css = parse_skin_css(page).expect("css");
        assert!(css.contains("background"));
        assert!(css.contains("#123"));
    }
}
