//! Shared page chrome: document skeleton, header/nav/footer and date labels.
//!
//! Pages pass a [`PageContext`] so every segment renders the same frame, the
//! same greeting and the same cached-count footer. The stylesheet is a static
//! asset (`/assets/ao3.css`), not inline, so it is cached once by the browser.

use super::escape;
use crate::time::{format_utc, now_secs};

/// How long before a cached copy is labelled stale.
const STALE_AFTER_SECS: i64 = 7 * 86_400;

/// Primary navigation segments. Only routes that exist are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavSection {
    Home,
    Fandoms,
    Browse,
    Search,
    About,
    Dashboard,
    Session,
    Audit,
    Diagnostics,
}

impl NavSection {
    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Fandoms => "Fandoms",
            Self::Browse => "Browse",
            Self::Search => "Search",
            Self::About => "About",
            Self::Dashboard => "Dashboard",
            Self::Session => "Session",
            Self::Audit => "Audit",
            Self::Diagnostics => "Diagnostics",
        }
    }

    fn href(self) -> &'static str {
        match self {
            Self::Home => "/",
            Self::Fandoms => "/fandoms",
            Self::Browse => "/works",
            Self::Search => "/search",
            Self::About => "/about",
            Self::Dashboard => "/dashboard",
            Self::Session => "/session",
            Self::Audit => "/audit",
            Self::Diagnostics => "/diagnostics",
        }
    }
}

/// Solver state the chrome may show. Never carries a cookie or clearance value.
#[derive(Debug, Clone, Default)]
pub struct SolverSummary {
    pub mode: String,
    pub reachable: bool,
    pub clearance_active: bool,
}

/// Everything a page needs that is not its own content.
#[derive(Debug, Clone)]
pub struct PageContext {
    pub title: String,
    pub active: NavSection,
    pub account: String,
    pub session_state: String,
    pub write_enabled: bool,
    pub cached_count: i64,
    pub solver: SolverSummary,
    /// `default` or `reversi`; rendered as `data-skin` on <html>.
    pub skin: String,
}

impl PageContext {
    pub fn new(title: impl Into<String>, active: NavSection) -> Self {
        Self {
            title: title.into(),
            active,
            account: String::new(),
            session_state: "logged-out".to_string(),
            write_enabled: false,
            cached_count: 0,
            solver: SolverSummary {
                mode: "auto".to_string(),
                ..SolverSummary::default()
            },
            skin: "default".to_string(),
        }
    }

    pub fn with_session(mut self, account: impl Into<String>, state: impl Into<String>) -> Self {
        self.account = account.into();
        self.session_state = state.into();
        self
    }

    pub fn with_write_enabled(mut self, enabled: bool) -> Self {
        self.write_enabled = enabled;
        self
    }

    pub fn with_cached_count(mut self, count: i64) -> Self {
        self.cached_count = count;
        self
    }

    pub fn with_solver(mut self, solver: SolverSummary) -> Self {
        self.solver = solver;
        self
    }

    pub fn with_skin(mut self, skin: impl Into<String>) -> Self {
        self.skin = skin.into();
        self
    }
}

fn greeting(ctx: &PageContext) -> String {
    if ctx.account.is_empty() {
        return "<span class=\"muted\">Not signed in</span> · \
                <details id=\"login\" class=\"dropdown\"><summary>Log In</summary>\
                <div class=\"dropdown-panel\"><form class=\"stack\" method=\"post\" \
                action=\"/session/login\">\
                <label>AO3 login or email <input type=\"text\" name=\"account\" required \
                autocomplete=\"username\"></label>\
                <label>Password <input type=\"password\" name=\"password\" required \
                autocomplete=\"current-password\"></label>\
                <label><input type=\"checkbox\" name=\"adult\" value=\"1\"> Allow adult                 content</label>\
                <button type=\"submit\">Log In</button></form>\
                <p class=\"meta\"><a href=\"/session#cookie-import\">Import cookies instead                 (recommended)</a></p></div></details>"
            .to_string();
    }
    format!(
        "<details class=\"dropdown\"><summary>Hi, {account}!</summary>\
         <div class=\"dropdown-panel\"><strong>{account}</strong> \
         <span class=\"state {state_class}\">{state}</span><ul>\
         <li><a href=\"/dashboard\">Dashboard</a></li>\
         <li><a href=\"/history\">History</a></li>\
         <li><a href=\"/audit\">Write audit</a></li>\
         <li><a href=\"/diagnostics\">Diagnostics</a></li>\
         <li><a href=\"/session\">Session &amp; preferences</a></li>\
         <li><a href=\"/users/logout/confirm\">Log out of AO3</a></li>\
         </ul></div></details>",
        account = escape(&ctx.account),
        state = escape(&ctx.session_state),
        state_class = if ctx.session_state == "active" {
            "active"
        } else {
            "imported"
        },
    )
}

