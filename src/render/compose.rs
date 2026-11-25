//! The AO3-shaped "New Work" composer and its confirmation step.
//!
//! Field order and grouping follow `otwarchive/app/views/works/new.html.erb`,
//! `_standard_form.html.erb`, `_work_form_tags.html.erb`,
//! `_work_form_associations_language.html.erb` and `_posting_fieldset.html.erb`.
//! Options (ratings, warnings, languages with their numeric ids, creator pseuds)
//! come from AO3's own `/works/new` page so the composer never guesses private
//! database ids.
//!
//! The confirmation page re-submits the same values as hidden fields (long text
//! in hidden textareas, which browsers still submit), so the write pipeline can
//! stay stateless: compose -> preview -> one POST to AO3.

use crate::ao3::compose::{NewWorkDefaults, NewWorkInput};

use super::chrome::{PageContext, layout_with_class};
use super::components::flash_notice;
use crate::time::format_count;

use super::escape;

/// AO3's new-work form (`#work-form.verbose.post.work`).
pub fn compose_page(
    ctx: &PageContext,
    defaults: &NewWorkDefaults,
    input: &NewWorkInput,
    errors: &[String],
    notice: Option<&str>,
) -> String {
    let mut body = String::from("<h2 class=\"heading\">New Work</h2>");
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
    body.push_str(
        "<p class=\"required notice\">* Required information</p>\
         <p class=\"note\">This composer posts directly to archiveofourown.org through your \
         session. Previewing locally costs nothing; only the final confirm button writes.</p>\
         <form id=\"work-form\" class=\"verbose post work\" method=\"post\" action=\"/write/work/preview\">",
    );

    // ---- Tags (`_work_form_tags.html.erb`) --------------------------------
    body.push_str(
        "<fieldset class=\"work meta\"><legend>Tags</legend>\
         <h3 class=\"landmark heading\">Tags</h3>\
         <p class=\"note\">Tags are comma separated. Fandom, relationship, character and \
         additional tags together may not exceed 75.</p><dl>\
         <dt class=\"rating required\"><label for=\"work_rating_string\">Rating *</label></dt>\
         <dd class=\"rating required\"><select id=\"work_rating_string\" name=\"rating\">",
    );
    body.push_str("<option value=\"\">Please select a rating</option>");
    for rating in &defaults.ratings {
        body.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(&rating.value),
            if input.rating == rating.value {
                " selected"
            } else {
                ""
            },
            escape(&rating.label)
        ));
    }
    body.push_str("</select></dd>");

    body.push_str(
        "<dt class=\"warning required\"><label>Archive Warnings *</label></dt>\
         <dd class=\"warning required\"><fieldset class=\"warnings\"><ul>",
    );
    if defaults.warnings.is_empty() {
        body.push_str("<li class=\"note\">The archive did not list warnings.</li>");
    }
    for warning in &defaults.warnings {
        body.push_str(&format!(
            "<li><label><input type=\"checkbox\" name=\"warnings\" value=\"{}\"{}> {}</label></li>",
            escape(&warning.value),
            if input
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
        &input.fandom,
        "Comma separated, e.g. Original Work",
        true,
    ));
    body.push_str(
        "<dt class=\"category\"><label>Categories</label></dt><dd class=\"category\">\
         <fieldset><ul>",
    );
    for category in &defaults.categories {
        body.push_str(&format!(
            "<li><label><input type=\"checkbox\" name=\"categories\" value=\"{}\"{}> {}</label></li>",
            escape(&category.value),
            if input.categories.iter().any(|checked| checked == &category.value) {
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
        "work_relationship",
        &input.relationships,
        "Comma separated tag names",
        false,
    ));
    body.push_str(&text_row(
        "characters",
        "Characters",
        "work_character",
        &input.characters,
        "Comma separated tag names",
        false,
    ));
    body.push_str(&text_row(
        "additional_tags",
        "Additional Tags",
        "work_freeform",
        &input.additional_tags,
        "Comma separated tag names",
        false,
    ));
    body.push_str("</dl></fieldset>");

    // ---- Work info (`_standard_form.html.erb` preface + byline) -----------
    body.push_str("<fieldset class=\"preface\"><legend>Work Info</legend><dl>");
    body.push_str(&text_row(
        "title",
        "Work Title *",
        "work_title",
        &input.title,
        "",
        true,
    ));
    body.push_str("<dt class=\"byline\"><label for=\"work_pseud_id\">Creator/Pseud *</label></dt><dd class=\"byline\">");
    if defaults.pseuds.len() > 1 {
        body.push_str("<select id=\"work_pseud_id\" name=\"pseud_id\">");
        for pseud in &defaults.pseuds {
            body.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                escape(&pseud.value),
                if input.pseud_id == pseud.value {
                    " selected"
                } else {
                    ""
                },
                escape(&pseud.label)
            ));
        }
        body.push_str("</select>");
    } else {
        let label = defaults.pseuds.first().map_or_else(
            || "your default pseud".to_string(),
            |pseud| pseud.label.clone(),
        );
        let value = if input.pseud_id.is_empty() {
            defaults
                .pseuds
                .first()
                .map(|pseud| pseud.value.clone())
                .unwrap_or_default()
        } else {
            input.pseud_id.clone()
        };
        body.push_str(&format!(
            "<input id=\"work_pseud_id\" type=\"hidden\" name=\"pseud_id\" value=\"{}\">\
             <span class=\"byline\">{label}</span>",
            escape(&value)
        ));
    }
    body.push_str("</dd>");
    body.push_str(&textarea_row(
        "summary",
        "Summary",
        "work_summary",
        &input.summary,
        4,
    ));
    body.push_str(&textarea_row(
        "notes",
        "Notes (at the beginning of your work)",
        "work_notes",
        &input.notes,
        3,
    ));
    body.push_str("</dl></fieldset>");

    // ---- Associations: language + collections + gifts ---------------------
    body.push_str(
        "<fieldset id=\"associations\"><legend>Associations</legend><dl>\
         <dt class=\"language required\"><label for=\"work_language_id\">Language *</label></dt>\
         <dd class=\"language required\"><select id=\"work_language_id\" name=\"language_id\">",
    );
    let selected_language = if input.language_id.is_empty() {
        defaults
            .default_language()
            .map(|choice| choice.value.clone())
            .unwrap_or_default()
    } else {
        input.language_id.clone()
    };
    for language in &defaults.languages {
        body.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(&language.value),
            if selected_language == language.value {
                " selected"
            } else {
                ""
            },
            escape(&language.label)
        ));
    }
    body.push_str("</select></dd>");
    body.push_str(&text_row(
        "collections",
        "Post to Collections / Challenges",
        "work_collection_names",
        &input.collections,
        "Comma separated collection names",
        false,
    ));
    body.push_str(&text_row(
        "recipients",
        "Gift this work to",
        "work_recipients",
        &input.recipients,
        "Comma separated pseud bylines",
        false,
    ));
    body.push_str("</dl></fieldset>");

    // ---- Work text --------------------------------------------------------
    body.push_str(&format!(
        "<fieldset class=\"work text\"><legend class=\"required\">Work Text *</legend><dl>\
         <dt><label for=\"chapter_title\">Chapter Title</label></dt>\
         <dd><input id=\"chapter_title\" type=\"text\" name=\"chapter_title\" value=\"{}\"></dd>\
         <dt class=\"required\"><label for=\"content\">Work Text *</label></dt>\
         <dd><textarea id=\"content\" name=\"content\" rows=\"20\">{}</textarea>\
         <p class=\"footnote\">HTML and rich formatting from AO3's own editor are supported; \
         at least 10 characters are required.</p></dd></dl></fieldset>",
        escape(&input.chapter_title),
        escape(&input.content)
    ));

    // ---- Posting fieldset (`_posting_fieldset.html.erb`) -------------------
    body.push_str(
        "<fieldset><legend>Post</legend><ul class=\"actions\">\
         <li><button type=\"submit\" name=\"action\" value=\"draft\">Save as Draft</button></li>\
         <li><button type=\"submit\" name=\"action\" value=\"publish\">Preview &amp; Post</button></li>\
         <li><a href=\"/\">Cancel</a></li></ul>\
         <p class=\"footnote\">Preview shows exactly what will be posted before anything is \
         sent to AO3. Drafts live on AO3 and are deleted by the archive after about 29 days.</p>\
         </fieldset></form>",
    );
    layout_with_class(ctx, "works-new", &body)
}

