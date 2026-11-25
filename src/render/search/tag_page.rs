//! Live AO3 tag works page (`/tags/<tag>/works`) and `#work-filters` sidebar.

use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::ao3::search::{LiveSearchResults, LiveTagFacets, TagFacet};
use crate::render::blurb::live_blurb;
use crate::render::chrome::{PageContext, layout_with_class};
use crate::render::components::{cache_banner, filter_panel, select_options};
use crate::render::{escape, tag_href};

use super::{AO3_LANGUAGES, COMPLETION_OPTIONS, CROSSOVER_OPTIONS, autocomplete_input};

/// Selected filters for a live tag page. Kept separate from the upstream query
/// so the sidebar can be re-rendered with the exact submitted values.
#[derive(Debug, Clone)]
pub struct TagFilter {
    pub sort: String,
    pub direction: String,
    pub query: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_fandoms: String,
    pub include_characters: String,
    pub include_relationships: String,
    pub include_additional: String,
    pub other_include: String,
    pub exclude_fandoms: String,
    pub exclude_characters: String,
    pub exclude_relationships: String,
    pub exclude_additional: String,
    pub other_exclude: String,
    pub complete: String,
    pub crossover: String,
    pub words_from: String,
    pub words_to: String,
    pub language: String,
}

impl Default for TagFilter {
    fn default() -> Self {
        Self {
            sort: "date".to_string(),
            direction: "desc".to_string(),
            query: String::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            include_fandoms: String::new(),
            include_characters: String::new(),
            include_relationships: String::new(),
            include_additional: String::new(),
            other_include: String::new(),
            exclude_fandoms: String::new(),
            exclude_characters: String::new(),
            exclude_relationships: String::new(),
            exclude_additional: String::new(),
            other_exclude: String::new(),
            complete: String::new(),
            crossover: String::new(),
            words_from: String::new(),
            words_to: String::new(),
            language: String::new(),
        }
    }
}

impl TagFilter {
    /// True when the submitted query narrowed the results.
    ///
    /// Mirrors AO3's `showFilters()`: an in-use filter set auto-opens the
    /// narrow-viewport Sort & Filter panel instead of leaving it collapsed.
    pub fn has_active_values(&self) -> bool {
        self.sort != "date"
            || !self.include.is_empty()
            || !self.exclude.is_empty()
            || !self.query.trim().is_empty()
            || [
                &self.include_fandoms,
                &self.include_characters,
                &self.include_relationships,
                &self.include_additional,
                &self.other_include,
                &self.exclude_fandoms,
                &self.exclude_characters,
                &self.exclude_relationships,
                &self.exclude_additional,
                &self.other_exclude,
                &self.complete,
                &self.crossover,
                &self.words_from,
                &self.words_to,
                &self.language,
            ]
            .iter()
            .any(|value| !value.trim().is_empty())
    }

    fn to_query(&self) -> String {
        let encode = |value: &str| percent_encode(value, SpaceMode::Percent20);
        let mut parts: Vec<String> = Vec::new();
        if !self.sort.is_empty() && self.sort != "date" {
            parts.push(format!("sort={}", encode(&self.sort)));
        }
        if !self.query.is_empty() {
            parts.push(format!("q={}", encode(&self.query)));
        }
        for (name, value) in [
            ("include_fandoms", &self.include_fandoms),
            ("include_characters", &self.include_characters),
            ("include_relationships", &self.include_relationships),
            ("include_additional", &self.include_additional),
            ("other_tags_include", &self.other_include),
            ("exclude_fandoms", &self.exclude_fandoms),
            ("exclude_characters", &self.exclude_characters),
            ("exclude_relationships", &self.exclude_relationships),
            ("exclude_additional", &self.exclude_additional),
            ("other_tags_exclude", &self.other_exclude),
            ("words_from", &self.words_from),
            ("words_to", &self.words_to),
            ("language", &self.language),
            ("complete", &self.complete),
            ("crossover", &self.crossover),
        ] {
            if !value.is_empty() {
                parts.push(format!("{name}={}", encode(value)));
            }
        }
        for tag in &self.include {
            parts.push(format!("include_tag={}", encode(tag)));
        }
        for tag in &self.exclude {
            parts.push(format!("exclude_tag={}", encode(tag)));
        }
        parts.join("&")
    }
}

