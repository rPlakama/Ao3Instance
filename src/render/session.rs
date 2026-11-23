//! AO3 session page: status, appearance/skins, cookie import, password login,
//! adult opt-in — using AO3's `<fieldset><legend><dl>` form pattern.

use super::chrome::{PageContext, layout_with_class};
use super::escape;

#[allow(
    clippy::too_many_arguments,
    reason = "server-rendered form with independent state fields"
)]
pub fn session_page(
    ctx: &PageContext,
    account: &str,
    state: &str,
    jar_complete: bool,
    adult_enabled: bool,
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h2 class=\"heading\">AO3 session</h2>\
         <p class=\"note\">State: <span class=\"state {class}\">{state}</span> \
         {account}</p>",
        class = if state == "active" {
            "active"
        } else {
            "imported"
        },
        account = if account.is_empty() {
            String::new()
        } else {
            format!("· signed in as <strong>{}</strong>", escape(account))
        },
    ));
    if !account.is_empty() {
        body.push_str(&format!(
            "<p class=\"meta\">Cookie jar: {}. Adult content: {}.</p>",
            if jar_complete {
                "complete"
            } else {
                "incomplete (needs user_credentials)"
            },
            if adult_enabled {
                "enabled"
            } else {
                "not enabled"
            },
        ));
        body.push_str(
            "<ul class=\"navigation actions\">\
             <li><form class=\"inline\" method=\"post\" action=\"/session/probe\">\
             <button type=\"submit\">Verify session now</button></form></li>\
             <li><form class=\"inline\" method=\"post\" action=\"/session/adult\">\
             <button type=\"submit\">Enable adult content</button></form></li>\
             <li><a href=\"/users/logout/confirm\">Log out of AO3</a></li></ul>",
        );
    }
    if let Some(notice) = notice {
        body.push_str(&format!(
            "<div class=\"flash notice\">{}</div>",
            escape(notice)
        ));
    }
    if let Some(error) = error {
        body.push_str(&format!(
            "<div class=\"flash error\">{}</div>",
            escape(error)
        ));
    }

    if state == "blocked" {
        body.push_str(
            "<div class=\"flash caution\"><strong>AO3's edge refused the request.</strong> \
             Check <a href=\"/diagnostics\">Diagnostics</a>, verify the account in your browser, \
             then re-import fresh cookies below. If the host runs FlareSolverr, a blocked work \
             page offers a one-click solve.</div>",
        );
    }

    let selected = |value: &str| {
        if ctx.skin == value { " checked" } else { "" }
    };
    let themes = [
        ("auto", "Auto (follow the system light/dark preference)"),
        ("default", "Classic (light)"),
        ("reversi", "Reversi (dark)"),
        ("snow", "Snow (blue on white)"),
        ("frost", "Frost (cool blue)"),
        ("cloud", "Cloud (soft grey)"),
        ("sepia", "Sepia (warm paper)"),
        ("midnight", "Midnight (deep blue)"),
    ];
    let mut theme_inputs = String::new();
    for (value, label) in themes {
        theme_inputs.push_str(&format!(
            "<li><label class=\"check\"><input type=\"radio\" name=\"skin\" value=\"{value}\"{}> \
             {label}</label></li>",
            selected(value)
        ));
    }
    body.push_str(&format!(
        "<h3 id=\"appearance\" class=\"heading\">Appearance</h3>\
         <form class=\"verbose preferences\" method=\"post\" action=\"/skin\">\
         <fieldset><legend>Display &amp; Site Skin</legend>\
         <input type=\"hidden\" name=\"next\" value=\"/session#appearance\">\
         <p class=\"note\">Choose how this instance looks. The setting is stored locally. \
         You can also <a href=\"/themes\">browse public AO3 site skins</a> to import more themes.</p>\
         <ul class=\"choice-list\">{theme_inputs}</ul>\
         <p class=\"submit actions\"><button type=\"submit\">Save appearance</button></p>\
         </fieldset></form>"
    ));
    body.push_str(
        "<h3 id=\"cookie-import\" class=\"heading\">Import cookies (recommended: no password custody)</h3>\
         <form class=\"verbose\" method=\"post\" action=\"/session/import\">\
         <fieldset><legend>Import AO3 Cookies</legend>\
         <p class=\"note\">In your browser, sign in to archiveofourown.org, open DevTools → \
         Application → Cookies, and copy the values of <code>_otwarchive_session</code> and \
         <code>user_credentials</code>. They are stored encrypted (AES-256-GCM) with a key \
         only this server can read.</p>\
         <dl>\
         <dt><label for=\"import_account\">AO3 login</label></dt>\
         <dd><input id=\"import_account\" type=\"text\" name=\"account\" required autocomplete=\"username\"></dd>\
         <dt><label for=\"import_session\">_otwarchive_session</label></dt>\
         <dd><input id=\"import_session\" type=\"text\" name=\"session_cookie\" required></dd>\
         <dt><label for=\"import_creds\">user_credentials</label></dt>\
         <dd><input id=\"import_creds\" type=\"text\" name=\"credentials_cookie\" required></dd>\
         <dt>Adult content</dt>\
         <dd><label class=\"check\"><input type=\"checkbox\" name=\"adult\" value=\"1\"> \
         This account may read adult content</label></dd></dl>\
         <p class=\"submit actions\"><button type=\"submit\">Import session</button></p>\
         </fieldset></form>",
    );
    body.push_str(
        "<h3 id=\"login-form\" class=\"heading\">Sign in with a password</h3>\
         <form class=\"verbose\" method=\"post\" action=\"/session/login\">\
         <fieldset><legend>Password Sign In</legend>\
         <p class=\"note\">Optional. The password is encrypted at rest and used only to \
         renew the AO3 session; cookie import is safer.</p>\
         <dl>\
         <dt><label for=\"login_account\">AO3 login or email</label></dt>\
         <dd><input id=\"login_account\" type=\"text\" name=\"account\" required autocomplete=\"username\"></dd>\
         <dt><label for=\"login_password\">Password</label></dt>\
         <dd><input id=\"login_password\" type=\"password\" name=\"password\" required autocomplete=\"current-password\"></dd>\
         <dt>Adult content</dt>\
         <dd><label class=\"check\"><input type=\"checkbox\" name=\"adult\" value=\"1\"> \
         This account may read adult content</label></dd></dl>\
         <p class=\"submit actions\"><button type=\"submit\">Sign in</button></p>\
         </fieldset></form>",
    );
    layout_with_class(ctx, "dashboard session-preferences", &body)
}

/// AO3-style logout confirmation: the destructive action needs a second click.
pub fn logout_confirm_page(ctx: &PageContext, account: &str) -> String {
    let who = if account.is_empty() {
        "the local AO3 session".to_string()
    } else {
        format!("the session for <strong>{}</strong>", escape(account))
    };
    let body = format!(
        "<h2 class=\"heading\">Log out?</h2>\
         <div class=\"flash caution\">This removes {who} from this instance locally \
         without contacting AO3. Cached works and reading progress are not affected.</div>\
         <ul class=\"actions\">\
         <li><form class=\"inline\" method=\"post\" action=\"/session/logout\">\
         <button type=\"submit\">Remove local session</button></form></li>\
         <li><a href=\"/\">Cancel</a></li>\
         <li><a href=\"/session\">Back to session</a></li></ul>"
    );
    layout_with_class(ctx, "dashboard", &body)
}
