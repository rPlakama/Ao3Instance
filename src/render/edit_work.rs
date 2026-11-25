//! Editing an existing work's metadata and tags.
//!
//! Mirrors `otwarchive/app/views/works/edit.html.erb` rendering
//! `_standard_form.html.erb` (heading "Edit Work", `form#work-form`, same tag
//! and info fields as the composer). Options still come from AO3's own form.
//! The confirmation page re-submits every value as hidden fields so the write
//! pipeline stays stateless; chapter text is edited on AO3's chapter page (see
//! `render::chapters`), never here.

use crate::ao3::compose::{Choice, EditWorkFields, NewWorkDefaults};

use super::chrome::{PageContext, layout_with_class};
use super::components::flash_notice;
use super::escape;

/// The edit form for one existing work.
pub fn edit_work_page(
    ctx: &PageContext,
    defaults: &NewWorkDefaults,
    work_id: &str,
    fields: &EditWorkFields,
    errors: &[String],
    notice: Option<&str>,
) -> String {
    let work_id = escape(work_id);
    let mut body = String::from("<h2 class=\"heading\">Edit Work</h2>");
    body.push_str(&flash_notice(notice));
    if !errors.is_empty() {
        body.push_str(
            "<div class=\"flash error\"><h4>Sorry! We couldn't save this work because:</h4><ul>",
        );
        for error in errors {
            body.push_str(&format!("<li>{}</li>", escape(error)));
        }
        body.push_str("</ul></div>");
    }
    body.push_str(&format!(
        "<p class=\"required notice\">* Required information</p>\
         <p class=\"note\">This page changes the work's tags and metadata on \
         archiveofourown.org. Chapter text is edited from the chapter list. Previewing locally \
         costs nothing; only the final confirm button writes.</p>\
         <p><a href=\"/my/works/{work_id}/chapters\">Chapters &amp; chapter editing</a> · \
         <a href=\"/read/{work_id}\">Read the cached copy</a></p>\
         <form id=\"work-form\" class=\"verbose post work\" method=\"post\" \
         action=\"/write/work/edit/preview\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{work_id}\">"
    ));

    body.push_str(
        "<fieldset class=\"work meta\"><legend>Tags</legend><dl>\
         <dt class=\"rating required\"><label for=\"work_rating_string\">Rating *</label></dt>\
         <dd class=\"rating required\"><select id=\"work_rating_string\" name=\"rating\">",
    );
    body.push_str("<option value=\"\">Please select a rating</option>");
    for rating in &defaults.ratings {
        body.push_str(&option(rating, &fields.rating));
    }
    body.push_str("</select></dd>");

    body.push_str(
        "<dt class=\"warning required\"><label>Archive Warnings *</label></dt>\
         <dd class=\"warning required\"><fieldset class=\"warnings\"><ul>",
    );
    for warning in &defaults.warnings {
        body.push_str(&format!(
            "<li><label><input type=\"checkbox\" name=\"warnings\" value=\"{}\"{}> {}</label></li>",
            escape(&warning.value),
            if fields
                .warnings
                .iter()
                .any(|checked| checked == &warning.value)
            {
                " checked"
            } else {
                ""
            },
            escape(&warning.label)
        ));
    }
    body.push_str("</ul></fieldset></dd>");

    body.push_str(&text_row(
        "fandom",
        "Fandoms *",
        "work_fandom",
        &fields.fandom,
        "Comma separated, e.g. Original Work",
    ));
    body.push_str(
        "<dt class=\"category\"><label>Categories</label></dt><dd class=\"category\">\
         <fieldset><ul>",
    );
    for category in &defaults.categories {
        body.push_str(&format!(
            "<li><label><input type=\"checkbox\" name=\"categories\" value=\"{}\"{}> {}</label></li>",
            escape(&category.value),
            if fields.categories.iter().any(|checked| checked == &category.value) {
                " checked"
            } else {
                ""
            },
            escape(&category.label)
        ));
    }
    body.push_str("</ul></fieldset></dd>");
    body.push_str(&text_row(
        "relationships",
        "Relationships",
        "work_relationships",
        &fields.relationships,
        "Comma separated tag names",
    ));
    body.push_str(&text_row(
        "characters",
        "Characters",
        "work_characters",
        &fields.characters,
        "Comma separated tag names",
    ));
    body.push_str(&text_row(
        "additional_tags",
        "Additional Tags",
        "work_additional_tags",
        &fields.additional_tags,
        "Comma separated tag names",
    ));
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset class=\"work info\"><legend>Work Info</legend><dl>\
         <dt><label for=\"work_title\">Work Title *</label></dt>\
         <dd><input type=\"text\" id=\"work_title\" name=\"title\" maxlength=\"255\" value=\"",
    );
    body.push_str(&escape(fields.title.trim()));
    body.push_str("\"></dd>");
    body.push_str(&textarea(
        "Summary",
        "summary",
        &fields.summary,
        4,
        "work_summary",
    ));
    body.push_str(&textarea("Notes", "notes", &fields.notes, 3, "work_notes"));
    body.push_str(&textarea(
        "End Notes",
        "endnotes",
        &fields.endnotes,
        3,
        "work_endnotes",
    ));
    body.push_str("</dl></fieldset>");

    body.push_str(
        "<fieldset class=\"work associations\"><legend>Associations</legend><dl>\
         <dt class=\"language required\"><label for=\"work_language\">Language *</label></dt>\
         <dd class=\"language\"><select id=\"work_language\" name=\"language_id\">",
    );
    for language in &defaults.languages {
        body.push_str(&option(language, &fields.language_id));
    }
    body.push_str("</select></dd>");
    body.push_str(&text_row(
        "collections",
        "Post to Collections / Challenges",
        "work_collections",
        &fields.collections,
        "Comma separated collection names",
    ));
    body.push_str(&text_row(
        "recipients",
        "Gift this work to",
        "work_recipients",
        &fields.recipients,
        "Comma separated pseud bylines",
    ));
    body.push_str(
        "</dl></fieldset><fieldset><legend>Post</legend>\
         <p class=\"note\">Updates the work's tags and metadata; chapter text is not sent.</p>\
         <p class=\"submit actions\">\
         <button type=\"submit\" name=\"action\" value=\"preview\">Preview &amp; Update</button>\
         <a href=\"/my/works\">Cancel</a></p></fieldset></form>",
    );
    layout_with_class(ctx, "works-index", &body)
}

