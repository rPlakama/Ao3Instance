//! Original, text-first required-tag badges (AO3's 2x2 square: rating, warning,
//! category, status). No AO3 artwork is copied; these are plain markup + CSS.

use super::escape;
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
    let (warning_class, warning_label) = if warnings.is_empty()
        || warnings
            .iter()
            .any(|warning| warning.contains("No Archive Warnings"))
    {
        ("warning-no", "–")
    } else {
        ("warning-yes", "!")
    };

    let category = work.tags.categories.first().map_or_else(
        || "?".to_string(),
        |category| category.chars().take(4).collect::<String>().to_uppercase(),
    );
    let status_label = format!("{}ch", work.chapter_count);
    let status_class = if work.chapter_count > 1 {
        "status-wip"
    } else {
        "status-complete"
    };

    format!(
        "<ul class=\"required-tags\" aria-label=\"rating, warnings, category, status\">\
         <li class=\"{rating_class}\" title=\"Rating: {rating_title}\">{rating_label}</li>\
         <li class=\"{warning_class}\" title=\"Archive warnings\">{warning_label}</li>\
         <li class=\"category\" title=\"Category\">{category}</li>\
         <li class=\"{status_class}\" title=\"Chapters\">{status}</li></ul>",
        rating_title = escape(work.rating.as_deref().unwrap_or("Not rated")),
        status = escape(&status_label),
        category = escape(&category),
    )
}
