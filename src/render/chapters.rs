//! Chapter manager, editor and confirmation pages.
//!
//! Editor layout and fields mirror `otwarchive/app/views/chapters/_chapter_form.html.erb`
//! and `_posting_fieldset.html.erb`; the chapter index mirrors
//! `app/views/works/navigate.html.erb` (`ol.chapter.index.group`).
//!
//! The confirmation page re-submits the chapter as hidden fields (long text in
//! hidden textareas, which browsers still submit) so the write pipeline stays
//! stateless: edit -> local preview -> one request to AO3.

use crate::ao3::chapters::{ChapterFields, ChapterIndex};
use crate::ao3::compose::{CONTENT_MIN, NOTES_MAX, SUMMARY_MAX, TITLE_MAX};

use super::chrome::{PageContext, layout_with_class};
use super::components::{cache_banner, flash_error, flash_notice};
use super::escape;

/// `GET /my/works/<id>/chapters` — the author's chapter list.
pub fn chapter_manager_page(
    ctx: &PageContext,
    work_id: &str,
    index: &ChapterIndex,
    cache_meta: Option<(bool, i64)>,
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let work_id = escape(work_id);
    let refresh_href = format!("/my/works/{work_id}/chapters?refresh=1");
    let mut body = format!(
        "<h2 class=\"heading\">Chapters: <a href=\"/open/{work_id}\">{}</a></h2>",
        escape(&index.work_title)
    );
    body.push_str(&flash_notice(notice));
    body.push_str(&flash_error(error));
    body.push_str(&format!(
        "<ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/read/{work_id}\">Open the work</a></li>\
         <li><a href=\"/my/works\">My Works</a></li>{add}</ul>",
        add = if ctx.write_enabled {
            format!("<li><a href=\"/write/chapter/new?work_id={work_id}\">Add Chapter</a></li>")
        } else {
            String::new()
        },
    ));
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));
    if index.chapters.is_empty() {
        body.push_str("<p class=\"muted\">AO3 returned no chapters for this work.</p>");
    } else {
        body.push_str(
            "<h3 class=\"landmark heading\">Chapter Index</h3>\
             <ol class=\"chapter index group\" role=\"navigation\">",
        );
        for (position, chapter) in index.chapters.iter().enumerate() {
            let chapter_id = escape(&chapter.chapter_id);
            body.push_str(&format!(
                "<li><a href=\"/read/{work_id}?chapter={position}\">{title}</a> \
                 <span class=\"datetime\">{date}</span>{edit}</li>",
                title = escape(&chapter.title),
                date = chapter
                    .posted_at
                    .as_deref()
                    .map_or_else(String::new, escape),
                edit = if ctx.write_enabled {
                    format!(
                        " <a class=\"action\" href=\"/write/chapter/edit?work_id={work_id}&amp;\
                         chapter_id={chapter_id}\">Edit</a>"
                    )
                } else {
                    String::new()
                },
            ));
        }
        body.push_str("</ol>");
    }
    layout_with_class(ctx, "works-index", &body)
}

