//! Query string builders for AO3's `/works/search` and `/tags/<tag>/works`.

use crate::ao3::parse::{SpaceMode, percent_encode};

use super::LiveSearchQuery;

/// Encodes a tag name into AO3's URL path segment format (`Tag#to_param`).
///
/// AO3 substitutes `/`, `&`, `.`, `?`, and `#` with `*s*`, `*a*`, `*d*`,
/// `*q*`, and `*h*` before percent-encoding the remaining characters.
pub fn ao3_tag_param(tag: &str) -> String {
    let mut escaped = String::with_capacity(tag.len());
    for ch in tag.chars() {
        match ch {
            '/' => escaped.push_str("*s*"),
            '&' => escaped.push_str("*a*"),
            '.' => escaped.push_str("*d*"),
            '?' => escaped.push_str("*q*"),
            '#' => escaped.push_str("*h*"),
            _ => escaped.push(ch),
        }
    }
    percent_encode(&escaped, SpaceMode::Percent20).replace("%2A", "*")
}

impl LiveSearchQuery {
    pub fn has_criteria(&self) -> bool {
        !self.query.trim().is_empty()
            || self.title.is_some()
            || self.creator.is_some()
            || self.fandom.is_some()
            || self.characters.is_some()
            || self.relationships.is_some()
            || self.additional_tags.is_some()
            || self.rating.is_some()
            || self.warning.is_some()
            || self.category.is_some()
            || !self.include_tags.is_empty()
            || !self.exclude_tags.is_empty()
            || self.language.is_some()
            || self.word_count.is_some()
            || self.hits_min.is_some()
            || self.kudos_min.is_some()
            || self.comments_min.is_some()
            || self.bookmarks_min.is_some()
            || self.complete.is_some()
            || self.crossover.is_some()
            || self.single_chapter
            || self.date.is_some()
    }

    /// AO3's `/works/search` query string for one page.
    pub fn path(&self, page: i64) -> String {
        let mut pairs: Vec<(String, String)> = Vec::new();
        push_work_search(&mut pairs, "query", &self.query);
        for (name, value) in [
            ("title", &self.title),
            ("creators", &self.creator),
            ("revised_at", &self.date),
            ("fandom_names", &self.fandom),
            ("character_names", &self.characters),
            ("relationship_names", &self.relationships),
            ("freeform_names", &self.additional_tags),
        ] {
            if let Some(value) = value {
                push_work_search(&mut pairs, name, value);
            }
        }
        if let Some(complete) = &self.complete {
            push_work_search(&mut pairs, "complete", complete);
        }
        if let Some(crossover) = &self.crossover {
            push_work_search(&mut pairs, "crossover", crossover);
        }
        if self.single_chapter {
            push_work_search(&mut pairs, "single_chapter", "1");
        }
        if let Some(words) = &self.word_count {
            push_work_search(&mut pairs, "word_count", words);
        }
        if let Some(language) = &self.language {
            push_work_search(&mut pairs, "language_id", language);
        }
        for (name, value) in [
            ("hits", self.hits_min),
            ("kudos_count", self.kudos_min),
            ("comments_count", self.comments_min),
            ("bookmarks_count", self.bookmarks_min),
        ] {
            if let Some(value) = value {
                push_work_search(&mut pairs, name, &format!(">{value}"));
            }
        }

        // Rating/warning/category names are canonical tags; AO3's free-text
        // "other tags" filter matches tag names across types.
        let mut tags = self.include_tags.clone();
        for value in [&self.rating, &self.warning, &self.category]
            .into_iter()
            .flatten()
        {
            tags.push(value.clone());
        }
        if !tags.is_empty() {
            push_work_search(&mut pairs, "other_tag_names", &tags.join(", "));
        }
        if !self.exclude_tags.is_empty() {
            push_work_search(
                &mut pairs,
                "excluded_tag_names",
                &self.exclude_tags.join(", "),
            );
        }

        let (sort, direction) = sort_columns(self.sort.as_deref(), self.direction.as_deref());
        // AO3's own default is best match descending; sending it explicitly
        // changes Elasticsearch's tie order, so omit it just like AO3 does.
        if !(sort == "_score" && direction == "desc") {
            push_work_search(&mut pairs, "sort_column", sort);
            push_work_search(&mut pairs, "sort_direction", direction);
        }
        let page = if page > 1 { page } else { self.page.max(1) };
        if page > 1 {
            pairs.push(("page".to_string(), page.to_string()));
        }
        format!("/works/search?{}", encode_query_pairs(&pairs))
    }

