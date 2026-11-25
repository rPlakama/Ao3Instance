//! Parsing AO3's new-work form and validating a composed work before posting.
//!
//! The composer copies AO3's own `works/_standard_form.html.erb` +
//! `_work_form_tags.html.erb` fields. Instead of hardcoding private database ids
//! (most importantly the numeric `Language` ids, which the search form exposes
//! only as ISO short codes), the compose page is built from AO3's real
//! `/works/new` page: the same form the user would fill in on the archive.
//!
//! Selector provenance (pinned otwarchive clone, `452391b`):
//! | field | evidence |
//! | --- | --- |
//! | form | `app/views/works/_standard_form.html.erb:2` — `form#work-form.verbose.post.work` |
//! | ratings | `_work_form_tags.html.erb:22-32` — `select[name="work[rating_string]"]` |
//! | warnings | `_work_form_tags.html.erb:38-55` — `input[name="work[archive_warning_strings][]"]` |
//! | categories | `_work_form_tags.html.erb:66-83` — `input[name="work[category_strings][]"]` |
//! | languages | `app/views/works/_work_form_associations_language.html.erb:5-10` — `select[name="work[language_id]"]` |
//! | byline | `app/views/pseuds/_byline.html.erb:24-34` — `work[author_attributes][ids][]` (hidden for one pseud, multi-select for several) |
//!
//! Contract: a page without `#work-form` is not the new-work form (login wall,
//! challenge, layout change) and yields [`Error::UpstreamChanged`]; callers show
//! an honest error instead of a composer that cannot post.

use std::sync::OnceLock;

use scraper::{ElementRef, Html, Selector};

use crate::ao3::html::{collapse_text, parse_selector as css};
use crate::error::{Error, Result};

/// One select/checkbox option from AO3's form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// The options AO3's `/works/new` page offers the signed-in user.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewWorkDefaults {
    pub ratings: Vec<Choice>,
    pub warnings: Vec<Choice>,
    pub categories: Vec<Choice>,
    /// Numeric language ids (the only place AO3 exposes them).
    pub languages: Vec<Choice>,
    /// Creator pseud ids from the byline field / select.
    pub pseuds: Vec<Choice>,
}

impl NewWorkDefaults {
    pub fn default_language(&self) -> Option<&Choice> {
        self.languages
            .iter()
            .find(|choice| choice.selected)
            .or_else(|| self.languages.first())
    }

    pub fn default_rating(&self) -> Option<&Choice> {
        self.ratings
            .iter()
            .find(|choice| choice.selected)
            .or_else(|| self.ratings.first())
    }

    pub fn default_pseud(&self) -> Option<&Choice> {
        self.pseuds
            .iter()
            .find(|choice| choice.selected)
            .or_else(|| self.pseuds.first())
    }
}

/// The values a composer submission carries (our form names, re-encoded for AO3
/// by `actions::post_work`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewWorkInput {
    pub title: String,
    pub pseud_id: String,
    pub rating: String,
    pub warnings: Vec<String>,
    pub fandom: String,
    pub categories: Vec<String>,
    pub relationships: String,
    pub characters: String,
    pub additional_tags: String,
    pub language_id: String,
    pub summary: String,
    pub notes: String,
    pub collections: String,
    pub recipients: String,
    pub chapter_title: String,
    pub content: String,
}

/// AO3 limits mirrored from `config/config.yml` (see each line).
pub const TITLE_MAX: usize = 255; // TITLE_MAX: 255
pub const SUMMARY_MAX: usize = 1_250; // SUMMARY_MAX: 1250
pub const NOTES_MAX: usize = 5_000; // NOTES_MAX: 5000
pub const TAG_MAX: usize = 150; // TAG_MAX: 150
pub const USER_DEFINED_TAGS_MAX: usize = 75; // USER_DEFINED_TAGS_MAX: 75
pub const CONTENT_MIN: usize = 10; // CONTENT_MIN: 10
pub const CONTENT_MAX: usize = 510_000; // CONTENT_MAX: 510000

