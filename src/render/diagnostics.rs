//! Diagnostics page: recent HTTP failures and the solver's local state.

use super::chrome::layout;
use super::escape;
use crate::config::SolverMode;
use crate::store::{HttpErrorRow, SolverEventRow};
use crate::time::format_utc;

/// Everything the page renders, gathered by the handler.
pub struct DiagnosticsView<'a> {
    pub errors: &'a [HttpErrorRow],
    pub events: &'a [SolverEventRow],
    pub solver_mode: SolverMode,
    pub solver_reachable: bool,
    pub solver_version: Option<&'a str>,
    pub send_cookies_allowed: bool,
    pub clearance_expires_at: Option<i64>,
    pub solves_last_hour: i64,
}

pub fn diagnostics_page(view: &DiagnosticsView<'_>) -> String {
    let mut body = String::from("<h2>HTTP diagnostics</h2>");

    body.push_str("<h3>Local FlareSolverr</h3><p class=\"meta\">");
    body.push_str(&format!(
        "mode <code>{}</code> · endpoint <code>127.0.0.1</code> · status {}",
        view.solver_mode,
        if view.solver_reachable {
            "reachable"
        } else {
            "not reachable"
        }
    ));
    if let Some(version) = view.solver_version {
        body.push_str(&format!(" · version <code>{}</code>", escape(version)));
    }
    if view.solver_mode == SolverMode::Off {
        body.push_str(" · disabled by <code>AO3I_FLARESOLVERR=off</code>");
    }
    body.push_str("</p>");
    body.push_str(&format!(
        "<p class=\"meta\">solves in the last hour: {} · session cookies to solver: {} · \
         clearance: {}</p>",
        view.solves_last_hour,
        if view.send_cookies_allowed {
            "allowed"
        } else {
            "disabled by operator"
        },
        match view.clearance_expires_at {
            Some(expires_at) => format!(
                "active until {} (value never shown)",
                format_utc(expires_at)
            ),
            None => "none".to_string(),
        }
    ));
    body.push_str(
        "<form class=\"inline\" method=\"post\" action=\"/diagnostics/solver/probe\">\
         <button type=\"submit\">Probe now</button></form>\
         <form class=\"inline\" method=\"post\" action=\"/diagnostics/solver/clearance\">\
         <button type=\"submit\">Forget clearance</button></form>",
    );

    body.push_str("<h3>Recent HTTP failures</h3>");
    if view.errors.is_empty() {
        body.push_str("<p class=\"muted\">Nothing recorded yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>when</th><th>dir</th><th>via</th>\
             <th>method</th><th>path</th><th>status</th><th>kind</th><th>ms</th>\
             <th>server</th><th>message</th></tr></thead><tbody>",
        );
        for row in view.errors {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
                 <td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&format_utc(row.created_at)),
                escape(&row.direction),
                escape(&row.transport),
                escape(&row.method),
                escape(&row.path),
                row.status
                    .map_or_else(|| "-".to_string(), |status| status.to_string()),
                escape(&row.error_kind),
                row.latency_ms
                    .map_or_else(|| "-".to_string(), |ms| ms.to_string()),
                escape(row.server_header.as_deref().unwrap_or("-")),
                escape(&row.message),
            ));
        }
        body.push_str("</tbody></table>");
        body.push_str(
            "<form class=\"inline\" method=\"post\" action=\"/diagnostics/clear\">\
             <button type=\"submit\">Clear logs</button></form>",
        );
    }

    body.push_str("<h3>Recent solver outcomes</h3>");
    if view.events.is_empty() {
        body.push_str("<p class=\"muted\">No solve attempts yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>when</th><th>outcome</th>\
             <th>target</th><th>status</th><th>ms</th></tr></thead><tbody>",
        );
        for event in view.events {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&format_utc(event.created_at)),
                escape(&event.outcome),
                escape(&event.target_path),
                event
                    .status
                    .map_or_else(|| "-".to_string(), |status| status.to_string()),
                event
                    .latency_ms
                    .map_or_else(|| "-".to_string(), |ms| ms.to_string()),
            ));
        }
        body.push_str("</tbody></table>");
    }
    body.push_str("<p><a href=\"/\">Back to the start</a></p>");
    layout("Diagnostics — AO3Instance", &body)
}
