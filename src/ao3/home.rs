//! Live AO3 home modules: favorite tags, news, read-later and unread inbox.
//!
//! One authenticated `GET /` is enough: AO3 renders all four modules there.
//! Parsing is best-effort — a module missing from the page is simply absent.

use scraper::{ElementRef, Html, Selector};

use super::search::{LiveWork, parse_work_element};

#[derive(Debug, Default, Clone)]
pub struct HomeModules {
    pub favorites: Vec<String>,
    pub news: Vec<HomeNewsPost>,
    pub readings: Vec<HomeReading>,
    pub inbox: Vec<HomeMessage>,
}

#[derive(Debug, Clone)]
pub struct HomeNewsPost {
    pub title: String,
    pub href: Option<String>,
    pub meta: String,
    pub summary_html: String,
}

#[derive(Debug, Clone)]
pub struct HomeReading {
    pub work: LiveWork,
    pub meta: String,
}

#[derive(Debug, Clone)]
pub struct HomeMessage {
    pub byline: String,
    pub content_html: String,
    pub href: Option<String>,
}

pub fn parse_home(html: &str) -> HomeModules {
    let document = Html::parse_document(html);
    let mut modules = HomeModules::default();

    if let Ok(selector) = css("div.favorite.module ul li a") {
        modules.favorites = document
            .select(&selector)
            .map(|node| text(&node))
            .filter(|value| !value.is_empty())
            .collect();
    }

    if let Ok(selector) = css("div.latest.news.module li.post") {
        for node in document.select(&selector) {
            let title = select_one(&node, "h4.heading a")
                .map(|link| text(&link))
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let href = select_one(&node, "h4.heading a")
                .and_then(|link| link.value().attr("href").map(str::to_string));
            let meta = select_one(&node, "p.meta")
                .map(|node| text(&node))
                .unwrap_or_default();
            let summary_html = select_one(&node, "blockquote.userstuff")
                .map(|node| node.inner_html())
                .unwrap_or_default();
            modules.news.push(HomeNewsPost {
                title,
                href,
                meta,
                summary_html,
            });
        }
    }

    if let Ok(selector) = css("li.reading.work") {
        for node in document.select(&selector) {
            let Ok(work) = parse_work_element(&node) else {
                continue;
            };
            let meta = select_one(&node, "div.user.module h4.viewed")
                .map(|node| text(&node))
                .unwrap_or_default();
            modules.readings.push(HomeReading { work, meta });
        }
    }

    if let Ok(selector) = css("div.latest.messages.module li.unread") {
        for node in document.select(&selector) {
            let byline = select_one(&node, "h4.heading.byline")
                .map(|node| text(&node))
                .unwrap_or_default();
            if byline.is_empty() {
                continue;
            }
            let content_html = select_one(&node, "blockquote.userstuff")
                .map(|node| node.inner_html())
                .unwrap_or_default();
            let href = select_one(&node, "h4.heading.byline a[href*=\"/works/\"]")
                .and_then(|link| link.value().attr("href").map(str::to_string));
            modules.inbox.push(HomeMessage {
                byline,
                content_html,
                href,
            });
        }
    }

    modules
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
    fn parses_all_home_modules() {
        let html = include_str!("../../tests/fixtures/ao3_home.html");
        let modules = parse_home(html);
        assert_eq!(
            modules.favorites,
            ["Assertive Emilia (Re:Zero)", "Natsuki Subaru/Rem"]
        );
        assert_eq!(modules.news.len(), 1);
        assert_eq!(modules.news[0].title, "Five Things Hobgirl Said");
        assert!(modules.news[0].meta.contains("Comments:"));
        assert!(modules.news[0].summary_html.contains("Every month"));
        assert_eq!(modules.readings.len(), 1);
        assert_eq!(modules.readings[0].work.ao3_id, "7737400");
        assert!(modules.readings[0].meta.contains("Last visited"));
        assert_eq!(modules.inbox.len(), 1);
        assert!(modules.inbox[0].byline.contains("Internet123"));
        assert_eq!(
            modules.inbox[0].content_html,
            "<p>Hungry man thinks of bread</p>"
        );
        assert_eq!(
            modules.inbox[0].href.as_deref(),
            Some("/works/12345/chapters/1")
        );
    }
}
