//! Original, text-first required-tag badges matching AO3's iconic 2x2 square
//! (top-left: rating, bottom-left: archive warnings, top-right: category,
//! bottom-right: completion status). No AO3 artwork is copied; pure HTML + CSS.

use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::StoredWork;

pub fn required_tags(work: &StoredWork) -> String {
    required_tags_from(
        work.rating.as_deref(),
        &work.tags.warnings,
        &work.tags.categories,
        work.complete,
        true,
    )
}

/// Shared 2x2 required-tags square for both cached works and live AO3 blurbs.
pub fn required_tags_from(
    rating: Option<&str>,
    warnings: &[String],
    categories: &[String],
    complete: Option<bool>,
    linked: bool,
) -> String {
    let rating_title = rating.unwrap_or("Not Rated");
    let (rating_class, rating_label) = match rating
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        value if value.contains("general") => ("rating-g rating-general-audience", "G"),
        value if value.contains("teen") => ("rating-t rating-teen", "T"),
        value if value.contains("mature") => ("rating-m rating-mature", "M"),
        value if value.contains("explicit") => ("rating-e rating-explicit", "E"),
        _ => ("rating-nr rating-notrated", "—"),
    };
    let rating_href = linked.then(|| {
        rating.map(|value| {
            format!(
                "/works?rating={}",
                percent_encode(value, SpaceMode::Percent20)
            )
        })
    })
    .flatten();

    let has_choose_not = warnings
        .iter()
        .any(|warning| warning.to_ascii_lowercase().contains("chose not to"));
    let has_real_warning = warnings.iter().any(|warning| {
        let lower = warning.to_ascii_lowercase();
        !lower.contains("no archive warnings") && !lower.contains("chose not to")
    });
    let (warning_class, warning_label, warning_title) = if has_real_warning {
        (
            "warning-yes",
            "!",
            warnings
                .first()
                .map_or("Archive Warnings Apply", String::as_str),
        )
    } else if has_choose_not {
        (
            "warning-choose warning-choosenotto",
            "?",
            "Creator Chose Not To Use Archive Warnings",
        )
    } else {
        ("warning-no", "–", "No Archive Warnings Apply")
    };
    let warning_href = if linked && (has_real_warning || has_choose_not) {
        warnings.first().map(|warning| super::tag_href(warning))
    } else {
        None
    };

    let (category_class, category_label, category_title) = classify_category(categories);
    let category_href = if linked {
        categories.first().map(|category| super::tag_href(category))
    } else {
        None
    };

    let (status_class, status_label, status_title) = match complete {
        Some(true) => ("status-complete complete-yes", "✓", "Work is Complete"),
        Some(false) => ("status-wip complete-no", "WIP", "Work in Progress"),
        None => ("status-unknown", "–", "Status unknown"),
    };

    format!(
        "<ul class=\"required-tags\" aria-label=\"rating, warnings, category, status\">\
         {rating_cell}{warning_cell}{category_cell}\
         <li class=\"{status_class}\" title=\"{status_title}\"><span>{}</span></li></ul>",
        escape(status_label),
        rating_cell = badge_cell(rating_class, rating_href.as_deref(), rating_label, rating_title),
        warning_cell = badge_cell(
            warning_class,
            warning_href.as_deref(),
            warning_label,
            warning_title
        ),
        category_cell = badge_cell(
            category_class,
            category_href.as_deref(),
            &category_label,
            &category_title
        ),
    )
}

fn classify_category(categories: &[String]) -> (&'static str, String, String) {
    if categories.is_empty() {
        return (
            "category category-none",
            "—".to_string(),
            "No category".to_string(),
        );
    }
    if categories.len() > 1 {
        return (
            "category category-multi",
            "Multi".to_string(),
            categories.join(", "),
        );
    }
    let first = &categories[0];
    match first.to_ascii_lowercase().as_str() {
        "m/m" => ("category category-slash", "M/M".to_string(), first.clone()),
        "f/f" => (
            "category category-femslash",
            "F/F".to_string(),
            first.clone(),
        ),
        "f/m" => ("category category-het", "F/M".to_string(), first.clone()),
        "gen" => ("category category-gen", "Gen".to_string(), first.clone()),
        "multi" => (
            "category category-multi",
            "Multi".to_string(),
            first.clone(),
        ),
        "other" => (
            "category category-other",
            "Other".to_string(),
            first.clone(),
        ),
        "no category" | "?" => (
            "category category-none",
            "—".to_string(),
            "No category".to_string(),
        ),
        _ => (
            "category category-other",
            first.chars().take(4).collect::<String>().to_uppercase(),
            first.clone(),
        ),
    }
}

/// A badge cell: a local-filter link when there is something to filter on.
fn badge_cell(class: &str, href: Option<&str>, label: &str, title: &str) -> String {
    let title = escape(title);
    let label = escape(label);
    match href {
        Some(href) => format!(
            "<li class=\"{class}\" title=\"{title}\"><a href=\"{href}\">{label}</a></li>"
        ),
        None => format!("<li class=\"{class}\" title=\"{title}\"><span>{label}</span></li>"),
    }
}
