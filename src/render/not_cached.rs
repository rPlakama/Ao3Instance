//! Offline miss for `GET /works/<id>`: offer the one-click cache action without
//! ever touching upstream from a `GET`.

use super::chrome::{PageContext, layout};
use super::escape;

pub fn not_cached_page(ctx: &PageContext, work_id: &str) -> String {
    let body = format!(
        "<h2 class=\"heading\">Not cached yet</h2>\
         <div class=\"flash notice\">No copy of work <code>{id}</code> is in the local cache. \
         Offline reads never call archiveofourown.org, so this page cannot fetch it \
         by itself.</div>\
         <form class=\"open\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{id}\">\
         <button type=\"submit\">Open work</button></form>\
         <p class=\"meta\">Opening fetches the work once (one user-initiated request) and \
         keeps it for offline reading. An AO3 session may be needed for restricted works; \
         see <a href=\"/session\">Session</a>.</p>",
        id = escape(work_id)
    );
    layout(ctx, &body)
}