/// Confirmation page: everything that will be posted, then one real write.
pub fn confirm_work_page(
    ctx: &PageContext,
    defaults: &NewWorkDefaults,
    input: &NewWorkInput,
    mode: &str,
    pseud_label: &str,
) -> String {
    let publish = mode == "publish";
    let summary = if input.summary.trim().is_empty() {
        "—".to_string()
    } else {
        escape(input.summary.trim())
    };
    let language = defaults
        .languages
        .iter()
        .find(|choice| choice.value == input.language_id)
        .map_or_else(|| input.language_id.clone(), |choice| choice.label.clone());
    let mut rows = String::new();
    for (label, value) in [
        ("Title", escape(input.title.trim())),
        ("Published by", escape(pseud_label)),
        ("Rating", escape(&input.rating)),
        ("Archive Warnings", escape(&input.warnings.join(", "))),
        ("Fandoms", escape(&input.fandom)),
        ("Categories", escape(&input.categories.join(", "))),
        ("Relationships", escape(&input.relationships)),
        ("Characters", escape(&input.characters)),
        ("Additional Tags", escape(&input.additional_tags)),
        ("Language", escape(&language)),
        ("Collections", escape(&input.collections)),
        ("Gifts", escape(&input.recipients)),
        ("Chapter Title", escape(&input.chapter_title)),
        (
            "Words",
            format_count(input.content.split_whitespace().count() as u64),
        ),
    ] {
        if value.is_empty() {
            continue;
        }
        rows.push_str(&format!("<dt>{label}:</dt><dd>{value}</dd>"));
    }
    let mut body = format!(
        "<h2 class=\"heading\">Confirm {action}</h2>\
         <p class=\"note\">{description}</p>\
         <div class=\"wrapper\"><dl class=\"work meta group confirm-work\">{rows}\
         <dt>Summary:</dt><dd>{summary}</dd>\
         <dt>Work Text:</dt><dd><pre class=\"preview\">{content}</pre></dd></dl></div>\
         <form class=\"verbose\" method=\"post\" action=\"/write/work\">\
         <fieldset><legend>{action}</legend>\
         <p class=\"note\">This sends exactly one request to archiveofourown.org and is recorded \
         in the audit log. A posted work cannot be withdrawn from this app.</p>",
        action = if publish { "Post Work" } else { "Save Draft" },
        description = if publish {
            "Leave the archive to post it publicly on AO3 as shown above?"
        } else {
            "Save this work as an unposted draft on AO3?"
        },
        content = escape(input.content.trim()),
    );
    body.push_str(&hidden_scalars(input));
    for warning in &input.warnings {
        body.push_str(&hidden("warnings", warning));
    }
    for category in &input.categories {
        body.push_str(&hidden("categories", category));
    }
    body.push_str(&hidden("confirm", "yes"));
    body.push_str(
        "<p class=\"submit actions\">\
         <button type=\"submit\" name=\"action\" value=\"publish\">Confirm &amp; Post</button>\
         <button type=\"submit\" name=\"action\" value=\"draft\">Confirm &amp; Save Draft</button>\
         <a href=\"/works/new\">Back to the composer</a></p></fieldset></form>",
    );
    layout_with_class(ctx, "works-new", &body)
}

