//! Two-step write confirmation, the audit log and the error page.

use super::chrome::{NavSection, PageContext, layout};
use super::escape;
use crate::time::format_utc;

/// Two-step confirmation page for a write action.
pub fn confirm_page(
    title: &str,
    description: &str,
    action: &str,
    hidden: &[(&str, &str)],
    detail: Option<&str>,
) -> String {
    let mut body = format!(
        "<h2 class=\"heading\">{}</h2><p>{}</p>\
         <form class=\"verbose\" method=\"post\" action=\"{}\"><fieldset>\
         <legend>Confirm Action</legend>\
         <p class=\"note\">This sends one request to archiveofourown.org and is recorded \
         in the audit log.</p>",
        escape(title),
        escape(description),
        escape(action)
    );
    if let Some(detail) = detail {
        body.push_str(&format!("<pre class=\"preview\">{}</pre>", escape(detail)));
    }
    for (name, value) in hidden {
        body.push_str(&format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            escape(name),
            escape(value)
        ));
    }
    body.push_str(
        "<p class=\"submit actions\"><button type=\"submit\">Confirm</button> \
         <a href=\"/\">Cancel</a></p></fieldset></form>",
    );
    layout(
        &PageContext::new("Confirm — AO3Instance", NavSection::Home),
        &body,
    )
}

/// Recent write actions, newest first. No secrets: the ledger stores targets and
/// outcomes, never credentials or payloads.
pub fn audit_page(
    ctx: &PageContext,
    entries: &[crate::store::AuditEntry],
    cached_works: &std::collections::HashSet<String>,
) -> String {
    let mut body = String::from("<h2 class=\"heading\">Audit log</h2>");
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No writes recorded yet.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>When</th><th>Action</th>\
             <th>Target</th><th>Outcome</th></tr></thead><tbody>",
        );
        for entry in entries {
            let target = entry
                .target
                .strip_prefix("Work ")
                .filter(|id| cached_works.contains(*id))
                .map_or_else(
                    || escape(&entry.target),
                    |id| format!("<a href=\"/read/{id}\">{}</a>", escape(&entry.target)),
                );
            body.push_str(&format!(
                "<tr><td class=\"datetime\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&format_utc(entry.created_at)),
                escape(&entry.action),
                target,
                escape(&entry.outcome),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    body.push_str("<p><a href=\"/\">Back to the start</a></p>");
    layout(ctx, &body)
}

pub fn error_page(status: u16, message: &str) -> String {
    layout(
        &PageContext::new("AO3Instance — error", NavSection::Home),
        &format!(
            "<h2 class=\"heading\">Something went wrong</h2>\
             <div class=\"flash error\">HTTP {status}: {}</div>\
             <p><a href=\"/\">Back to the start</a></p>",
            escape(message)
        ),
    )
}

/// Error page for an edge block, with the explicit solve affordance when a
/// host-provided solver is reachable. Still a 502: nothing is retried on its own.
pub fn challenge_error_page(
    status: u16,
    work_id: &str,
    solver_reachable: bool,
    send_cookies_allowed: bool,
) -> String {
    let mut body = format!(
        "<h2 class=\"heading\">Something went wrong</h2>\
         <div class=\"flash error\">HTTP {status}: AO3 refused the \
         request (HTTP {status}); it may be a bot challenge.</div>"
    );
    if solver_reachable && !work_id.is_empty() {
        body.push_str(
            "<div class=\"flash caution\"><strong>A local FlareSolverr is available.</strong> \
             It solves one page at a time in a real browser; this request is sent only when \
             you press the button.</div>",
        );
        body.push_str(&format!(
            "<form class=\"verbose\" method=\"post\" action=\"/solve\"><fieldset>\
             <legend>Challenge Fallback</legend>\
             <input type=\"hidden\" name=\"target\" value=\"{id}\">\
             <input type=\"hidden\" name=\"next\" value=\"/read/{id}\">",
            id = escape(work_id)
        ));
        if send_cookies_allowed {
            body.push_str(
                "<p><label class=\"check\"><input type=\"checkbox\" name=\"send_cookies\" value=\"1\"> \
                 Send my AO3 session cookies to the local solver (needed for restricted or \
                 adult works)</label></p>",
            );
        }
        body.push_str(
            "<p class=\"submit actions\"><button type=\"submit\">Solve &amp; cache this page</button></p>\
             </fieldset></form>",
        );
    } else {
        body.push_str(
            "<p class=\"meta\">A host-provided FlareSolverr can help when it is reachable; \
             check <a href=\"/diagnostics\">Diagnostics</a>.</p>",
        );
    }
    body.push_str(
        "<ul class=\"actions\">\
         <li><a href=\"/session#cookie-import\">Import fresh session cookies</a></li>\
         <li><a href=\"/diagnostics\">Diagnostics</a></li>\
         <li><a href=\"/\">Back to the start</a></li></ul>",
    );
    layout(
        &PageContext::new("AO3Instance — edge block", NavSection::Home),
        &body,
    )
}
