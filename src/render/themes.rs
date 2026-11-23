//! Public AO3 site-skin gallery modelled on `otwarchive/app/views/skins/index.html.erb`.

use super::chrome::{PageContext, layout_with_class};
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
         <ul class=\"navigation actions\" role=\"navigation\">\
         <li><a href=\"/session#appearance\">Built-in Themes</a></li>\
         <li class=\"active\"><span class=\"current\">Public Site Skins</span></li></ul>\
         <p class=\"notes\">Fetched from <a href=\"https://archiveofourown.org/skins\" \
         rel=\"noopener noreferrer\">archiveofourown.org/skins</a>. Selecting a skin imports \
         its CSS locally; built-in themes remain under \
         <a href=\"/session#appearance\">Appearance</a>.</p>",
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
        body.push_str("<h4 class=\"landmark heading\">List of Skins</h4>");
        body.push_str("<ul class=\"skin picture index group public-skins\">");
        for skin in skins {
            let href = if skin.href.starts_with("https://") {
                skin.href.clone()
            } else {
                format!("https://archiveofourown.org{}", skin.href)
            };
            body.push_str(&format!(
                "<li class=\"skins picture blurb group\">\
                 <div class=\"header module\">\
                 <h4 class=\"heading\"><a href=\"{href}\" rel=\"noopener noreferrer\">{title}</a></h4>\
                 <p class=\"byline\">{byline}</p></div>\
                 <blockquote class=\"userstuff summary\"><p>{description}</p></blockquote>\
                 <ul class=\"actions\"><li>\
                 <form class=\"inline\" method=\"post\" action=\"/themes/apply\">\
                 <input type=\"hidden\" name=\"href\" value=\"{apply_href}\">\
                 <button type=\"submit\">Use this skin</button></form></li></ul></li>",
                title = escape(&skin.title),
                byline = escape(&skin.byline),
                description = escape(&skin.description),
                apply_href = escape(&skin.href),
            ));
        }
        body.push_str("</ul>");
    }
    layout_with_class(ctx, "dashboard skins-index", &body)
}
