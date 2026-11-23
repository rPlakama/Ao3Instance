//! Public AO3 site skins: the list at `/skins` and the CSS at `/skins/<id>`.

use scraper::{ElementRef, Html, Selector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSkin {
    pub title: String,
    pub byline: String,
    pub href: String,
    pub description: String,
}

pub fn parse_skin_list(html: &str) -> Vec<PublicSkin> {
    let document = Html::parse_document(html);
    let Ok(selector) = css("ul.skin.picture.index.group li.skins") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .filter_map(|node| {
            let link = select_one(&node, "h4.heading a")?;
            let title = text(&link);
            if title.is_empty() {
                return None;
            }
            let href = link.value().attr("href")?.to_string();
            let byline = select_one(&node, "h4.heading")
                .map(|heading| text(&heading))
                .unwrap_or_default();
            let description = select_one(&node, "blockquote.summary")
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
    let document = Html::parse_document(html);
    let selector = css("div.css.module blockquote pre").ok()?;
    let css = document
        .select(&selector)
        .map(|node| node.text().collect::<Vec<_>>().join("\n"))
        .collect::<Vec<_>>()
        .join("\n");
    (!css.trim().is_empty()).then_some(css)
}

fn select_one<'a>(node: &'a ElementRef<'a>, selector: &str) -> Option<ElementRef<'a>> {
    Selector::parse(selector)
        .ok()
        .and_then(|selector| node.select(&selector).next())
}

fn css(selector: &str) -> Result<Selector, ()> {
    Selector::parse(selector).map_err(|_| ())
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