fn text_row(
    name: &str,
    label: &str,
    id: &str,
    value: &str,
    placeholder: &str,
    required: bool,
) -> String {
    let required = if required { " required" } else { "" };
    format!(
        "<dt class=\"{name}{required}\"><label for=\"{id}\">{label}</label></dt>\
         <dd class=\"{name}{required}\"><input id=\"{id}\" type=\"text\" name=\"{name}\" \
         value=\"{value}\" placeholder=\"{placeholder}\"></dd>",
        name = name,
        required = required,
        id = id,
        label = escape(label),
        value = escape(value),
        placeholder = escape(placeholder),
    )
}

fn textarea_row(name: &str, label: &str, id: &str, value: &str, rows: u8) -> String {
    format!(
        "<dt class=\"{name}\"><label for=\"{id}\">{label}</label></dt>\
         <dd class=\"{name}\"><textarea id=\"{id}\" name=\"{name}\" rows=\"{rows}\">{value}</textarea></dd>",
        name = name,
        id = id,
        label = escape(label),
        rows = rows,
        value = escape(value),
    )
}

fn hidden(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        escape(name),
        escape(value)
    )
}

fn hidden_scalars(input: &NewWorkInput) -> String {
    let mut out = String::new();
    for (name, value) in [
        ("title", &input.title),
        ("pseud_id", &input.pseud_id),
        ("rating", &input.rating),
        ("fandom", &input.fandom),
        ("relationships", &input.relationships),
        ("characters", &input.characters),
        ("additional_tags", &input.additional_tags),
        ("language_id", &input.language_id),
        ("collections", &input.collections),
        ("recipients", &input.recipients),
        ("chapter_title", &input.chapter_title),
    ] {
        out.push_str(&hidden(name, value));
    }
    // Long values: hidden textareas are still submitted by browsers, and unlike
    // attribute values they preserve newlines exactly.
    for (name, value) in [
        ("summary", &input.summary),
        ("notes", &input.notes),
        ("content", &input.content),
    ] {
        out.push_str(&format!(
            "<textarea class=\"hidden\" name=\"{}\">{}</textarea>",
            escape(name),
            escape(value)
        ));
    }
    out
}
