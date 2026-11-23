//! Original, text-first required-tag badges (AO3's 2x2 square: rating, warning,
//! category, status). No AO3 artwork is copied; these are plain markup + CSS.

use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::StoredWork;

pub fn required_tags(work: &StoredWork) -> String {
    let (rating_class, rating_label) = match work
        .rating
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        value if value.contains("general") => ("rating-g", "G"),
        value if value.contains("teen") => ("rating-t", "T"),
        value if value.contains("mature") => ("rating-m", "M"),
        value if value.contains("explicit") => ("rating-e", "E"),
        _ => ("rating-nr", "NR"),
    };

    let warnings = &work.tags.warnings;
    let no_warnings = warnings.is_empty()
        || warnings
            .iter()
            .any(|warning| warning.contains("No Archive Warnings"));
    let (warning_class, warning_label) = if no_warnings {
        ("warning-no", "–")
    } else {
        ("warning-yes", "!")
    };
    let warning_href = if no_warnings {
        None
    } else {
        warnings.first().map(|warning| super::tag_href(warning))
    };
    let category_href = work
        .tags
        .categories
        .first()
        .map(|category| super::tag_href(category));
    let rating_href = work.rating.as_deref().map(|rating| {
        format!(
            "/works?rating={}",
            percent_encode(rating, SpaceMode::Percent20)
        )
    });

    let category = work.tags.categories.first().map_or_else(
        || "?".to_string(),
        |category| category.chars().take(4).collect::<String>().to_uppercase(),
    );
    let (status_class, status_label, status_title) = match work.complete {
        Some(true) => ("status-complete", "✓".to_string(), "Completed"),
        Some(false) => ("status-wip", "WIP".to_string(), "Work in progress"),
        None => ("status-unknown", "?".to_string(), "Status unknown"),
    };

    format!(
        "<ul class=\"required-tags\" aria-label=\"rating, warnings, category, status\">\
         {rating_cell}{warning_cell}{category_cell}\
         <li class=\"{status_class}\" title=\"{status_title}\">{status}</li></ul>",
        rating_cell = badge_cell(rating_class, rating_href.as_deref(), rating_label, "Rating"),
        warning_cell = badge_cell(
            warning_class,
            warning_href.as_deref(),
            warning_label,
            "Archive warnings"
        ),
        category_cell = badge_cell("category", category_href.as_deref(), &category, "Category"),
        status = escape(&status_label),
    )
}

/// A badge cell: a local-filter link when there is something to filter on.
fn badge_cell(class: &str, href: Option<&str>, label: &str, title: &str) -> String {
    match href {
        Some(href) => format!(
            "<li class=\"{class}\" title=\"{title}\"><a href=\"{href}\">{}</a></li>",
            escape(label)
        ),
        None => format!(
            "<li class=\"{class}\" title=\"{title}\">{}</li>",
            escape(label)
        ),
    }
}