/// Confirmation page before the one PATCH request.
pub fn confirm_edit_page(ctx: &PageContext, work_id: &str, fields: &EditWorkFields) -> String {
    let work_id_attr = escape(work_id);
    let mut hidden = String::new();
    for (name, value) in [
        ("title", fields.title.as_str()),
        ("rating", fields.rating.as_str()),
        ("fandom", fields.fandom.as_str()),
        ("relationships", fields.relationships.as_str()),
        ("characters", fields.characters.as_str()),
        ("additional_tags", fields.additional_tags.as_str()),
        ("language_id", fields.language_id.as_str()),
        ("collections", fields.collections.as_str()),
        ("recipients", fields.recipients.as_str()),
    ] {
        hidden.push_str(&format!(
            "<input type=\"hidden\" name=\"{name}\" value=\"{}\">",
            escape(value)
        ));
    }
    for warning in &fields.warnings {
        hidden.push_str(&format!(
            "<input type=\"hidden\" name=\"warnings\" value=\"{}\">",
            escape(warning)
        ));
    }
    for category in &fields.categories {
        hidden.push_str(&format!(
            "<input type=\"hidden\" name=\"categories\" value=\"{}\">",
            escape(category)
        ));
    }
    for (name, value) in [
        ("summary", fields.summary.as_str()),
        ("notes", fields.notes.as_str()),
        ("endnotes", fields.endnotes.as_str()),
    ] {
        hidden.push_str(&format!(
            "<textarea class=\"hidden\" name=\"{name}\">{}</textarea>",
            escape(value)
        ));
    }
    let mut body = String::from(
        "<h2 class=\"heading\">Confirm Work Update</h2>\
         <p class=\"note\">Updating the metadata/tags of this work on \
         archiveofourown.org; chapter text is unchanged.</p>\
         <div class=\"wrapper\"><dl class=\"work meta group confirm-work\">",
    );
    body.push_str(&format!(
        "<dt>Title:</dt><dd>{}</dd>\
         <dt>Rating:</dt><dd>{}</dd>\
         <dt>Archive Warnings:</dt><dd>{}</dd>\
         <dt>Fandoms:</dt><dd>{}</dd>\
         <dt>Categories:</dt><dd>{}</dd>\
         <dt>Relationships:</dt><dd>{}</dd>\
         <dt>Characters:</dt><dd>{}</dd>\
         <dt>Additional Tags:</dt><dd>{}</dd>\
         <dt>Summary:</dt><dd>{}</dd>\
         <dt>Notes:</dt><dd>{}</dd>\
         <dt>End Notes:</dt><dd>{}</dd>\
         <dt>Collections:</dt><dd>{}</dd>\
         <dt>Gifts:</dt><dd>{}</dd></dl></div>",
        display(&fields.title),
        display(&fields.rating),
        list(&fields.warnings),
        display(&fields.fandom),
        list(&fields.categories),
        display(&fields.relationships),
        display(&fields.characters),
        display(&fields.additional_tags),
        display(&fields.summary),
        display(&fields.notes),
        display(&fields.endnotes),
        display(&fields.collections),
        display(&fields.recipients),
    ));
    body.push_str(&format!(
        "<form class=\"verbose\" method=\"post\" action=\"/write/work/edit\">\
         <fieldset><legend>Confirm Work Update</legend>\
         <p class=\"note\">This sends exactly one request to archiveofourown.org and is \
         recorded in the audit log.</p>\
         <input type=\"hidden\" name=\"work_id\" value=\"{work_id_attr}\">\
         <input type=\"hidden\" name=\"confirm\" value=\"yes\">{hidden}\
         <p class=\"submit actions\">\
         <button type=\"submit\">Confirm &amp; Update</button>\
         <a href=\"/write/work/edit?work_id={work_id_attr}\">Back to the editor</a></p>\
         </fieldset></form>"
    ));
    layout_with_class(ctx, "works-index", &body)
}

