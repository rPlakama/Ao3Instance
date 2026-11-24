use super::*;

const RESULTS: &str = include_str!("../../../tests/fixtures/search_results.html");

#[test]
fn parses_ao3_search_results() {
    let results = parse_search_results(RESULTS).expect("parse");
    assert_eq!(results.total, 2);
    assert_eq!(results.summary, "mirror sort by: best match descending");
    assert_eq!(results.works.len(), 2);

    let first = &results.works[0];
    assert_eq!(first.ao3_id, "111");
    assert_eq!(first.title, "A Mirror of Small Things");
    assert!(first.restricted);
    assert_eq!(first.collections, Some(2));
    assert_eq!(first.series.len(), 1);
    assert_eq!(first.series[0].part.as_deref(), Some("2"));
    assert_eq!(first.series[0].title, "purge_king basement");
    assert_eq!(first.series[0].href.as_deref(), Some("/series/5383226"));
    assert_eq!(first.authors, ["tester"]);
    assert_eq!(first.fandoms, ["Test Fandom"]);
    assert_eq!(first.rating.as_deref(), Some("Explicit"));
    assert_eq!(first.warnings, ["Graphic Depictions Of Violence"]);
    assert_eq!(first.categories, ["F/M"]);
    assert_eq!(first.complete, Some(true));
    assert_eq!(first.relationships, ["A/B"]);
    assert_eq!(first.characters, ["Character A"]);
    assert_eq!(first.additional_tags, ["Fluff"]);
    assert_eq!(first.datetime, "02 Jan 2025");
    assert_eq!(first.words, Some(4_321));
    assert_eq!(first.chapters.as_deref(), Some("2/2"));
    assert_eq!(first.comments, Some(57));
    assert_eq!(first.kudos, Some(1_024));
    assert_eq!(first.bookmarks, Some(12));
    assert_eq!(first.hits, Some(9_876));
    assert!(
        first
            .summary_html
            .as_deref()
            .is_some_and(|html| html.contains("cartographer"))
    );

    let second = &results.works[1];
    assert_eq!(second.complete, Some(false));
    assert_eq!(second.chapters.as_deref(), Some("1/?"));
    assert_eq!(second.comments, None);
    assert!(!second.restricted);
    assert!(second.series.is_empty());
}

#[test]
fn builds_ao3_query_parameters() {
    let query = LiveSearchQuery {
        query: "mirror".into(),
        creator: Some("tester".into()),
        fandom: Some("Test Fandom".into()),
        rating: Some("Explicit".into()),
        include_tags: vec!["Fluff".into(), "Angst".into()],
        exclude_tags: vec!["Major Character Death".into()],
        complete: Some("T".into()),
        single_chapter: true,
        sort: Some("kudos".into()),
        direction: Some("asc".into()),
        ..LiveSearchQuery::default()
    };
    let path = query.path(2);
    assert!(path.starts_with("/works/search?"), "{path}");
    assert!(path.contains("work_search%5Bquery%5D=mirror"), "{path}");
    assert!(path.contains("work_search%5Bcreators%5D=tester"), "{path}");
    assert!(
        path.contains("work_search%5Bfandom_names%5D=Test%20Fandom"),
        "{path}"
    );
    assert!(
        path.contains("work_search%5Bother_tag_names%5D=Fluff%2C%20Angst%2C%20Explicit"),
        "{path}"
    );
    assert!(
        path.contains("work_search%5Bexcluded_tag_names%5D=Major%20Character%20Death"),
        "{path}"
    );
    assert!(path.contains("work_search%5Bcomplete%5D=T"), "{path}");
    assert!(path.contains("work_search%5Bsingle_chapter%5D=1"), "{path}");
    assert!(
        path.contains("work_search%5Bsort_column%5D=kudos_count"),
        "{path}"
    );
    assert!(
        path.contains("work_search%5Bsort_direction%5D=asc"),
        "{path}"
    );
    assert!(path.contains("page=2"), "{path}");
    assert!(query.has_criteria());
}

#[test]
fn empty_query_has_no_criteria() {
    assert!(!LiveSearchQuery::default().has_criteria());
}

#[test]
fn tag_page_totals_parse_from_range_headings() {
    assert_eq!(
        parse_total_from_heading("1 - 20 of 9,432 Works in Re:Zero"),
        Some(9_432)
    );
    assert_eq!(parse_total_from_heading("1 Work in Fluff"), Some(1));
    assert_eq!(parse_total_from_heading("Cached works"), None);
}