fn menu_link(label: &str, href: &str, external: bool) -> String {
    let label = escape(label);
    if external {
        format!("<li><a href=\"{href}\" rel=\"noopener noreferrer\">{label}</a></li>")
    } else {
        format!("<li><a href=\"{href}\">{label}</a></li>")
    }
}

fn dropdown(section: NavSection, active: NavSection, items: &[(&str, &str, bool)]) -> String {
    let mut menu = String::from("<ul class=\"menu\">");
    for (label, href, external) in items {
        menu.push_str(&menu_link(label, href, *external));
    }
    menu.push_str("</ul>");
    format!(
        "<li class=\"dropdown{}\"><a href=\"{}\">{}</a>{menu}</li>",
        if section == active { " active" } else { "" },
        section.href(),
        section.label()
    )
}

fn navigation(ctx: &PageContext) -> String {
    let mut items = String::new();
    items.push_str(&dropdown(
        NavSection::Fandoms,
        ctx.active,
        &[
            ("All Fandoms", "/fandoms", false),
            (
                "Anime & Manga",
                "https://archiveofourown.org/media/Anime%20*a*%20Manga/fandoms",
                true,
            ),
            (
                "Books & Literature",
                "https://archiveofourown.org/media/Books%20*a*%20Literature/fandoms",
                true,
            ),
            (
                "Cartoons & Comics & Graphic Novels",
                "https://archiveofourown.org/media/Cartoons%20*a*%20Comics%20*a*%20Graphic%20Novels/fandoms",
                true,
            ),
            (
                "Celebrities & Real People",
                "https://archiveofourown.org/media/Celebrities%20*a*%20Real%20People/fandoms",
                true,
            ),
            ("Movies", "https://archiveofourown.org/media/Movies/fandoms", true),
            (
                "Music & Bands",
                "https://archiveofourown.org/media/Music%20*a*%20Bands/fandoms",
                true,
            ),
            ("Other Media", "https://archiveofourown.org/media/Other%20Media/fandoms", true),
            ("Theater", "https://archiveofourown.org/media/Theater/fandoms", true),
            ("TV Shows", "https://archiveofourown.org/media/TV%20Shows/fandoms", true),
            (
                "Video Games",
                "https://archiveofourown.org/media/Video%20Games/fandoms",
                true,
            ),
            (
                "Uncategorized Fandoms",
                "https://archiveofourown.org/media/Uncategorized%20Fandoms/fandoms",
                true,
            ),
        ],
    ));
    items.push_str(&dropdown(
        NavSection::Browse,
        ctx.active,
        &[
            ("Works", "/works", false),
            ("Bookmarks", "/bookmarks", false),
            ("Tags", "/tags/search", false),
            (
                "Collections",
                "https://archiveofourown.org/collections",
                true,
            ),
        ],
    ));
    items.push_str(&dropdown(
        NavSection::Search,
        ctx.active,
        &[
            ("Works", "/works/search", false),
            ("Bookmarks", "/bookmarks", false),
            ("Tags", "/tags/search", false),
            ("People", "https://archiveofourown.org/people/search", true),
        ],
    ));
    items.push_str(&dropdown(
        NavSection::About,
        ctx.active,
        &[
            ("About Us", "/about", false),
            ("News", "/#news", false),
            ("FAQ", "/about#faq", false),
            (
                "Wrangling Guidelines",
                "https://archiveofourown.org/wrangling_guidelines",
                true,
            ),
            ("Donate", "https://archiveofourown.org/donate", true),
        ],
    ));
    format!(
        "<ul class=\"primary navigation\">{items}\
         <li class=\"search\"><form class=\"search\" method=\"get\" action=\"/works/search\" \
         role=\"search\"><fieldset><p>\
         <label class=\"landmark\" for=\"site_search\">Search</label>\
         <input class=\"text\" id=\"site_search\" type=\"search\" name=\"q\" \
         aria-describedby=\"site_search_tooltip\">\
         <span class=\"tip\" role=\"tooltip\" id=\"site_search_tooltip\">tip: \
         <a href=\"/works/search?q=fluff%20sort%3Akudos\">fluff sort:kudos</a></span>\
         <span class=\"submit actions\"><button type=\"submit\">Search</button></span>\
         </p></fieldset></form></li></ul>"
    )
}

fn dashboard_nav(active: NavSection) -> String {
    let links = [
        (NavSection::Dashboard, "Dashboard", "/dashboard"),
        (NavSection::Browse, "Cached library", "/works"),
        (NavSection::Fandoms, "Fandoms", "/fandoms"),
        (NavSection::Session, "Session & preferences", "/session"),
        (NavSection::Audit, "Write audit", "/audit"),
        (NavSection::Diagnostics, "HTTP diagnostics", "/diagnostics"),
    ];
    let mut items = String::new();
    items.push_str(
        "<li><a href=\"/subscriptions\">Subscriptions</a></li>\
                    <li><a href=\"/bookmarks\">Bookmarks</a></li>\
                    <li><a href=\"/reading\">Marked for later</a></li>\
                    <li><a href=\"/history\">Local history</a></li>",
    );
    for (section, label, href) in links {
        let class = if section == active {
            " class=\"active\""
        } else {
            ""
        };
        items.push_str(&format!("<li{class}><a href=\"{href}\">{label}</a></li>"));
    }
    format!("<aside id=\"dashboard\"><h3>Dashboard</h3><ul>{items}</ul></aside>")
}