/// Live tag works page (`/tags/<tag>/works`).
pub fn live_tag_page(
    ctx: &PageContext,
    tag: &str,
    page: i64,
    results: &LiveSearchResults,
    filter: &TagFilter,
    cache_meta: Option<(bool, i64)>,
) -> String {
    let total = i64::try_from(results.total).unwrap_or(i64::MAX);
    let per_page = 20;
    let start = (page - 1) * per_page + 1;
    let end = start + results.works.len() as i64 - 1;
    let noun = if total == 1 { "Work" } else { "Works" };
    let total_display = crate::time::format_count(u64::try_from(total).unwrap_or(0));
    let heading = if total > per_page {
        format!(
            "{start} - {end} of {total_display} {noun} in <a href=\"{}\">{}</a>",
            tag_href(tag),
            escape(tag)
        )
    } else {
        format!(
            "{total_display} {noun} in <a href=\"{}\">{}</a>",
            tag_href(tag),
            escape(tag)
        )
    };
    let tag_path = format!("/tags/{}/works", percent_encode(tag, SpaceMode::Percent20));
    let extra_query = filter.to_query();
    let refresh_href = if extra_query.is_empty() {
        format!("{tag_path}?page={page}&refresh=1")
    } else {
        format!("{tag_path}?{extra_query}&page={page}&refresh=1")
    };
    let mut body = format!("<h2 class=\"heading\">{heading}</h2>");
    body.push_str(&format!(
        "<div class=\"navigation actions module\">\
         <h3 class=\"landmark heading\">Navigation and Actions</h3>\
         <ul class=\"user navigation actions\" role=\"navigation\">\
         <li class=\"active\"><span class=\"current\">Works</span></li>\
         <li><a href=\"/bookmarks\">Bookmarks</a></li>\
         <li><a href=\"/works\">Clear tag</a></li>\
         <li><a href=\"/works/search?include_tags={}\">Search within this tag on AO3</a></li>\
         <li><a href=\"{refresh_href}\">Refresh</a></li>\
         </ul></div>",
        percent_encode(tag, SpaceMode::Percent20)
    ));
    body.push_str(&cache_banner(
        cache_meta,
        &refresh_href,
        "Live from archiveofourown.org",
    ));
    let pager = crate::render::pagination(page, per_page, total, &tag_path, &extra_query);
    body.push_str("<div class=\"browse-grid\"><section class=\"browse-results\">");
    if results.works.is_empty() {
        body.push_str("<p class=\"muted\">No works on this tag page.</p>");
    } else {
        body.push_str(&pager);
        body.push_str("<h3 class=\"landmark heading\">Listing Works</h3>");
        body.push_str("<ol class=\"work index group\">");
        for work in &results.works {
            body.push_str(&live_blurb(work));
        }
        body.push_str("</ol>");
        body.push_str(&pager);
        if ctx.account.is_empty() {
            body.push_str(
                "<p class=\"meta\">Signed out: this is AO3's signed-out tag page. \
                 <a href=\"/session#cookie-import\">Import your AO3 session</a> for your \
                 account's view.</p>",
            );
        }
    }
    body.push_str("</section>");
    body.push_str(&tag_filter_sidebar(tag, filter, &results.facets));
    body.push_str("</div>");
    layout_with_class(ctx, "works-index filtered", &body)
}

