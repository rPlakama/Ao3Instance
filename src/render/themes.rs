//! Public AO3 site-skin gallery and custom theme controls.

use super::chrome::{PageContext, layout};
use super::escape;
use crate::ao3::skins::PublicSkin;

pub fn themes_page(
    ctx: &PageContext,
    skins: &[PublicSkin],
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let mut body = String::from(
        "<h2 class=\"heading\">Public AO3 site skins</h2>\
         <p class=\"meta\">Fetched from <a href=\"https://archiveofourown.org/skins\">\
         archiveofourown.org/skins</a>. Selecting a skin imports its CSS locally; \
         built-in themes remain under <a href=\"/session#appearance\">Appearance</a>.</p>",
    );
    if ctx.skin == "public" {
        body.push_str(
            "<div class=\"flash notice\">A public AO3 skin is active. \
             <form class=\"inline\" method=\"post\" action=\"/themes/clear\">\
             <button type=\"submit\">Remove public skin</button></form></div>",
        );
    }
    if let Some(notice) = notice {
        body.push_str(&format!(
            "<div class=\"flash notice\">{}</div>",
            escape(notice)
        ));
    }
    if let Some(error) = error {
        body.push_str(&format!(
            "<div class=\"flash error\">{}</div>",
            escape(error)
        ));
    }
    if skins.is_empty() {
        body.push_str(
            "<p class=\"muted\">No public skins were returned. AO3 may be blocking the \
             request; check <a href=\"/diagnostics\">Diagnostics</a>.</p>",
        );
    } else {
        body.push_str("<ul class=\"public-skins\">");
        for skin in skins {
            let href = if skin.href.starts_with("https://") {
                skin.href.clone()
            } else {
                format!("https://archiveofourown.org{}", skin.href)
            };
            body.push_str(&format!(
                "<li><h3><a href=\"{href}\">{title}</a></h3>\
                 <p class=\"meta\">{byline}</p><p>{description}</p>\
                 <form class=\"inline\" method=\"post\" action=\"/themes/apply\">\
                 <input type=\"hidden\" name=\"href\" value=\"{apply_href}\">\
                 <button type=\"submit\">Use this skin</button></form></li>",
                title = escape(&skin.title),
                byline = escape(&skin.byline),
                description = escape(&skin.description),
                apply_href = escape(&skin.href),
            ));
        }
        body.push_str("</ul>");
    }
    layout(ctx, &body)
}
