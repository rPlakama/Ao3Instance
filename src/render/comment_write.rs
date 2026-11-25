//! Reply and edit pages for one cached AO3 comment.
//!
//! The confirmation step reuses `render::confirm_page`; this module only builds
//! the small editor form (AO3's `_comment_form.html.erb` fieldset shape, one
//! textarea plus hidden `work_id`/`comment_id`). Delete uses the generic
//! confirmation page directly.

use super::chrome::{PageContext, layout_with_class};
use super::components::flash_notice;
use super::escape;

/// Reply editor (`heading` "Reply to comment") and comment editor ("Edit comment").
#[allow(
    clippy::too_many_arguments,
    reason = "small server-rendered form with independent state fields"
)]
pub fn comment_editor_page(
    ctx: &PageContext,
    heading: &str,
    description: &str,
    action: &str,
    work_id: &str,
    comment_id: &str,
    content: &str,
    parent_author: Option<&str>,
    parent_excerpt: Option<&str>,
    errors: &[String],
    notice: Option<&str>,
    submit_label: &str,
) -> String {
    let work_id_attr = escape(work_id);
    let comment_id_attr = escape(comment_id);
    let mut body = format!(
        "<h2 class=\"heading\">{heading}</h2><p>{description}</p>",
        heading = escape(heading),
        description = escape(description),
    );
    body.push_str(&flash_notice(notice));
    body.push_str(&flash_error_from(errors));
    if let (Some(author), Some(excerpt)) = (parent_author, parent_excerpt) {
        body.push_str(&format!(
            "<blockquote class=\"userstuff comment context\">\
             <p class=\"byline\">{}</p>\
             <div class=\"userstuff\">{}</div>\
             </blockquote>",
            escape(author),
            excerpt
        ));
    }
    body.push_str(&format!(
        "<div class=\"post comment\" id=\"comment_form_for_{work_id_attr}\">\
         <form class=\"new_comment\" method=\"post\" action=\"{action}\">\
         <fieldset><legend>{submit_label}</legend>\
         <p class=\"footnote\">It will be public on AO3. Up to 10,000 characters.</p>\
         <p><label class=\"landmark\" for=\"comment_content\">Comment</label>\
         <textarea id=\"comment_content\" name=\"content\" rows=\"8\" maxlength=\"10000\" \
         required>{content}</textarea></p>\
         <input type=\"hidden\" name=\"work_id\" value=\"{work_id_attr}\">\
         <input type=\"hidden\" name=\"comment_id\" value=\"{comment_id_attr}\">\
         <p class=\"submit actions\"><button type=\"submit\">{submit_label}</button> \
         <a href=\"/read/{work_id_attr}?show_comments=1\">Cancel</a></p>\
         </fieldset></form></div>",
        action = escape(action),
        submit_label = escape(submit_label),
        content = escape(content),
    ));
    layout_with_class(ctx, "works-show", &body)
}

fn flash_error_from(errors: &[String]) -> String {
    if errors.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<div class=\"flash error\"><h4>Sorry! We couldn't save this comment because:</h4><ul>",
    );
    for error in errors {
        out.push_str(&format!("<li>{}</li>", escape(error)));
    }
    out.push_str("</ul></div>");
    out
}