    /// AO3's native `/tags/<tag>/works` URL for one filtered tag page.
    ///
    /// Unlike `/works/search`, AO3's tag works index runs with `faceted: true`
    /// and renders `#work-filters` with global top-10 popular tags + counts.
    pub fn tag_works_path(
        &self,
        tag: &str,
        words_from: Option<&str>,
        words_to: Option<&str>,
        page: i64,
    ) -> String {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut other_tags: Vec<String> = self
            .include_tags
            .iter()
            .filter(|item| !item.eq_ignore_ascii_case(tag))
            .cloned()
            .collect();
        for value in [&self.rating, &self.warning, &self.category]
            .into_iter()
            .flatten()
        {
            if !value.eq_ignore_ascii_case(tag) {
                other_tags.push(value.clone());
            }
        }

        let (sort, _) = sort_columns(self.sort.as_deref(), self.direction.as_deref());
        let has_filters = !self.query.trim().is_empty()
            || !other_tags.is_empty()
            || !self.exclude_tags.is_empty()
            || self.complete.is_some()
            || self.crossover.is_some()
            || self.language.is_some()
            || words_from.is_some_and(|v| !v.trim().is_empty())
            || words_to.is_some_and(|v| !v.trim().is_empty())
            || self.word_count.is_some();

        // AO3's default sort on `/tags/<tag>/works` is `revised_at`. Keep the
        // unfiltered URL completely clean so it hits AO3's cached tag index.
        if sort != "revised_at" || has_filters {
            push_work_search(&mut pairs, "sort_column", sort);
        }
        push_work_search(&mut pairs, "query", &self.query);

        if !other_tags.is_empty() {
            push_work_search(&mut pairs, "other_tag_names", &other_tags.join(", "));
        }
        if !self.exclude_tags.is_empty() {
            push_work_search(
                &mut pairs,
                "excluded_tag_names",
                &self.exclude_tags.join(", "),
            );
        }
        if let Some(complete) = &self.complete {
            push_work_search(&mut pairs, "complete", complete);
        }
        if let Some(crossover) = &self.crossover {
            push_work_search(&mut pairs, "crossover", crossover);
        }
        if let Some(from) = words_from {
            push_work_search(&mut pairs, "words_from", from);
        }
        if let Some(to) = words_to {
            push_work_search(&mut pairs, "words_to", to);
        }
        if words_from.is_none()
            && words_to.is_none()
            && let Some(words) = &self.word_count
        {
            push_work_search(&mut pairs, "word_count", words);
        }
        if let Some(language) = &self.language {
            push_work_search(&mut pairs, "language_id", language);
        }

        let page = if page > 1 { page } else { self.page.max(1) };
        if page > 1 {
            pairs.push(("page".to_string(), page.to_string()));
        }

        let base = format!("/tags/{}/works", ao3_tag_param(tag));
        if pairs.is_empty() {
            return base;
        }
        format!("{base}?{}", encode_query_pairs(&pairs))
    }

    /// Query string for the form/results links (without AO3's bracket syntax).
    pub fn form_query(&self) -> String {
        let encode = |value: &str| percent_encode(value, SpaceMode::Percent20);
        let mut parts: Vec<String> = Vec::new();
        let mut push = |name: &str, value: &str| {
            if !value.trim().is_empty() {
                parts.push(format!("{name}={}", encode(value.trim())));
            }
        };
        push("q", &self.query);
        for (name, value) in [
            ("title", &self.title),
            ("creator", &self.creator),
            ("date", &self.date),
            ("fandom", &self.fandom),
            ("characters", &self.characters),
            ("relationships", &self.relationships),
            ("additional_tags", &self.additional_tags),
            ("rating", &self.rating),
            ("warning", &self.warning),
            ("category", &self.category),
            ("language", &self.language),
            ("words", &self.word_count),
            ("complete", &self.complete),
            ("crossover", &self.crossover),
            ("sort", &self.sort),
            ("direction", &self.direction),
        ] {
            if let Some(value) = value {
                push(name, value);
            }
        }
        if self.single_chapter {
            parts.push("single_chapter=1".to_string());
        }
        for (name, value) in [
            ("hits_min", self.hits_min),
            ("kudos_min", self.kudos_min),
            ("comments_min", self.comments_min),
            ("bookmarks_min", self.bookmarks_min),
        ] {
            if let Some(value) = value {
                parts.push(format!("{name}={value}"));
            }
        }
        if !self.include_tags.is_empty() {
            parts.push(format!(
                "include_tags={}",
                encode(&self.include_tags.join(","))
            ));
        }
        if !self.exclude_tags.is_empty() {
            parts.push(format!(
                "exclude_tags={}",
                encode(&self.exclude_tags.join(","))
            ));
        }
        parts.join("&")
    }
}

fn push_work_search(pairs: &mut Vec<(String, String)>, name: &str, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        pairs.push((format!("work_search[{name}]"), trimmed.to_string()));
    }
}

fn encode_query_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encode(key, SpaceMode::Percent20),
                percent_encode(value, SpaceMode::Percent20)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn sort_columns(sort: Option<&str>, direction: Option<&str>) -> (&'static str, &'static str) {
    let column = match sort.unwrap_or("best_match") {
        "date" => "revised_at",
        "created_at" => "created_at",
        "words" => "word_count",
        "kudos" => "kudos_count",
        "hits" => "hits",
        "comments" => "comments_count",
        "bookmarks" => "bookmarks_count",
        "title" => "title_to_sort_on",
        _ => "_score",
    };
    let direction = match direction {
        Some("asc") => "asc",
        Some("desc") => "desc",
        _ if column == "title_to_sort_on" => "asc",
        _ => "desc",
    };
    (column, direction)
}