/// Parses AO3's `/works/new` page into the options the composer renders.
pub fn parse_new_work_page(html: &str) -> Result<NewWorkDefaults> {
    let document = Html::parse_document(html);
    let selectors = selectors()?;
    let form = document.select(&selectors.form).next().ok_or_else(|| {
        Error::UpstreamChanged(
            "AO3 did not show the new-work form (not signed in, or posting is unavailable)"
                .to_string(),
        )
    })?;

    let ratings = options(form, &selectors.rating_options);
    let warnings = checkbox_values(form, &selectors.warning_boxes);
    let categories = checkbox_values(form, &selectors.category_boxes);
    let mut languages = options(form, &selectors.language_options);
    languages.retain(|choice| !choice.value.trim().is_empty());
    let mut pseuds = checkbox_values(form, &selectors.author_ids);
    pseuds.extend(options(form, &selectors.author_selections));
    // The hidden single-pseud input carries no label; callers fill the name from
    // the session's pseud discovery when needed.
    for pseud in &mut pseuds {
        if pseud.label.is_empty() {
            pseud.label = format!("Pseud #{}", pseud.value);
        }
    }

    Ok(NewWorkDefaults {
        ratings,
        warnings,
        categories,
        languages,
        pseuds,
    })
}

fn options(form: ElementRef<'_>, selector: &Selector) -> Vec<Choice> {
    form.select(selector)
        .filter_map(|option| {
            let value = option.value().attr("value")?.trim().to_string();
            let label = collapse_text(&option);
            Some(Choice {
                selected: option.value().attr("selected").is_some(),
                value,
                label,
            })
        })
        .collect()
}

fn checkbox_values(form: ElementRef<'_>, selector: &Selector) -> Vec<Choice> {
    form.select(selector)
        .filter_map(|input| {
            let value = input.value().attr("value")?.trim().to_string();
            if value.is_empty() {
                return None;
            }
            Some(Choice {
                label: value.clone(),
                value,
                selected: input.value().attr("checked").is_some(),
            })
        })
        .collect()
}

/// Local validation before the confirmation step, mirroring AO3's model rules
/// (`app/models/work.rb:93-172`, `app/models/chapter.rb:20-22`) so a composer
/// round-trip never sends an obviously invalid work upstream.
pub fn validate(input: &NewWorkInput) -> Vec<String> {
    let mut errors = Vec::new();
    let title = input.title.trim();
    if title.is_empty() {
        errors.push("Work Title is required.".to_string());
    } else if title.chars().count() > TITLE_MAX {
        errors.push(format!(
            "Work Title must be at most {TITLE_MAX} characters."
        ));
    }
    if input.fandom.trim().is_empty() {
        errors.push("Fandoms is required (comma separated tag names).".to_string());
    }
    if input.rating.trim().is_empty() {
        errors.push("Rating is required.".to_string());
    }
    if input
        .warnings
        .iter()
        .all(|warning| warning.trim().is_empty())
    {
        errors.push("At least one Archive Warning is required.".to_string());
    }
    if input.language_id.trim().is_empty() {
        errors.push("Language is required.".to_string());
    }
    if input.pseud_id.trim().is_empty() {
        errors.push("A creator pseud is required.".to_string());
    }
    if input.summary.chars().count() > SUMMARY_MAX {
        errors.push(format!("Summary must be at most {SUMMARY_MAX} characters."));
    }
    if input.notes.chars().count() > NOTES_MAX {
        errors.push(format!("Notes must be at most {NOTES_MAX} characters."));
    }
    for (label, tags) in [
        ("Fandom", &input.fandom),
        ("Relationship", &input.relationships),
        ("Character", &input.characters),
        ("Additional Tags", &input.additional_tags),
    ] {
        for tag in tags.split(',') {
            let tag = tag.trim();
            if tag.chars().count() > TAG_MAX {
                errors.push(format!(
                    "Each {label} tag must be at most {TAG_MAX} characters (\"{}\").",
                    shortened(tag)
                ));
            }
        }
    }
    let user_defined = count_tags(&input.fandom)
        + count_tags(&input.relationships)
        + count_tags(&input.characters)
        + count_tags(&input.additional_tags);
    if user_defined > USER_DEFINED_TAGS_MAX {
        errors.push(format!(
            "Fandom, relationship, character and additional tags may not add up to more than \
             {USER_DEFINED_TAGS_MAX} (you have {user_defined})."
        ));
    }
    let content_len = input.content.trim().chars().count();
    if content_len < CONTENT_MIN {
        errors.push(format!(
            "Work Text must be at least {CONTENT_MIN} characters long."
        ));
    } else if input.content.chars().count() > CONTENT_MAX {
        errors.push(format!(
            "Work Text must be at most {CONTENT_MAX} characters long."
        ));
    }
    if input.chapter_title.chars().count() > TITLE_MAX {
        errors.push(format!(
            "Chapter Title must be at most {TITLE_MAX} characters."
        ));
    }
    errors
}

fn count_tags(tags: &str) -> usize {
    tags.split(',').filter(|tag| !tag.trim().is_empty()).count()
}

fn shortened(tag: &str) -> String {
    let mut out: String = tag.chars().take(40).collect();
    if tag.chars().count() > 40 {
        out.push('…');
    }
    out
}

