//! Bookmark editor: notes, tags, collections and the private/rec flags.
//!
//! Mirrors `otwarchive/app/views/bookmarks/_bookmark_form.html.erb` (notes
//! textarea, "Your tags", collections, Private/Rec checkboxes). Confirmation and
//! the destructive delete page reuse `render::confirm_page`.

use crate::ao3::account_lists::BookmarkEditFields;
use crate::ao3::compose::{NOTES_MAX, TAG_MAX};

use super::chrome::{PageContext, layout_with_class};
use super::components::flash_notice;
use super::escape;

/// `GET /write/bookmark/edit` — the prefilled bookmark form.
pub fn bookmark_edit_page(
    ctx: &PageContext,
    bookmark_id: &str,
    work_id: &str,
    title: &str,
    fields: &BookmarkEditFields,
    errors: &[String],
    notice: Option<&str>,
) -> String {
    let bookmark_id = escape(bookmark_id);
    let work_id = escape(work_id);
    let mut body = format!(
        "<h2 class=\"heading\">Edit Bookmark</h2>\
         <p class=\"note\">Bookmark for “{}”. Notes, tags, collections and the \
         private/recommendation flags are changed on archiveofourown.org.</p>",
        escape(title)
    );
    body.push_str(&flash_notice(notice));
    if !errors.is_empty() {
        body.push_str(
            "<div class=\"flash error\"><h4>Sorry! We couldn't save this bookmark because:</h4><ul>",
        );
        for error in errors {
            body.push_str(&format!("<li>{}</li>", escape(error)));
        }
        body.push_str("</ul></div>");
    }
    body.push_str(&format!(
        "<div id=\"bookmark-form\" class=\"verbose post bookmark\">\
         <form method=\"post\" action=\"/write/bookmark/edit/preview\">\
         <input type=\"hidden\" name=\"bookmark_id\" value=\"{bookmark_id}\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{work_id}\">\
         <fieldset><legend>Write Comments</legend><dl>\
         <dt><label for=\"bookmark_notes\">Notes</label></dt>\
         <dd><textarea id=\"bookmark_notes\" name=\"notes\" rows=\"4\" maxlength=\"{notes_max}\">\
         {notes}</textarea></dd>\
         <dt><label for=\"bookmark_tags\">Your tags</label></dt>\
         <dd><input type=\"text\" id=\"bookmark_tags\" name=\"tags\" value=\"{tags}\"></dd>\
         <dt><label for=\"bookmark_collections\">Post to Collections / Challenges</label></dt>\
         <dd><input type=\"text\" id=\"bookmark_collections\" name=\"collections\" \
         value=\"{collections}\"></dd>\
         <dt>Private bookmark?</dt>\
         <dd><label><input type=\"checkbox\" name=\"private\" value=\"1\"{private}> Private</label></dd>\
         <dt>Rec?</dt>\
         <dd><label><input type=\"checkbox\" name=\"rec\" value=\"1\"{rec}> Recommendation</label>\
         <p class=\"footnote\">Tags are comma separated, at most {tag_max} characters each.</p>\
         </dd></dl></fieldset>\
         <fieldset><legend>Post</legend><p class=\"submit actions\">\
         <button type=\"submit\" name=\"action\" value=\"preview\">Preview &amp; Update</button>\
         <a href=\"/bookmarks\">Cancel</a></p></fieldset></form></div>",
        notes = escape(&fields.notes),
        tags = escape(&fields.tags),
        collections = escape(&fields.collections),
        private = if fields.private { " checked" } else { "" },
        rec = if fields.rec { " checked" } else { "" },
        notes_max = NOTES_MAX,
        tag_max = TAG_MAX,
    ));
    layout_with_class(ctx, "bookmarks-index", &body)
}