fn facet_group(
    action: &str,
    label: &str,
    facets: &[TagFacet],
    fallback: &[&str],
    selected: &[String],
    fallback_input: Option<(&str, &str, &str, &str)>,
    rendered_selected: &mut Vec<String>,
) -> String {
    let items: Vec<TagFacet> = if !facets.is_empty() {
        facets.to_vec()
    } else {
        fallback
            .iter()
            .map(|name| TagFacet {
                name: (*name).to_string(),
                count: None,
            })
            .collect()
    };

    let has_checked = items
        .iter()
        .any(|facet| selected.iter().any(|item| item == &facet.name));
    let has_text = fallback_input.is_some_and(|(_, value, _, _)| !value.trim().is_empty());
    let open_attr = if has_checked || has_text { " open" } else { "" };

    let mut out =
        format!("<details class=\"filter-group {action}\"{open_attr}><summary>{label}</summary>");
    if !items.is_empty() {
        out.push_str("<ul class=\"facet-list\">");
        for facet in &items {
            let is_checked = selected.iter().any(|item| item == &facet.name);
            if is_checked {
                rendered_selected.push(facet.name.clone());
            }
            let checked = if is_checked { " checked" } else { "" };
            let count_suffix = facet.count.map_or_else(String::new, |n| {
                format!(" ({})", crate::time::format_count(n))
            });
            out.push_str(&format!(
                "<li><label><input type=\"checkbox\" name=\"{action}_tag\" value=\"{value}\"{checked}>\
                 <span class=\"indicator\" aria-hidden=\"true\"></span><span>{value}{count_suffix}</span></label></li>",
                value = escape(&facet.name),
            ));
        }
        out.push_str("</ul>");
    }
    if let Some((name, value, placeholder, kind)) = fallback_input
        && (items.is_empty() || !value.trim().is_empty())
    {
        out.push_str("<div class=\"facet-extra\">");
        out.push_str(&autocomplete_input(
            None,
            name,
            Some(value),
            placeholder,
            kind,
        ));
        out.push_str("</div>");
    }
    out.push_str("</details>");
    out
}

fn radio_filter_group(title: &str, name: &str, options: &[(&str, &str)], current: &str) -> String {
    let open_attr = if current.is_empty() { "" } else { " open" };
    let mut out = format!(
        "<details class=\"filter-group\"{open_attr}><summary>{title}</summary><ul class=\"facet-list\">"
    );
    for (value, label) in options {
        out.push_str(&format!(
            "<li><label><input type=\"radio\" name=\"{name}\" value=\"{value}\"{}>\
             <span class=\"indicator\" aria-hidden=\"true\"></span><span>{label}</span></label></li>",
            if current == *value { " checked" } else { "" }
        ));
    }
    out.push_str("</ul></details>");
    out
}

