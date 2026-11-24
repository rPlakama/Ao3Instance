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
    /// Theme name; rendered as `data-skin` on <html>.
    pub skin: String,
    /// Optional CSS imported from a public AO3 site skin.
    pub skin_css: Option<String>,
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
            skin_css: None,
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

    pub fn with_skin_css(mut self, css: Option<String>) -> Self {
        self.skin_css = css;
        self
    }
}

fn greeting(ctx: &PageContext) -> String {
    if ctx.account.is_empty() {
        return "<div id=\"login\" class=\"dropdown\">\
                <span class=\"muted\">Not signed in</span> \
                <details class=\"login-toggle\"><summary class=\"action\">Log In</summary>\
                <div id=\"small_login\" class=\"simple login dropdown-panel\">\
                <form method=\"post\" action=\"/session/login\"><dl>\
                <dt><label for=\"user_session_login_small\">AO3 login or email</label></dt>\
                <dd><input id=\"user_session_login_small\" type=\"text\" name=\"account\" required \
                autocomplete=\"username\"></dd>\
                <dt><label for=\"user_session_password_small\">Password</label></dt>\
                <dd><input id=\"user_session_password_small\" type=\"password\" name=\"password\" \
                required autocomplete=\"current-password\"></dd></dl>\
                <p class=\"remember\"><label><input type=\"checkbox\" name=\"adult\" value=\"1\"> \
                Allow adult content</label></p>\
                <p class=\"submit actions\"><button type=\"submit\" class=\"action\">Log In</button></p>\
                </form>\
                <ul class=\"footnote actions\"><li><a href=\"/session#cookie-import\">\
                Import cookies instead (recommended)</a></li></ul>\
                </div></details></div>"
            .to_string();
    }
    format!(
        "<nav id=\"greeting\" aria-label=\"User\"><ul class=\"user navigation actions\">\
         <li class=\"dropdown\"><a href=\"/dashboard\" data-dropdown-toggle \
         aria-haspopup=\"true\" aria-expanded=\"false\">Hi, {account}!</a>\
         <ul class=\"menu\">\
         <li class=\"account-status\"><strong>{account}</strong> \
         <span class=\"state {state_class}\">{state}</span></li>\
         <li><a href=\"/dashboard\">My Dashboard</a></li>\
         <li><a href=\"/subscriptions\">My Subscriptions</a></li>\
         <li><a href=\"/bookmarks\">My Bookmarks</a></li>\
         <li><a href=\"/reading\">My History</a></li>\
         <li><a href=\"/reading?show=to-read\">Marked for Later</a></li>\
         <li><a href=\"/history\">Local History</a></li>\
         <li><a href=\"/session\">My Preferences</a></li>\
         <li><a href=\"/themes\">Site Skins</a></li>\
         <li><a href=\"/audit\">Write Audit</a></li>\
         <li><a href=\"/diagnostics\">HTTP Diagnostics</a></li>\
         </ul></li>\
         <li><a href=\"/session\">Preferences</a></li>\
         <li><a href=\"/users/logout/confirm\">Log Out</a></li>\
         </ul></nav>",
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
        "<li class=\"dropdown{}\"><a href=\"{}\" data-dropdown-toggle \
         aria-haspopup=\"true\" aria-expanded=\"false\">{}</a>{menu}</li>",
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
            ("Anime & Manga", "/media/Anime%20*a*%20Manga/fandoms", false),
            (
                "Books & Literature",
                "/media/Books%20*a*%20Literature/fandoms",
                false,
            ),
            (
                "Cartoons & Comics & Graphic Novels",
                "/media/Cartoons%20*a*%20Comics%20*a*%20Graphic%20Novels/fandoms",
                false,
            ),
            (
                "Celebrities & Real People",
                "/media/Celebrities%20*a*%20Real%20People/fandoms",
                false,
            ),
            ("Movies", "/media/Movies/fandoms", false),
            ("Music & Bands", "/media/Music%20*a*%20Bands/fandoms", false),
            ("Other Media", "/media/Other%20Media/fandoms", false),
            ("Theater", "/media/Theater/fandoms", false),
            ("TV Shows", "/media/TV%20Shows/fandoms", false),
            ("Video Games", "/media/Video%20Games/fandoms", false),
            (
                "Uncategorized Fandoms",
                "/media/Uncategorized%20Fandoms/fandoms",
                false,
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
        "<nav aria-label=\"Site\"><ul class=\"primary navigation\">{items}\
         <li class=\"search\"><form class=\"search\" id=\"search\" method=\"get\" \
         action=\"/works/search\" role=\"search\" aria-label=\"Work Search\"><fieldset><p>\
         <label class=\"landmark\" for=\"site_search\">Search</label>\
         <input class=\"text\" id=\"site_search\" type=\"search\" name=\"q\" \
         aria-describedby=\"site_search_tooltip\">\
         <span class=\"tip\" role=\"tooltip\" id=\"site_search_tooltip\">tip: \
         <a href=\"/works/search?q=fluff%20sort%3Akudos\">fluff sort:kudos</a></span>\
         <span class=\"submit actions\"><button type=\"submit\" class=\"button\">Search</button></span>\
         </p></fieldset></form></li></ul></nav>"
    )
}

fn show_dashboard_sidebar(active: NavSection) -> bool {
    matches!(
        active,
        NavSection::Dashboard | NavSection::Session | NavSection::Audit | NavSection::Diagnostics
    )
}

fn dashboard_nav(active: NavSection) -> String {
    let nav_item = |section: Option<NavSection>, label: &str, href: &str| {
        if section == Some(active) {
            format!("<li><span class=\"current\">{label}</span></li>")
        } else {
            format!("<li><a href=\"{href}\">{label}</a></li>")
        }
    };
    let choices = [
        nav_item(Some(NavSection::Dashboard), "Dashboard", "/dashboard"),
        nav_item(
            Some(NavSection::Session),
            "Session &amp; preferences",
            "/session",
        ),
        nav_item(None, "Public AO3 skins", "/themes"),
    ]
    .concat();
    let library = [
        nav_item(Some(NavSection::Browse), "Cached library", "/works"),
        nav_item(Some(NavSection::Fandoms), "Fandoms", "/fandoms"),
        nav_item(None, "Bookmarks", "/bookmarks"),
    ]
    .concat();
    let reading = [
        nav_item(None, "Subscriptions", "/subscriptions"),
        nav_item(None, "Marked for later", "/reading"),
        nav_item(None, "Local history", "/history"),
    ]
    .concat();
    let operator = [
        nav_item(Some(NavSection::Audit), "Write audit", "/audit"),
        nav_item(
            Some(NavSection::Diagnostics),
            "HTTP diagnostics",
            "/diagnostics",
        ),
    ]
    .concat();
    format!(
        "<div id=\"dashboard\" class=\"region own\" role=\"navigation\" aria-label=\"Dashboard\">\
         <h4 class=\"landmark heading\">Choices</h4>\
         <ul class=\"navigation actions\">{choices}</ul>\
         <h4 class=\"landmark heading\">Library</h4>\
         <ul class=\"navigation actions\">{library}</ul>\
         <h4 class=\"landmark heading\">Reading</h4>\
         <ul class=\"navigation actions\">{reading}</ul>\
         <h4 class=\"landmark heading\">Operator</h4>\
         <ul class=\"navigation actions\">{operator}</ul>\
         </div>"
    )
}

fn footer_skin_button(skin: &str, label: &str) -> String {
    format!(
        "<li><form method=\"post\" action=\"/skin\">\
         <input type=\"hidden\" name=\"skin\" value=\"{skin}\">\
         <button type=\"submit\">{label}</button></form></li>"
    )
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
    let skins = [
        footer_skin_button("default", "Default (Classic)"),
        footer_skin_button("auto", "Auto"),
        footer_skin_button("reversi", "Reversi"),
        footer_skin_button("snow", "Snow"),
        footer_skin_button("sepia", "Sepia"),
        footer_skin_button("midnight", "Midnight"),
    ]
    .concat();
    format!(
        "<footer id=\"footer\" role=\"contentinfo\" class=\"region\">\
         <h3 class=\"landmark heading\">Footer</h3>\
         <ul class=\"navigation actions\">\
         <li class=\"module group\"><h4 class=\"heading\">Customize</h4><ul class=\"menu\">\
         {skins}\
         <li><a href=\"/session#appearance\">Theme settings</a></li>\
         <li><a href=\"/themes\">Public AO3 skins</a></li></ul></li>\
         <li class=\"module group\"><h4 class=\"heading\">About</h4><ul class=\"menu\">\
         <li><a href=\"/\">AO3Instance</a></li>\
         <li><a href=\"/about\">About Us</a></li>\
         <li><a href=\"/about#faq\">FAQ</a></li>\
         <li><a href=\"/session#cookie-import\">Cookie import</a></li>\
         <li><a href=\"/history\">Local history</a></li>\
         <li>Personal offline archive</li></ul></li>\
         <li class=\"module group\"><h4 class=\"heading\">Operator &amp; cache</h4><ul class=\"menu\">\
         <li>{count} cached work(s)</li>\
         <li>{write_state}</li>\
         <li>mode <code>{mode}</code></li>\
         <li><a href=\"/\">Back to the start</a></li></ul></li>\
         <li class=\"module group\"><h4 class=\"heading\">Diagnostics</h4><ul class=\"menu\">\
         <li><a href=\"/diagnostics\">HTTP diagnostics</a></li>\
         <li><a href=\"/audit\">Write audit</a></li>\
         <li><a href=\"/diagnostics\">{solver_state}</a></li>\
         <li>Content belongs to its authors</li></ul></li>\
         </ul></footer>",
        count = ctx.cached_count,
        mode = escape(&ctx.solver.mode),
    )
}

/// The one document skeleton. `body` is already-escaped markup from a segment.
pub(super) fn layout(ctx: &PageContext, body: &str) -> String {
    let main_class = match ctx.active {
        NavSection::Home => "home",
        NavSection::Fandoms => "fandoms-index",
        NavSection::Browse => "works-index",
        NavSection::Search => "works-search",
        NavSection::About => "system docs",
        NavSection::Dashboard
        | NavSection::Session
        | NavSection::Audit
        | NavSection::Diagnostics => "dashboard",
    };
    layout_with_class(ctx, main_class, body)
}

/// Case-insensitively neutralises `</style` breakouts and `@import` rules in
/// imported public AO3 site-skin CSS before embedding inside `<style>`.
pub(crate) fn sanitize_public_skin_css(css: &str) -> String {
    let without_close = replace_ascii_case_insensitive(css, "</style", "");
    replace_ascii_case_insensitive(&without_close, "@import", "/* import removed */")
}

fn replace_ascii_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
    let needle_lower = needle.to_ascii_lowercase();
    let needle_bytes = needle_lower.as_bytes();
    if needle_bytes.is_empty() || input.len() < needle_bytes.len() {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut idx = 0;
    let mut last = 0;
    while idx + needle_bytes.len() <= bytes.len() {
        let matched = bytes[idx..idx + needle_bytes.len()]
            .iter()
            .zip(needle_bytes.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b);
        if matched
            && input.is_char_boundary(idx)
            && input.is_char_boundary(idx + needle_bytes.len())
        {
            out.push_str(&input[last..idx]);
            out.push_str(replacement);
            idx += needle_bytes.len();
            last = idx;
        } else {
            idx += 1;
        }
    }
    out.push_str(&input[last..]);
    out
}

/// Document skeleton with an explicit `#main` zone class list (matching AO3's
/// `classes_for_main`, e.g. `"home"`, `"works-show"`, `"works-index filtered"`).
pub(super) fn layout_with_class(ctx: &PageContext, main_class: &str, body: &str) -> String {
    let public_css = ctx.skin_css.as_deref().map_or_else(String::new, |css| {
        format!(
            "<style id=\"public-skin\">{}</style>",
            sanitize_public_skin_css(css)
        )
    });
    let body_class = if ctx.account.is_empty() {
        "logged-out"
    } else {
        "logged-in"
    };
    let has_sidebar = show_dashboard_sidebar(ctx.active);
    let main_classes = if has_sidebar && !main_class.split_whitespace().any(|c| c == "dashboard") {
        format!("dashboard {main_class}")
    } else {
        main_class.to_string()
    };
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\" data-skin=\"{skin}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title>\
         <link rel=\"stylesheet\" href=\"/assets/ao3.css?v={css_v:016x}\">{public_css}</head>\
         <body class=\"{body_class}\"><div id=\"outer\" class=\"wrapper\">\
         <ul id=\"skiplinks\"><li><a href=\"#main\">Main Content</a></li></ul>\
         <header id=\"header\" class=\"region\">\
         <div class=\"header-top\">\
         <h1 class=\"heading\"><a href=\"/\">\
         <svg class=\"logo\" viewBox=\"0 0 36 36\" width=\"30\" height=\"30\" aria-hidden=\"true\">\
         <path d=\"M6 8c4.5 0 8.5 1.8 12 4.8C21.5 9.8 25.5 8 30 8v19c-4.5 0-8.5 1.5-12 4.2-3.5-2.7-7.5-4.2-12-4.2V8z\" \
         fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linejoin=\"round\"/>\
         <path d=\"M18 12.8V31.2\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.2\"/></svg>\
         <span>AO3Instance</span><sup>local</sup></a></h1>\
         {greeting}</div>{nav}<div class=\"clear\"></div></header>\
         <div id=\"inner\" class=\"wrapper\">{sidebar}\
         <main id=\"main\" class=\"{main_classes} region\" role=\"main\">{body}\
         <div class=\"clear\"></div></main></div>{footer}</div>\
         <script src=\"/assets/ui.js?v={ui_v:016x}\" defer></script></body></html>",
        title = escape(&ctx.title),
        skin = escape(&ctx.skin),
        css_v = super::AO3_CSS_HASH,
        ui_v = super::UI_JS_HASH,
        greeting = greeting(ctx),
        nav = navigation(ctx),
        sidebar = if has_sidebar {
            dashboard_nav(ctx.active)
        } else {
            String::new()
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
        assert!(
            html.contains(
                "<a href=\"/about\" data-dropdown-toggle aria-haspopup=\"true\" aria-expanded=\"false\">About</a>"
            ),
            "{html}"
        );
        assert!(html.contains("Hi, reader&lt;b&gt;!"), "{html}");
        assert!(html.contains("/assets/ao3.css"), "{html}");
        assert!(html.contains("/assets/ui.js?v="), "{html}");
        assert!(html.contains("<p>body</p>"), "{html}");
    }

    #[test]
    fn public_skin_css_strips_style_breakouts_and_imports_case_insensitively() {
        let hostile = "body{color:red}</STYLE><script>alert(1)</script>@IMPORT url('https://evil.test/x.css');";
        let clean = sanitize_public_skin_css(hostile);
        assert!(!clean.to_ascii_lowercase().contains("</style"), "{clean}");
        assert!(!clean.to_ascii_lowercase().contains("@import"), "{clean}");
        assert!(clean.contains("/* import removed */"), "{clean}");
        assert!(clean.contains("body{color:red}"), "{clean}");
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
