//! AO3 session page: status, cookie import, password login, adult opt-in.

use super::chrome::layout;
use super::escape;

#[allow(
    clippy::too_many_arguments,
    reason = "server-rendered form with independent state fields"
)]
pub fn session_page(
    account: &str,
    state: &str,
    jar_complete: bool,
    adult_enabled: bool,
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h2>AO3 session</h2><p class=\"meta\">State: <span class=\"state {class}\">{state}</span> \
         {account}</p>",
        class = if state == "active" {
            "active"
        } else {
            "imported"
        },
        account = if account.is_empty() {
            String::new()
        } else {
            format!("· signed in as {}", crate::render::escape(account))
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
            "<form class=\"inline\" method=\"post\" action=\"/session/probe\">\
             <button type=\"submit\">Verify session now</button></form>\
             <form class=\"inline\" method=\"post\" action=\"/session/adult\">\
             <button type=\"submit\">Enable adult content</button></form>\
             <form class=\"inline\" method=\"post\" action=\"/session/logout\">\
             <button type=\"submit\">Log out of AO3</button></form>",
        );
    }
    if let Some(notice) = notice {
        body.push_str(&format!("<div class=\"notice\">{}</div>", escape(notice)));
    }
    if let Some(error) = error {
        body.push_str(&format!("<div class=\"error\">{}</div>", escape(error)));
    }

    body.push_str(
        "<h3>Import cookies (recommended: no password custody)</h3>\
         <p class=\"meta\">In your browser, sign in to archiveofourown.org, open DevTools → \
         Application → Cookies, and copy the values of <code>_otwarchive_session</code> and \
         <code>user_credentials</code>. They are stored encrypted (AES-256-GCM) with a key \
         only this server can read.</p>\
         <form class=\"stack\" method=\"post\" action=\"/session/import\">\
         <label>AO3 login <input type=\"text\" name=\"account\" required autocomplete=\"username\"></label>\
         <label>_otwarchive_session <input type=\"text\" name=\"session_cookie\" required></label>\
         <label>user_credentials <input type=\"text\" name=\"credentials_cookie\" required></label>\
         <label><input type=\"checkbox\" name=\"adult\" value=\"1\"> This account may read adult content</label>\
         <button type=\"submit\">Import session</button></form>",
    );
    body.push_str(
        "<h3>Sign in with a password</h3>\
         <p class=\"meta\">Optional. The password is encrypted at rest and used only to \
         renew the AO3 session; import is safer.</p>\
         <form class=\"stack\" method=\"post\" action=\"/session/login\">\
         <label>AO3 login or email <input type=\"text\" name=\"account\" required autocomplete=\"username\"></label>\
         <label>Password <input type=\"password\" name=\"password\" required autocomplete=\"current-password\"></label>\
         <label><input type=\"checkbox\" name=\"adult\" value=\"1\"> This account may read adult content</label>\
         <button type=\"submit\">Sign in</button></form>",
    );
    layout("Session — AO3Instance", &body)
}