struct Selectors {
    form: Selector,
    rating_options: Selector,
    warning_boxes: Selector,
    category_boxes: Selector,
    language_options: Selector,
    author_ids: Selector,
    author_selections: Selector,
}

fn selectors() -> Result<&'static Selectors> {
    static CACHE: OnceLock<std::result::Result<Selectors, String>> = OnceLock::new();
    let compiled = CACHE.get_or_init(|| build_selectors().map_err(|err| err.to_string()));
    compiled
        .as_ref()
        .map_err(|msg| Error::UpstreamChanged(msg.clone()))
}

fn build_selectors() -> Result<Selectors> {
    Ok(Selectors {
        form: css("#work-form")?,
        rating_options: css(r#"select[name="work[rating_string]"] option"#)?,
        warning_boxes: css(r#"input[name="work[archive_warning_strings][]"]"#)?,
        category_boxes: css(r#"input[name="work[category_strings][]"]"#)?,
        language_options: css(r#"select[name="work[language_id]"] option"#)?,
        author_ids: css(r#"input[name="work[author_attributes][ids][]"]"#)?,
        author_selections: css(r#"select[name="work[author_attributes][ids][]"] option"#)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_WORK: &str = include_str!("../../tests/fixtures/ao3_new_work.html");

    #[test]
    fn parses_the_live_form_options() {
        let defaults = parse_new_work_page(NEW_WORK).expect("new-work page");
        assert_eq!(
            defaults
                .ratings
                .iter()
                .map(|choice| choice.value.as_str())
                .collect::<Vec<_>>(),
            [
                "Not Rated",
                "General Audiences",
                "Teen And Up Audiences",
                "Mature",
                "Explicit"
            ]
        );
        assert_eq!(defaults.warnings.len(), 6);
        assert!(
            defaults
                .warnings
                .iter()
                .any(|warning| warning.value == "No Archive Warnings Apply")
        );
        assert_eq!(defaults.categories.len(), 6);
        assert_eq!(defaults.languages.len(), 3);
        assert_eq!(defaults.languages[0].value, "1");
        assert_eq!(defaults.languages[0].label, "English");
        assert_eq!(
            defaults.default_language().map(|c| c.value.as_str()),
            Some("1")
        );
        assert_eq!(
            defaults
                .pseuds
                .iter()
                .map(|choice| (choice.value.as_str(), choice.label.as_str()))
                .collect::<Vec<_>>(),
            [("111", "Main Pseud"), ("222", "Second Pseud")]
        );
        assert_eq!(
            defaults.default_pseud().map(|choice| choice.value.as_str()),
            Some("111")
        );
    }

    #[test]
    fn a_page_without_the_work_form_fails_closed() {
        let html = "<html><body><div id=\"login\">Log in</div></body></html>";
        let error = parse_new_work_page(html).expect_err("not a new-work page");
        assert!(matches!(error, Error::UpstreamChanged(_)), "{error:?}");
    }

    fn input() -> NewWorkInput {
        NewWorkInput {
            title: "A Test Fic".into(),
            pseud_id: "111".into(),
            rating: "General Audiences".into(),
            warnings: vec!["No Archive Warnings Apply".into()],
            fandom: "Test Fandom".into(),
            language_id: "1".into(),
            content: "The lighthouse keeper counted every ship.".into(),
            ..NewWorkInput::default()
        }
    }

    #[test]
    fn validation_mirrors_the_model_limits() {
        assert!(validate(&input()).is_empty());

        let mut blank = input();
        blank.title = "   ".into();
        blank.fandom.clear();
        blank.rating.clear();
        blank.warnings.clear();
        blank.language_id.clear();
        blank.content = "short".into();
        let errors = validate(&blank);
        for needle in [
            "Work Title is required",
            "Fandoms is required",
            "Rating is required",
            "Archive Warning is required",
            "Language is required",
            "at least 10 characters",
        ] {
            assert!(
                errors.iter().any(|error| error.contains(needle)),
                "missing {needle}: {errors:?}"
            );
        }

        let mut long = input();
        long.title = "x".repeat(TITLE_MAX + 1);
        long.summary = "y".repeat(SUMMARY_MAX + 1);
        long.additional_tags = (0..80)
            .map(|n| format!("tag{n}"))
            .collect::<Vec<_>>()
            .join(",");
        let errors = validate(&long);
        assert!(errors.iter().any(|error| error.contains("Work Title")));
        assert!(errors.iter().any(|error| error.contains("Summary")));
        assert!(errors.iter().any(|error| error.contains("may not add up")));
    }
}