/// Chapter editor (`chapters/new.html.erb` / `chapters/edit.html.erb`).
#[allow(
    clippy::too_many_arguments,
    reason = "server-rendered form with independent state fields"
)]
pub fn chapter_editor_page(
    ctx: &PageContext,
    work_id: &str,
    chapter_id: Option<&str>,
    work_title: &str,
    default_position: usize,
    fields: &ChapterFields,
    errors: &[String],
    notice: Option<&str>,
) -> String {
    let work_id = escape(work_id);
    let editing = chapter_id.is_some();
    let action = if editing {
        format!(
            "/write/chapter/preview?work_id={work_id}&amp;chapter_id={}",
            escape(chapter_id.unwrap_or_default())
        )
    } else {
        format!("/write/chapter/preview?work_id={work_id}")
    };
    let mut body = format!(
        "<h2 class=\"heading\">{heading}</h2><h3 class=\"heading\">{work}</h3>",
        heading = if editing {
            "Edit Chapter"
        } else {
            "Post New Chapter"
        },
        work = escape(work_title)
    );
    body.push_str(&flash_notice(notice));
    if !errors.is_empty() {
        body.push_str(
            "<div class=\"flash error\"><h4>Sorry! We couldn't save this chapter because:</h4><ul>",
        );
        for error in errors {
            body.push_str(&format!("<li>{}</li>", escape(error)));
        }
        body.push_str("</ul></div>");
    }
    let position = if fields.position.trim().is_empty() {
        default_position.to_string()
    } else {
        fields.position.clone()
    };
    body.push_str(&format!(
        "<p class=\"required notice\">* Required information</p>\
         <div id=\"chapter-form\" class=\"verbose post work chapter\">\
         <form method=\"post\" action=\"{action}\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{work_id}\">{chapter_hidden}\
         <fieldset id=\"chapter-ordering\"><legend>Name, Order and Date</legend><dl>\
         <dt><label for=\"chapter_title\">Chapter Title</label></dt>\
         <dd><input type=\"text\" id=\"chapter_title\" name=\"title\" maxlength=\"{title_max}\" \
         value=\"{title}\"></dd>\
         <dt><label for=\"chapter_position\">Chapter Number</label></dt>\
         <dd><p><input type=\"text\" inputmode=\"numeric\" class=\"number\" id=\"chapter_position\" \
         name=\"position\" value=\"{position}\"> <label for=\"chapter_wip_length\">of</label> \
         <input type=\"text\" class=\"number\" id=\"chapter_wip_length\" \
         name=\"wip_length\" value=\"{wip}\"></p>\
         <p class=\"footnote\">Leave \"of\" empty while the total chapter count is unknown.</p></dd>\
         </dl></fieldset>\
         <fieldset><legend>Chapter Preface</legend><dl>{summary}{notes}{endnotes}</dl></fieldset>\
         <fieldset class=\"text\"><legend class=\"required\">Chapter Text *</legend>\
         <textarea id=\"content\" name=\"content\" rows=\"20\">{content}</textarea>\
         <p class=\"footnote\">HTML from AO3's editor is supported; at least {content_min} \
         characters are required.</p></fieldset>\
         <fieldset><legend>Post</legend><ul class=\"actions\">{draft_button}\
         <li><button type=\"submit\" name=\"action\" value=\"preview\">{preview_label}</button></li>\
         <li><a href=\"/my/works/{work_id}/chapters\">Cancel</a></li></ul></fieldset>\
         </form></div>",
        action = action,
        work_id = work_id,
        chapter_hidden = chapter_id.map_or_else(String::new, |chapter_id| format!(
            "<input type=\"hidden\" name=\"chapter_id\" value=\"{}\">",
            escape(chapter_id)
        )),
        title_max = TITLE_MAX,
        title = escape(fields.title.trim()),
        position = escape(position.trim()),
        wip = escape(fields.wip_length.trim()),
        summary = textarea("Chapter Summary", "summary", &fields.summary, SUMMARY_MAX, 4),
        notes = textarea("Chapter Notes", "notes", &fields.notes, NOTES_MAX, 3),
        endnotes = textarea("End Notes", "endnotes", &fields.endnotes, NOTES_MAX, 3),
        content = escape(&fields.content),
        content_min = CONTENT_MIN,
        draft_button = if editing {
            String::new()
        } else {
            "<li><button type=\"submit\" name=\"action\" value=\"draft\">Save as Draft</button></li>"
                .to_string()
        },
        preview_label = if editing { "Preview & Update" } else { "Preview & Post" },
    ));
    layout_with_class(ctx, "works-index", &body)
}

