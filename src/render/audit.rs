//! Two-step write confirmation, the audit log and the error page.

use super::chrome::{format_utc, layout};
use super::escape;
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
