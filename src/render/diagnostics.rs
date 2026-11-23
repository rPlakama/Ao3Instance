//! Diagnostics page: recent HTTP failures and the solver's local state.

use std::collections::HashSet;

use super::chrome::{PageContext, layout};
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
    /// `None` = all, otherwise `upstream` or `inbound`.
    pub direction: Option<&'a str>,
    /// Cached works, so a logged `/works/<id>` path can link to the reader.
    pub cached_works: &'a HashSet<String>,
}

pub fn diagnostics_page(ctx: &PageContext, view: &DiagnosticsView<'_>) -> String {
    let mut body = String::from("<h2 class=\"heading\">HTTP diagnostics</h2>");

    body.push_str("<div class=\"listbox group\"><h3 class=\"heading\">Local FlareSolverr</h3>");
    body.push_str("<div class=\"index group\"><p class=\"meta\">");
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
        "</div><ul class=\"actions\">\
         <li><form class=\"inline\" method=\"post\" action=\"/diagnostics/solver/probe\">\
         <button type=\"submit\">Probe now</button></form></li>\
         <li><form class=\"inline\" method=\"post\" action=\"/diagnostics/solver/clearance\">\
         <button type=\"submit\">Forget clearance</button></form></li></ul></div>",
    );

    body.push_str(
        "<h3 class=\"heading\">Recent HTTP failures</h3>\
         <ul class=\"navigation actions\" role=\"navigation\">",
    );
    for (label, value) in [
        ("All", None),
        ("Upstream", Some("upstream")),
        ("Inbound", Some("inbound")),
    ] {
        let href = value.map_or_else(
            || "/diagnostics".to_string(),
            |value| format!("/diagnostics?direction={value}"),
        );
        if view.direction == value {
            body.push_str(&format!(
                "<li class=\"active\"><span class=\"current\">{label}</span></li>"
            ));
        } else {
            body.push_str(&format!("<li><a href=\"{href}\">{label}</a></li>"));
        }
    }
    body.push_str("</ul>");
    if view.errors.is_empty() {
        body.push_str("<p class=\"muted\">Nothing recorded yet.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>When</th><th>Dir</th>\
             <th>Via</th><th>Method</th><th>Path</th><th>Status</th><th>Kind</th><th>Ms</th>\
             <th>Server</th><th>Message</th></tr></thead><tbody>",
        );
        for row in view.errors {
            body.push_str(&format!(
                "<tr><td class=\"datetime\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
                 <td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&format_utc(row.created_at)),
                escape(&row.direction),
                escape(&row.transport),
                escape(&row.method),
                target_cell(&row.path, view.cached_works),
                row.status
                    .map_or_else(|| "-".to_string(), |status| status.to_string()),
                escape(&row.error_kind),
                row.latency_ms
                    .map_or_else(|| "-".to_string(), |ms| ms.to_string()),
                escape(row.server_header.as_deref().unwrap_or("-")),
                escape(&row.message),
            ));
        }
        body.push_str("</tbody></table></div>");
        body.push_str(
            "<ul class=\"actions\"><li><form class=\"inline\" method=\"post\" \
             action=\"/diagnostics/clear\">\
             <button type=\"submit\">Clear logs</button></form></li></ul>",
        );
    }

    body.push_str("<h3 class=\"heading\">Recent solver outcomes</h3>");
    if view.events.is_empty() {
        body.push_str("<p class=\"muted\">No solve attempts yet.</p>");
    } else {
        body.push_str(
            "<div class=\"wrapper\"><table class=\"audit\"><thead><tr><th>When</th>\
             <th>Outcome</th><th>Target</th><th>Status</th><th>Ms</th></tr></thead><tbody>",
        );
        for event in view.events {
            body.push_str(&format!(
                "<tr><td class=\"datetime\">{}</td><td>{}</td><td>{}</td><td>{}</td>\
                 <td>{}</td></tr>",
                escape(&format_utc(event.created_at)),
                escape(&event.outcome),
                target_cell(&event.target_path, view.cached_works),
                event
                    .status
                    .map_or_else(|| "-".to_string(), |status| status.to_string()),
                event
                    .latency_ms
                    .map_or_else(|| "-".to_string(), |ms| ms.to_string()),
            ));
        }
        body.push_str("</tbody></table></div>");
    }
    body.push_str("<p><a href=\"/\">Back to the start</a></p>");
    layout(ctx, &body)
}

/// A logged path is an entry: link cached work paths to the local reader and
/// leave everything else as text.
fn target_cell(path: &str, cached_works: &HashSet<String>) -> String {
    if let Some(rest) = path.strip_prefix("/works/") {
        let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() && cached_works.contains(&id) {
            return format!("<a href=\"/read/{id}\">{}</a>", escape(path));
        }
    }
    escape(path)
}
