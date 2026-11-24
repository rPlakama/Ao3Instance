//! Parser for `GET /users/:login/subscriptions`.

use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::{collapse_text, extract_digits_after};
use crate::ao3::parse::percent_decode;

use super::parse_max_page;

/// One row from `GET /users/:login/subscriptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSubscription {
    pub subscription_id: Option<i64>,
    pub kind: String,
    pub target_id: String,
    pub title: String,
    pub href: Option<String>,
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveSubscriptionsPage {
    pub items: Vec<LiveSubscription>,
    pub total_pages: i64,
}

/// Parses `GET /users/:login/subscriptions`. Returns `None` if the HTML does
/// not look like an AO3 subscriptions page.
pub fn parse_subscriptions(html: &str) -> Option<LiveSubscriptionsPage> {
    if !html.contains("subscription index") && !html.contains("My Subscriptions") {
        return None;
    }
    let doc = Html::parse_document(html);
    let total_pages = parse_max_page(&doc);
    let Ok(dl_sel) = Selector::parse("dl.subscription.index.group") else {
        return Some(LiveSubscriptionsPage::default());
    };
    let Some(dl) = doc.select(&dl_sel).next() else {
        return Some(LiveSubscriptionsPage {
            items: Vec::new(),
            total_pages,
        });
    };

    let mut items = Vec::new();
    let mut pending: Option<LiveSubscription> = None;
    for child in dl.children() {
        let Some(el) = ElementRef::wrap(child) else {
            continue;
        };
        match el.value().name() {
            "dt" => {
                if let Some(prev) = pending.take() {
                    items.push(prev);
                }
                pending = parse_subscription_dt(&el);
            }
            "dd" => {
                if let Some(mut sub) = pending.take() {
                    sub.subscription_id = parse_subscription_dd_id(&el);
                    items.push(sub);
                }
            }
            _ => {}
        }
    }
    if let Some(last) = pending {
        items.push(last);
    }

    Some(LiveSubscriptionsPage { items, total_pages })
}

fn parse_subscription_dt(dt: &ElementRef<'_>) -> Option<LiveSubscription> {
    let link_sel = Selector::parse("a[href]").ok()?;
    let author_sel = Selector::parse("a[rel=author]").ok()?;
    let authors: Vec<String> = dt
        .select(&author_sel)
        .map(|node| collapse_text(&node))
        .filter(|name| !name.is_empty())
        .collect();

    if let Some(link) = dt.select(&link_sel).next() {
        let href = link.value().attr("href").unwrap_or_default().to_string();
        let title = collapse_text(&link);
        let (kind, target_id) = classify_subscription_href(&href);
        if title.is_empty() && target_id.is_empty() {
            return None;
        }
        return Some(LiveSubscription {
            subscription_id: None,
            kind,
            target_id,
            title,
            href: Some(href),
            authors,
        });
    }

    let raw = collapse_text(dt);
    if raw.is_empty() {
        return None;
    }
    let kind = if raw.contains("(Work)") {
        "Work"
    } else if raw.contains("(Series)") {
        "Series"
    } else {
        "Other"
    };
    Some(LiveSubscription {
        subscription_id: None,
        kind: kind.to_string(),
        target_id: String::new(),
        title: raw,
        href: None,
        authors,
    })
}

fn classify_subscription_href(href: &str) -> (String, String) {
    let clean = href.split(['?', '#']).next().unwrap_or(href);
    if clean.starts_with("/works/")
        && let Some(id) = extract_digits_after(clean, "/works/")
    {
        return ("Work".to_string(), id.to_string());
    }
    if clean.starts_with("/series/")
        && let Some(id) = extract_digits_after(clean, "/series/")
    {
        return ("Series".to_string(), id.to_string());
    }
    if let Some(rest) = clean.strip_prefix("/users/") {
        let seg = rest.split('/').next().unwrap_or_default();
        if !seg.is_empty() {
            let decoded = percent_decode(seg).unwrap_or_else(|| seg.to_string());
            return ("User".to_string(), decoded);
        }
    }
    ("Other".to_string(), clean.to_string())
}

fn parse_subscription_dd_id(dd: &ElementRef<'_>) -> Option<i64> {
    let form_sel = Selector::parse("form[action]").ok()?;
    let action = dd.select(&form_sel).next()?.value().attr("action")?;
    extract_digits_after(action, "/subscriptions/")?
        .parse()
        .ok()
}
