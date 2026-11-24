//! Search & tag-filter pages: local offline search, live AO3 Work Search, and
//! live AO3 tag pages with the `#work-filters` sidebar.

mod live;
mod local;
mod tag_page;

use super::components::select_options;
use super::escape;

pub use live::{live_search_form_page, live_search_results_page, search_challenge_page};
pub use local::{filter_from, search_form_page, search_results_page};
pub use tag_page::{TagFilter, live_tag_page};

/// Shared AO3 completion filter choices used by both Work Search and tag sidebars.
pub(super) const COMPLETION_OPTIONS: &[(&str, &str)] = &[
    ("", "All works"),
    ("T", "Complete works only"),
    ("F", "Works in progress only"),
];

/// Shared AO3 crossover filter choices used by both Work Search and tag sidebars.
pub(super) const CROSSOVER_OPTIONS: &[(&str, &str)] = &[
    ("", "Include crossovers"),
    ("F", "Exclude crossovers"),
    ("T", "Only crossovers"),
];

/// Shared AO3 language codes used by both Work Search and tag sidebars.
pub(super) const AO3_LANGUAGES: &[(&str, &str)] = &[
    ("", "Any language"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("ptBR", "Português brasileiro"),
    ("ru", "Русский"),
    ("zh", "中文"),
    ("ja", "日本語"),
];

pub fn row(label: &str, control: &str) -> String {
    format!("<dt><label>{label}</label></dt><dd>{control}</dd>")
}

pub(super) fn text_input(name: &str, value: Option<&str>, placeholder: &str) -> String {
    format!(
        "<input type=\"text\" name=\"{name}\" value=\"{}\" placeholder=\"{}\">",
        escape(value.unwrap_or("")),
        escape(placeholder)
    )
}

pub(super) fn number_input(name: &str, value: Option<&str>) -> String {
    format!(
        "<input type=\"number\" min=\"0\" name=\"{name}\" value=\"{}\" class=\"number\">",
        escape(value.unwrap_or(""))
    )
}

pub(super) fn autocomplete_input(
    id: Option<&str>,
    name: &str,
    value: Option<&str>,
    placeholder: &str,
    kind: &str,
) -> String {
    let id_attr = id
        .map(|id| format!(" id=\"{}\"", escape(id)))
        .unwrap_or_default();
    format!(
        "<div class=\"autocomplete-field\" data-autocomplete=\"{kind}\">\
         <input{id_attr} type=\"text\" name=\"{name}\" value=\"{}\" placeholder=\"{}\" autocomplete=\"off\">\
         </div>",
        escape(value.unwrap_or_default()),
        escape(placeholder),
    )
}

pub(super) fn radio_group(name: &str, options: &[(&str, &str)], current: &str) -> String {
    let items: String = options
        .iter()
        .map(|(value, label)| {
            format!(
                "<li><label class=\"check\"><input type=\"radio\" name=\"{name}\" value=\"{}\"{}> \
                 {label}</label></li>",
                escape(value),
                if current == *value { " checked" } else { "" }
            )
        })
        .collect();
    format!("<ul class=\"choice-list\">{items}</ul>")
}

pub(super) fn live_select(name: &str, options: &[(&str, &str)], current: &str) -> String {
    format!(
        "<select name=\"{name}\">{}</select>",
        select_options(options, current)
    )
}