/// Confirmation page before the one real chapter request.
pub fn chapter_confirm_page(
    ctx: &PageContext,
    work_id: &str,
    chapter_id: Option<&str>,
    work_title: &str,
    fields: &ChapterFields,
    action: &str,
) -> String {
    let updating = chapter_id.is_some();
    let action_label = if updating {
        "Confirm Chapter Update"
    } else if action == "draft" {
        "Confirm Chapter Draft"
    } else {
        "Confirm Chapter Post"
    };
    let description = if updating {
        format!(
            "Update “{}” on AO3? It replaces the current chapter text and notes.",
            escape(work_title)
        )
    } else if action == "draft" {
        format!(
            "Save this chapter as an unposted draft on “{}”?",
            escape(work_title)
        )
    } else {
        format!(
            "Post this chapter publicly on “{}”? It will be visible immediately \
             (moderated collections may hold it).",
            escape(work_title)
        )
    };
    let words = fields.content.split_whitespace().count();
    let body = format!(
        "<h2 class=\"heading\">{action_label}</h2><p class=\"note\">{description}</p>\
         <div class=\"wrapper\"><dl class=\"work meta group confirm-work\">\
         <dt>Chapter Title:</dt><dd>{title}</dd>\
         <dt>Chapter Number:</dt><dd>{position}{wip}</dd>\
         <dt>Words:</dt><dd>{words}</dd>\
         <dt>Chapter Summary:</dt><dd>{summary}</dd>\
         <dt>Chapter Notes:</dt><dd>{notes}</dd>\
         <dt>End Notes:</dt><dd>{endnotes}</dd>\
         <dt>Chapter Text:</dt><dd><pre class=\"preview\">{content}</pre></dd></dl></div>\
         <form class=\"verbose\" method=\"post\" action=\"/write/chapter\">\
         <fieldset><legend>{action_label}</legend>\
         <p class=\"note\">This sends exactly one request to archiveofourown.org and is \
         recorded in the audit log.</p>\
         <input type=\"hidden\" name=\"work_id\" value=\"{work_id_attr}\">{chapter_hidden}\
         <input type=\"hidden\" name=\"title\" value=\"{title_attr}\">\
         <input type=\"hidden\" name=\"position\" value=\"{position_attr}\">\
         <input type=\"hidden\" name=\"wip_length\" value=\"{wip_attr}\">\
         <textarea class=\"hidden\" name=\"summary\">{summary_attr}</textarea>\
         <textarea class=\"hidden\" name=\"notes\">{notes_attr}</textarea>\
         <textarea class=\"hidden\" name=\"endnotes\">{endnotes_attr}</textarea>\
         <textarea class=\"hidden\" name=\"content\">{content_attr}</textarea>\
         <input type=\"hidden\" name=\"confirm\" value=\"yes\">\
         <p class=\"submit actions\">{buttons}\
         <a href=\"/my/works/{work_id_attr}/chapters\">Back to the chapter list</a></p>\
         </fieldset></form>",
        content = escape(fields.content.trim()),
        title = display(&fields.title),
        position = display(&fields.position),
        wip = if fields.wip_length.trim().is_empty() {
            String::new()
        } else {
            format!(" of {}", escape(fields.wip_length.trim()))
        },
        summary = display(&fields.summary),
        notes = display(&fields.notes),
        endnotes = display(&fields.endnotes),
        work_id_attr = escape(work_id),
        chapter_hidden = chapter_id.map_or_else(String::new, |chapter_id| format!(
            "<input type=\"hidden\" name=\"chapter_id\" value=\"{}\">",
            escape(chapter_id)
        )),
        title_attr = escape(fields.title.trim()),
        position_attr = escape(fields.position.trim()),
        wip_attr = escape(fields.wip_length.trim()),
        summary_attr = escape(&fields.summary),
        notes_attr = escape(&fields.notes),
        endnotes_attr = escape(&fields.endnotes),
        content_attr = escape(&fields.content),
        buttons = if updating {
            "<button type=\"submit\" name=\"action\" value=\"update\">Confirm &amp; Update</button>"
                .to_string()
        } else if action == "draft" {
            "<button type=\"submit\" name=\"action\" value=\"draft\">Confirm &amp; Save Draft</button>"
                .to_string()
        } else {
            "<button type=\"submit\" name=\"action\" value=\"post\">Confirm &amp; Post</button>"
                .to_string()
        },
    );
    layout_with_class(ctx, "works-index", &body)
}

fn display(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "—".to_string()
    } else {
        escape(trimmed)
    }
}

fn textarea(label: &str, name: &str, value: &str, max: usize, rows: u8) -> String {
    format!(
        "<dt class=\"{name}\"><label for=\"chapter_{name}\">{label}</label></dt>\
         <dd class=\"{name}\"><textarea id=\"chapter_{name}\" name=\"{name}\" rows=\"{rows}\" \
         maxlength=\"{max}\">{value}</textarea></dd>",
        name = name,
        label = escape(label),
        rows = rows,
        max = max,
        value = escape(value),
    )
}