fn footer(ctx: &PageContext) -> String {
    let write_state = if ctx.write_enabled {
        "writes enabled"
    } else {
        "writes disabled"
    };
    let solver_state = if ctx.solver.reachable {
        "solver ready"
    } else {
        "solver not reachable"
    };
    format!(
        "<footer id=\"footer\"><div class=\"footer-inner\">\
         <div><h4>About</h4><ul>\
         <li><a href=\"/\">AO3Instance</a></li>\
         <li><a href=\"/session#cookie-import\">Cookie import</a></li>\
         <li><a href=\"/history\">Local history</a></li>\
         <li>Personal offline archive</li></ul></div>\
         <div><h4>Operator &amp; cache</h4><ul>\
         <li>{count} cached work(s)</li>\
         <li>{write_state}</li>\
         <li>mode <code>{mode}</code></li></ul></div>\
         <div><h4>Diagnostics</h4><ul>\
         <li><a href=\"/diagnostics\">HTTP diagnostics</a></li>\
         <li><a href=\"/audit\">Write audit</a></li>\
         <li><a href=\"/diagnostics\">{solver_state}</a></li></ul></div>\
         <div><h4>Display</h4><ul>\
         <li><form class=\"inline\" method=\"post\" action=\"/skin\">\
         <input type=\"hidden\" name=\"skin\" value=\"default\">\
         <button type=\"submit\">Classic</button></form>\
         <form class=\"inline\" method=\"post\" action=\"/skin\">\
         <input type=\"hidden\" name=\"skin\" value=\"reversi\">\
         <button type=\"submit\">Reversi</button></form></li>\
         <li><a href=\"/\">Back to the start</a></li>\
         <li>Content belongs to its authors</li></ul></div>\
         </div></footer>",
        count = ctx.cached_count,
        mode = escape(&ctx.solver.mode),
    )
}

/// The one document skeleton. `body` is already-escaped markup from a segment.
pub(super) fn layout(ctx: &PageContext, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\" data-skin=\"{skin}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><link rel=\"stylesheet\" href=\"/assets/ao3.css\"></head>\
         <body><header id=\"header\"><div class=\"header-inner\">\
         <h1 class=\"heading\"><a href=\"/\">AO3Instance<sup>local</sup></a></h1>\
         <div id=\"greeting\">{greeting}</div></div>{nav}</header>\
         <div id=\"inner\">{sidebar}<main id=\"main\">{body}</main></div>{footer}</body></html>",
        title = escape(&ctx.title),
        skin = escape(&ctx.skin),
        greeting = greeting(ctx),
        nav = navigation(ctx),
        sidebar = if ctx.account.is_empty() {
            String::new()
        } else {
            dashboard_nav(ctx.active)
        },
        footer = footer(ctx),
    )
}

pub(super) fn checked_label(fetched_at: i64) -> String {
    let age = now_secs().saturating_sub(fetched_at);
    if age > STALE_AFTER_SECS {
        format!(
            "checked {} ({} days ago — stale)",
            format_utc(fetched_at),
            age / 86_400
        )
    } else {
        format!("checked {}", format_utc(fetched_at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_label_flags_only_old_copies() {
        let fresh = checked_label(now_secs());
        assert!(fresh.contains("checked "), "{fresh}");
        assert!(!fresh.contains("stale"), "{fresh}");

        let stale = checked_label(now_secs() - STALE_AFTER_SECS - 86_400);
        assert!(stale.contains("stale"), "{stale}");
        assert!(stale.contains("8 days ago"), "{stale}");
    }

    #[test]
    fn layout_renders_nav_active_state_and_escapes_the_title() {
        let mut ctx = PageContext::new("A <title>", NavSection::About);
        ctx.account = "reader<b>".into();
        let html = layout(&ctx, "<p>body</p>");
        assert!(html.contains("<title>A &lt;title&gt;</title>"), "{html}");
        assert!(html.contains("class=\"dropdown active\""), "{html}");
        assert!(html.contains("<a href=\"/about\">About</a>"), "{html}");
        assert!(html.contains("Hi, reader&lt;b&gt;!"), "{html}");
        assert!(html.contains("/assets/ao3.css"), "{html}");
        assert!(html.contains("<p>body</p>"), "{html}");
    }

    #[test]
    fn signed_out_greeting_offers_the_login_dropdown() {
        let html = layout(&PageContext::new("t", NavSection::Home), "");
        assert!(html.contains("Not signed in"), "{html}");
        assert!(html.contains("id=\"login\""), "{html}");
        assert!(html.contains("action=\"/session/login\""), "{html}");
        assert!(html.contains("Import cookies instead"), "{html}");
    }
}