fn tag_filter_sidebar(tag: &str, filter: &TagFilter, facets: &LiveTagFacets) -> String {
    const SORT_OPTIONS: &[(&str, &str)] = &[
        ("date", "Date Updated"),
        ("created_at", "Date Posted"),
        ("words", "Word Count"),
        ("kudos", "Kudos"),
        ("hits", "Hits"),
        ("comments", "Comments"),
        ("bookmarks", "Bookmarks"),
        ("best_match", "Best Match"),
    ];
    const DEFAULT_RATINGS: &[&str] = &[
        "Not Rated",
        "General Audiences",
        "Teen And Up Audiences",
        "Mature",
        "Explicit",
    ];
    const DEFAULT_WARNINGS: &[&str] = &[
        "Creator Chose Not To Use Archive Warnings",
        "No Archive Warnings Apply",
        "Graphic Depictions Of Violence",
        "Major Character Death",
        "Rape/Non-Con",
        "Underage Sex",
    ];
    const DEFAULT_CATEGORIES: &[&str] = &["F/F", "F/M", "Gen", "M/M", "Multi", "Other"];

    let path = format!("/tags/{}/works", percent_encode(tag, SpaceMode::Percent20));
    let mut out = format!(
        "<form id=\"work-filters\" class=\"filters\" method=\"get\" action=\"{path}\">\
         <h3 class=\"landmark heading\">Filters</h3><fieldset>\
         <legend>Filter results:</legend>\
         <div class=\"filter-submit\"><button type=\"submit\">Sort and Filter</button></div>\
         <label class=\"filter-sort\">Sort by<select name=\"sort\">{}</select></label>",
        select_options(SORT_OPTIONS, &filter.sort)
    );

    for (action, label, include) in [("include", "Include", true), ("exclude", "Exclude", false)] {
        let selected = if include {
            &filter.include
        } else {
            &filter.exclude
        };
        let mut rendered_selected = Vec::new();
        out.push_str(&format!("<h4 class=\"filter-heading\">{label}</h4>"));
        out.push_str(&facet_group(
            action,
            "Ratings",
            &facets.ratings,
            DEFAULT_RATINGS,
            selected,
            None,
            &mut rendered_selected,
        ));
        out.push_str(&facet_group(
            action,
            "Warnings",
            &facets.warnings,
            DEFAULT_WARNINGS,
            selected,
            None,
            &mut rendered_selected,
        ));
        out.push_str(&facet_group(
            action,
            "Categories",
            &facets.categories,
            DEFAULT_CATEGORIES,
            selected,
            None,
            &mut rendered_selected,
        ));
        let prefix = if include { "include" } else { "exclude" };
        let (fandoms, characters, relationships, additional, other) = if include {
            (
                &filter.include_fandoms,
                &filter.include_characters,
                &filter.include_relationships,
                &filter.include_additional,
                &filter.other_include,
            )
        } else {
            (
                &filter.exclude_fandoms,
                &filter.exclude_characters,
                &filter.exclude_relationships,
                &filter.exclude_additional,
                &filter.other_exclude,
            )
        };
        for (group_label, group_facets, suffix, value, placeholder, kind) in [
            (
                "Fandoms",
                &facets.fandoms,
                "fandoms",
                fandoms,
                "Search fandoms",
                "fandom",
            ),
            (
                "Characters",
                &facets.characters,
                "characters",
                characters,
                "Search characters",
                "character",
            ),
            (
                "Relationships",
                &facets.relationships,
                "relationships",
                relationships,
                "Search relationships",
                "relationship",
            ),
            (
                "Additional Tags",
                &facets.freeforms,
                "additional",
                additional,
                "Search additional tags",
                "freeform",
            ),
        ] {
            let field_name = format!("{prefix}_{suffix}");
            out.push_str(&facet_group(
                action,
                group_label,
                group_facets,
                &[],
                selected,
                Some((&field_name, value, placeholder, kind)),
                &mut rendered_selected,
            ));
        }

        // Preserve any checked tag that narrowed out of the top-10 facets
        // inside the "Other tags to include/exclude" autocomplete field so
        // the user can still see and remove it.
        let mut other_parts: Vec<String> = other
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        for item in selected {
            if !rendered_selected.iter().any(|seen| seen == item)
                && !other_parts.iter().any(|seen| seen == item)
            {
                other_parts.push(item.clone());
            }
        }
        let other_value = other_parts.join(", ");
        let other_name = format!("other_tags_{prefix}");
        let other_label = format!("Other tags to {prefix}");
        out.push_str(&format!(
            "<div class=\"filter-autocomplete\">\
             <label for=\"{other_name}\">{other_label}</label>{}</div>",
            autocomplete_input(
                Some(&other_name),
                &other_name,
                Some(&other_value),
                "",
                "tag",
            )
        ));
    }

    out.push_str("<h4 class=\"filter-heading\">More Options</h4>");
    out.push_str(&radio_filter_group(
        "Completion Status",
        "complete",
        COMPLETION_OPTIONS,
        &filter.complete,
    ));
    out.push_str(&radio_filter_group(
        "Crossovers",
        "crossover",
        CROSSOVER_OPTIONS,
        &filter.crossover,
    ));
    let words_open = if filter.words_from.trim().is_empty() && filter.words_to.trim().is_empty() {
        ""
    } else {
        " open"
    };
    out.push_str(&format!(
        "<details class=\"filter-group\"{words_open}><summary>Word Count</summary>\
         <div class=\"range\"><label>From <input type=\"number\" min=\"0\" name=\"words_from\" value=\"{}\"></label>\
         <label>To <input type=\"number\" min=\"0\" name=\"words_to\" value=\"{}\"></label></div></details>",
        escape(&filter.words_from),
        escape(&filter.words_to)
    ));
    out.push_str(&format!(
        "<label class=\"filter-query\">Search within results\
         <input type=\"search\" name=\"q\" value=\"{}\"></label>",
        escape(&filter.query)
    ));
    out.push_str(&format!(
        "<label class=\"filter-sort\">Language<select name=\"language\">{}</select></label>",
        select_options(AO3_LANGUAGES, &filter.language)
    ));
    out.push_str(&format!(
        "<div class=\"filter-submit\"><button type=\"submit\">Sort and Filter</button></div>\
         <p class=\"footnote\"><a href=\"{path}\">Clear Filters</a></p></fieldset></form>"
    ));
    filter_panel(filter.has_active_values(), &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_active_values_flags_every_non_default_filter() {
        assert!(!TagFilter::default().has_active_values());

        let sorted = TagFilter {
            sort: "kudos".into(),
            ..TagFilter::default()
        };
        assert!(sorted.has_active_values());

        let searched = TagFilter {
            query: "mirror".into(),
            ..TagFilter::default()
        };
        assert!(searched.has_active_values());

        let included = TagFilter {
            include: vec!["Explicit".into()],
            ..TagFilter::default()
        };
        assert!(included.has_active_values());

        let excluded = TagFilter {
            exclude: vec!["Fluff".into()],
            ..TagFilter::default()
        };
        assert!(excluded.has_active_values());

        let text_fields: [fn(&mut TagFilter) -> &mut String; 15] = [
            |filter| &mut filter.include_fandoms,
            |filter| &mut filter.include_characters,
            |filter| &mut filter.include_relationships,
            |filter| &mut filter.include_additional,
            |filter| &mut filter.other_include,
            |filter| &mut filter.exclude_fandoms,
            |filter| &mut filter.exclude_characters,
            |filter| &mut filter.exclude_relationships,
            |filter| &mut filter.exclude_additional,
            |filter| &mut filter.other_exclude,
            |filter| &mut filter.complete,
            |filter| &mut filter.crossover,
            |filter| &mut filter.words_from,
            |filter| &mut filter.words_to,
            |filter| &mut filter.language,
        ];
        for field in text_fields {
            let mut filter = TagFilter::default();
            field(&mut filter).push_str("set");
            assert!(filter.has_active_values());
        }

        let blank = TagFilter {
            query: "   ".into(),
            ..TagFilter::default()
        };
        assert!(!blank.has_active_values(), "whitespace is not a filter");
    }

    #[test]
    fn sidebar_wraps_the_form_and_opens_only_in_use_groups() {
        let facets = crate::ao3::search::LiveTagFacets::default();
        let default_html = tag_filter_sidebar("Test", &TagFilter::default(), &facets);
        assert!(
            default_html.starts_with("<details id=\"work-filters-panel\""),
            "{default_html}"
        );
        assert!(
            default_html.contains("data-active=\"false\""),
            "{default_html}"
        );
        assert!(
            default_html
                .contains("<details class=\"filter-group\"><summary>Completion Status</summary>"),
            "{default_html}"
        );
        assert!(
            default_html.contains("<details class=\"filter-group\"><summary>Word Count</summary>"),
            "{default_html}"
        );

        let filter = TagFilter {
            sort: "words".into(),
            complete: "T".into(),
            words_from: "1000".into(),
            ..TagFilter::default()
        };
        let active_html = tag_filter_sidebar("Test", &filter, &facets);
        assert!(
            active_html.contains("data-active=\"true\""),
            "{active_html}"
        );
        assert!(
            active_html.contains(
                "<details class=\"filter-group\" open><summary>Completion Status</summary>"
            ),
            "{active_html}"
        );
        assert!(
            active_html
                .contains("<details class=\"filter-group\" open><summary>Word Count</summary>"),
            "{active_html}"
        );
    }
}
