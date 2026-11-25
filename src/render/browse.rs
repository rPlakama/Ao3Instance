//! Offline browse with a server-rendered filter sidebar and pagination.

use super::blurb::work_blurb;
use super::chrome::{PageContext, layout_with_class};
use super::components::filter_panel;
use super::escape;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::store::{BrowseResult, BrowseSort, ReadingStatus, WorkFilter};

pub fn browse_page(ctx: &PageContext, result: &BrowseResult, filter: &WorkFilter) -> String {
    let start = (result.page - 1) * result.per_page + 1;
    let end = start + result.works.len() as i64 - 1;
    let noun = if result.total == 1 { "Work" } else { "Works" };
    let heading = if let Some(tag) = filter.tag.as_deref() {
        if result.total > result.per_page as usize {
            format!(
                "{start} - {end} of {} {noun} in {}",
                result.total,
                escape(tag)
            )
        } else {
            format!("{} {noun} in {}", result.total, escape(tag))
        }
    } else if let Some(fandom) = filter.fandom.as_deref() {
        format!("{} {noun} in {}", result.total, escape(fandom))
    } else {
        "Cached works".to_string()
    };
    let mut body = format!("<h2 class=\"heading\">{heading}</h2>");
    if filter.tag.is_some() || filter.fandom.is_some() {
        body.push_str(
            "<div class=\"navigation actions module\">\
             <h3 class=\"landmark heading\">Navigation and Actions</h3>\
             <ul class=\"user navigation actions\" role=\"navigation\">\
             <li class=\"active\"><span class=\"current\">Works</span></li>\
             <li><a href=\"/bookmarks\">Bookmarks</a></li>\
             <li><a href=\"/works\">Clear tag</a></li>",
        );
        if let Some(tag) = filter.tag.as_deref().or(filter.fandom.as_deref()) {
            body.push_str(&format!(
                "<li><a href=\"/works/search?include_tags={}\">Search this tag on AO3</a></li>",
                percent_encode(tag, SpaceMode::Percent20)
            ));
        }
        body.push_str("</ul></div>");
    } else {
        body.push_str(
            "<p class=\"note\">Browse is offline: filters never call archiveofourown.org.</p>",
        );
    }

    let pager = crate::render::pagination(
        result.page,
        result.per_page,
        i64::try_from(result.total).unwrap_or(i64::MAX),
        "/works",
        &query(filter),
    );
    body.push_str("<div class=\"browse-grid\"><section class=\"browse-results\">");
    if result.works.is_empty() {
        body.push_str(
            "<p class=\"muted\">No cached work matches these filters. \
             <a href=\"/works\">Clear filters</a> or open a work from the start page.</p>",
        );
    } else {
        body.push_str(&format!(
            "<p class=\"meta\">showing {}–{} of {}</p>",
            (result.page - 1) * result.per_page + 1,
            (result.page - 1) * result.per_page + result.works.len() as i64,
            result.total
        ));
        body.push_str(&pager);
        body.push_str("<h3 class=\"landmark heading\">Listing Works</h3>");
        body.push_str("<ol class=\"work index group\">");
        for work in &result.works {
            body.push_str(&work_blurb(work, None));
        }
        body.push_str("</ol>");
        body.push_str(&pager);
    }
    body.push_str("</section>");
    body.push_str(&sidebar(result, filter));
    body.push_str("</div>");
    layout_with_class(ctx, "works-index filtered", &body)
}

