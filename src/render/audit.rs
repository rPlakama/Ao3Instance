//! Two-step write confirmation, the audit log and the error page.

use super::chrome::layout;
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
        "<h2>{}</h2><p>{}</p><form class=\"stack\" method=\"post\" action=\"{}\"\
         <p class=\"meta\">This sends one request to archiveofourown.org and is recorded \
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
    body.push_str("<button type=\"submit\">Confirm</button></form><p><a href=\"/\">Cancel</a></p>");
    layout("Confirm — AO3Instance", &body)
}

/// Recent write actions, newest first. No secrets: the ledger stores targets and
/// outcomes, never credentials or payloads.
pub fn audit_page(entries: &[crate::store::AuditEntry]) -> String {
    let mut body = String::from("<h2>Audit log</h2>");
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No writes recorded yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>when</th><th>action</th>\
             <th>target</th><th>outcome</th></tr></thead><tbody>",
        );
        for entry in entries {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&format_utc(entry.created_at)),
                escape(&entry.action),
                escape(&entry.target),
                escape(&entry.outcome),
            ));
        }
        body.push_str("</tbody></table>");
    }
    body.push_str("<p><a href=\"/\">Back to the start</a></p>");
    layout("Audit — AO3Instance", &body)
}

pub fn error_page(status: u16, message: &str) -> String {
    layout(
        "AO3Instance — error",
        &format!(
            "<h2>Something went wrong</h2><div class=\"error\">HTTP {status}: {}</div>\
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
        "<h2>Something went wrong</h2><div class=\"error\">HTTP {status}: AO3 refused the \
         request (HTTP {status}); it may be a bot challenge.</div>"
    );
    if solver_reachable && !work_id.is_empty() {
        body.push_str(
            "<div class=\"continue\"><strong>A local FlareSolverr is available.</strong> \
             It solves one page at a time in a real browser; this request is sent only when \
             you press the button.</div>",
        );
        body.push_str(&format!(
            "<form class=\"stack\" method=\"post\" action=\"/solve\">\
             <input type=\"hidden\" name=\"target\" value=\"{id}\">\
             <input type=\"hidden\" name=\"next\" value=\"/read/{id}\">",
            id = escape(work_id)
        ));
        if send_cookies_allowed {
            body.push_str(
                "<label><input type=\"checkbox\" name=\"send_cookies\" value=\"1\"> \
                 Send my AO3 session cookies to the local solver (needed for restricted or \
                 adult works)</label>",
            );
        }
        body.push_str("<button type=\"submit\">Solve &amp; cache this page</button></form>");
    } else {
        body.push_str(
            "<p class=\"meta\">A host-provided FlareSolverr can help when it is reachable; \
             check <a href=\"/diagnostics\">Diagnostics</a>.</p>",
        );
    }
    body.push_str(
        "<p><a href=\"/session#cookie-import\">Import fresh session cookies</a> · \
         <a href=\"/diagnostics\">Diagnostics</a> · <a href=\"/\">Back to the start</a></p>",
    );
    layout("AO3Instance — edge block", &body)
}
