//! About this instance.

use super::chrome::{PageContext, layout};

pub fn about_page(ctx: &PageContext) -> String {
    let body = "<h2>About AO3Instance</h2>\
        <p>A local, single-operator reader and write client for archiveofourown.org. \
        Works are fetched only when you ask for them, cached in SQLite and then \
        served from this instance, including when the archive is unreachable.</p>\
        <p>The cache is personal: content belongs to its authors. Do not redistribute \
        cached works.</p>\
        <h3>What is local</h3>\
        <ul>\
        <li><a href=\"/works\">Cached works</a> and <a href=\"/fandoms\">fandoms</a></li>\
        <li><a href=\"/reading\">Reading positions and flags</a></li>\
        <li><a href=\"/audit\">Write audit</a> and <a href=\"/diagnostics\">HTTP diagnostics</a></li>\
        <li><a href=\"/session\">AO3 session</a> (encrypted cookie jar)</li>\
        </ul>\
        <h3>Privacy</h3>\
        <p>No analytics, no third-party embeds. Diagnostics redact cookies, CSRF \
        tokens and response bodies. The optional FlareSolverr fallback is a \
        host-provided service and never a build dependency.</p>";
    layout(ctx, body)
}