fn option(choice: &Choice, selected: &str) -> String {
    format!(
        "<option value=\"{}\"{}>{}</option>",
        escape(&choice.value),
        if choice.value == selected {
            " selected"
        } else {
            ""
        },
        escape(&choice.label)
    )
}

fn text_row(name: &str, label: &str, id: &str, value: &str, placeholder: &str) -> String {
    format!(
        "<dt class=\"{name}\"><label for=\"{id}\">{label}</label></dt>\
         <dd class=\"{name}\"><input type=\"text\" id=\"{id}\" name=\"{name}\" value=\"{}\" \
         placeholder=\"{}\"></dd>",
        escape(value),
        escape(placeholder),
        name = escape(name),
        label = escape(label),
        id = escape(id),
    )
}

fn textarea(label: &str, name: &str, value: &str, rows: u8, id: &str) -> String {
    format!(
        "<dt class=\"{name}\"><label for=\"{id}\">{label}</label></dt>\
         <dd class=\"{name}\"><textarea id=\"{id}\" name=\"{name}\" rows=\"{rows}\">{}</textarea></dd>",
        escape(value),
        name = escape(name),
        label = escape(label),
        id = escape(id),
        rows = rows,
    )
}

fn display(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "—".to_string()
    } else {
        escape(trimmed)
    }
}

fn list(values: &[String]) -> String {
    if values.is_empty() {
        "—".to_string()
    } else {
        values
            .iter()
            .map(|value| escape(value))
            .collect::<Vec<_>>()
            .join(", ")
    }
}