fn sidebar(result: &BrowseResult, filter: &WorkFilter) -> String {
    let mut form = String::from(
        "<form id=\"work-filters\" class=\"filters\" method=\"get\" action=\"/works\">\
         <h3 class=\"landmark heading\">Filters</h3><fieldset>\
         <legend>Filter results:</legend><dl>\
         <dt class=\"landmark\">Submit</dt>\
         <dd class=\"submit actions\"><button type=\"submit\">Sort and Filter</button></dd>\
         <dt class=\"sort\"><label for=\"browse_sort\">Sort by</label></dt>\
         <dd class=\"sort\"><select id=\"browse_sort\" name=\"sort\">",
    );
    for sort in [
        BrowseSort::FetchedDesc,
        BrowseSort::WordsDesc,
        BrowseSort::KudosDesc,
        BrowseSort::TitleAsc,
    ] {
        let label = match sort {
            BrowseSort::FetchedDesc => "Recently cached",
            BrowseSort::WordsDesc => "Word count",
            BrowseSort::KudosDesc => "Kudos",
            BrowseSort::TitleAsc => "Title A–Z",
        };
        form.push_str(&option(sort.as_str(), label, filter.sort == sort));
    }
    form.push_str("</select></dd>");

    form.push_str(
        "<dt><label for=\"browse_rating\">Rating</label></dt>\
         <dd><select id=\"browse_rating\" name=\"rating\">",
    );
    form.push_str(&option("", "Any rating", filter.rating.is_none()));
    for (rating, count) in &result.ratings {
        form.push_str(&option(
            rating,
            &format!("{rating} ({count})"),
            filter.rating.as_deref() == Some(rating.as_str()),
        ));
    }
    form.push_str("</select></dd>");

    form.push_str(&format!(
        "<dt><label for=\"browse_author\">Author</label></dt>\
         <dd><input id=\"browse_author\" type=\"text\" name=\"author\" value=\"{}\" \
         placeholder=\"exact author\"></dd>",
        escape(filter.author.as_deref().unwrap_or(""))
    ));

    form.push_str(
        "<dt><label for=\"browse_fandom\">Fandom</label></dt>\
         <dd><select id=\"browse_fandom\" name=\"fandom\">",
    );
    form.push_str(&option("", "Any fandom", filter.fandom.is_none()));
    for (fandom, count) in &result.fandoms {
        form.push_str(&option(
            fandom,
            &format!("{fandom} ({count})"),
            filter.fandom.as_deref() == Some(fandom.as_str()),
        ));
    }
    form.push_str("</select></dd>");

    form.push_str(
        "<dt><label for=\"browse_language\">Language</label></dt>\
         <dd><select id=\"browse_language\" name=\"language\">",
    );
    form.push_str(&option("", "Any language", filter.language.is_none()));
    for (language, count) in &result.languages {
        form.push_str(&option(
            language,
            &format!("{language} ({count})"),
            filter.language.as_deref() == Some(language.as_str()),
        ));
    }
    form.push_str("</select></dd>");

    form.push_str(
        "<dt><label for=\"browse_status\">Reading status</label></dt>\
         <dd><select id=\"browse_status\" name=\"status\">",
    );
    for status in [
        ReadingStatus::All,
        ReadingStatus::InProgress,
        ReadingStatus::ForLater,
        ReadingStatus::Read,
        ReadingStatus::Bookmarked,
    ] {
        let label = match status {
            ReadingStatus::All => "Everything",
            ReadingStatus::InProgress => "In progress",
            ReadingStatus::ForLater => "Marked for later",
            ReadingStatus::Read => "Read",
            ReadingStatus::Bookmarked => "Bookmarked",
        };
        form.push_str(&option(
            status.as_str(),
            label,
            filter.reading_status == status,
        ));
    }
    form.push_str("</select></dd>");

    form.push_str(&format!(
        "<dt><label for=\"browse_tag\">Tag</label></dt>\
         <dd><input id=\"browse_tag\" type=\"text\" name=\"tag\" value=\"{}\" \
         placeholder=\"exact tag\"></dd>",
        escape(filter.tag.as_deref().unwrap_or(""))
    ));

    form.push_str(
        "<dt class=\"landmark\">Submit</dt>\
         <dd class=\"submit actions\"><button type=\"submit\">Sort and Filter</button></dd></dl>\
         <p class=\"footnote\"><a href=\"/works\">Clear Filters</a></p></fieldset></form>",
    );
    filter_panel(filter.has_active_filters(), &form)
}

fn option(value: &str, label: &str, selected: bool) -> String {
    format!(
        "<option value=\"{}\"{}>{}</option>",
        escape(value),
        if selected { " selected" } else { "" },
        escape(label)
    )
}

fn query(filter: &WorkFilter) -> String {
    let mut parts: Vec<String> = Vec::new();
    let encode = |value: &str| percent_encode(value, SpaceMode::Percent20);
    if let Some(rating) = &filter.rating {
        parts.push(format!("rating={}", encode(rating)));
    }
    if let Some(author) = &filter.author {
        parts.push(format!("author={}", encode(author)));
    }
    if let Some(fandom) = &filter.fandom {
        parts.push(format!("fandom={}", encode(fandom)));
    }
    if let Some(tag) = &filter.tag {
        parts.push(format!("tag={}", encode(tag)));
    }
    if let Some(language) = &filter.language {
        parts.push(format!("language={}", encode(language)));
    }
    parts.push(format!("status={}", filter.reading_status.as_str()));
    parts.push(format!("sort={}", filter.sort.as_str()));
    parts.join("&")
}

/// Parses the filters from `GET /works` (and the tag alias) into one struct.
#[allow(
    clippy::too_many_arguments,
    reason = "each argument is one independent AO3 filter query parameter"
)]
pub fn filter_from(
    rating: Option<String>,
    author: Option<String>,
    fandom: Option<String>,
    tag: Option<String>,
    language: Option<String>,
    status: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
) -> WorkFilter {
    let clean = |value: Option<String>| {
        value.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        })
    };
    WorkFilter {
        rating: clean(rating),
        author: clean(author),
        fandom: clean(fandom),
        tag: clean(tag),
        language: clean(language),
        reading_status: ReadingStatus::parse(status.as_deref()),
        sort: BrowseSort::parse(sort.as_deref()),
        page: page.unwrap_or(1).max(1),
        per_page: 20,
    }
}