#[test]
fn default_best_match_omits_sort_and_commit() {
    let query = LiveSearchQuery {
        query: "re zero".into(),
        ..LiveSearchQuery::default()
    };
    let path = query.path(1);
    assert!(!path.contains("sort_column"), "{path}");
    assert!(!path.contains("commit"), "{path}");
    assert!(path.contains("work_search%5Bquery%5D=re%20zero"), "{path}");
}

#[test]
fn encodes_ao3_tag_params_and_tag_works_paths() {
    assert_eq!(ao3_tag_param("A/B"), "A*s*B");
    assert_eq!(
        ao3_tag_param("Re:Zero & Friends? #1.0"),
        "Re%3AZero%20*a*%20Friends*q*%20*h*1*d*0"
    );

    let default_query = LiveSearchQuery {
        sort: Some("date".into()),
        direction: Some("desc".into()),
        ..LiveSearchQuery::default()
    };
    assert_eq!(
        default_query.tag_works_path("A/B", None, None, 1),
        "/tags/A*s*B/works"
    );
    assert_eq!(
        default_query.tag_works_path("A/B", None, None, 2),
        "/tags/A*s*B/works?page=2"
    );

    let filtered = LiveSearchQuery {
        include_tags: vec!["A/B".into(), "Fluff".into()],
        exclude_tags: vec!["Angst".into()],
        sort: Some("kudos".into()),
        complete: Some("T".into()),
        ..LiveSearchQuery::default()
    };
    let path = filtered.tag_works_path("A/B", Some("1000"), Some("5000"), 1);
    assert!(path.starts_with("/tags/A*s*B/works?"), "{path}");
    assert!(
        path.contains("work_search%5Bsort_column%5D=kudos_count"),
        "{path}"
    );
    assert!(
        path.contains("work_search%5Bother_tag_names%5D=Fluff"),
        "{path}"
    );
    assert!(
        !path.contains("A%2FB"),
        "page tag should not duplicate: {path}"
    );
    assert!(
        path.contains("work_search%5Bexcluded_tag_names%5D=Angst"),
        "{path}"
    );
    assert!(path.contains("work_search%5Bwords_from%5D=1000"), "{path}");
    assert!(path.contains("work_search%5Bwords_to%5D=5000"), "{path}");
}

#[test]
fn parses_work_filters_popular_tag_facets() {
    let html = r#"
        <html><body>
        <h2 class="heading">1 - 2 of 2 Works in Re:Zero</h2>
        <form id="work-filters">
          <dd id="include_rating_tags"><ul>
            <li><label><input type="radio" value="10"><span class="indicator"></span><span>Teen And Up Audiences (1,234)</span></label></li>
          </ul></dd>
          <dd id="include_relationship_tags"><ul>
            <li><label><input type="checkbox" value="1"><span class="indicator"></span><span>Emilia (Re:Zero)/Natsuki Subaru (987)</span></label></li>
            <li><label><input type="checkbox" value="2"><span class="indicator"></span><span>Rem (Re:Zero)/Natsuki Subaru (456)</span></label></li>
          </ul></dd>
          <dd id="include_freeform_tags"><ul>
            <li><label><input type="checkbox" value="3"><span class="indicator"></span><span>Fluff (Anime)</span></label></li>
          </ul></dd>
        </form>
        <ol class="work index group">
          <li id="work_42" class="work blurb group">
            <h4 class="heading"><a href="/works/42">Sample</a></h4>
          </li>
        </ol>
        </body></html>
    "#;
    let parsed = parse_search_results(html).expect("parse");
    assert_eq!(parsed.facets.ratings.len(), 1);
    assert_eq!(parsed.facets.ratings[0].name, "Teen And Up Audiences");
    assert_eq!(parsed.facets.ratings[0].count, Some(1_234));
    assert_eq!(parsed.facets.relationships.len(), 2);
    assert_eq!(
        parsed.facets.relationships[0].name,
        "Emilia (Re:Zero)/Natsuki Subaru"
    );
    assert_eq!(parsed.facets.relationships[0].count, Some(987));
    assert_eq!(parsed.facets.freeforms.len(), 1);
    assert_eq!(parsed.facets.freeforms[0].name, "Fluff (Anime)");
    assert_eq!(parsed.facets.freeforms[0].count, None);
}

#[test]
fn computes_fallback_facets_from_works() {
    let parsed = parse_search_results(RESULTS).expect("parse");
    assert!(parsed.facets.is_empty());
    let tallied = LiveTagFacets::from_works(&parsed.works);
    assert!(
        tallied
            .fandoms
            .iter()
            .any(|f| f.name == "Test Fandom" && f.count == Some(1))
    );
    assert_eq!(tallied.relationships[0].name, "A/B");
    assert_eq!(tallied.characters[0].name, "Character A");
    assert_eq!(tallied.freeforms[0].name, "Fluff");
}
