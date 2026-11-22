//! Offline lists: browse/facets, fandoms, subscriptions, bookmarks, reading
//! history and cache statistics. Every query reads SQLite only.

use std::collections::{BTreeMap, HashMap, HashSet};

use sqlx::Row;

use crate::error::{Error, Result};

use super::{Store, StoredWork};

/// Which local reading state a browse filter wants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReadingStatus {
    #[default]
    All,
    InProgress,
    ForLater,
    Read,
    Bookmarked,
}

impl ReadingStatus {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some("in_progress") => Self::InProgress,
            Some("for_later") => Self::ForLater,
            Some("read") => Self::Read,
            Some("bookmarked") => Self::Bookmarked,
            _ => Self::All,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::InProgress => "in_progress",
            Self::ForLater => "for_later",
            Self::Read => "read",
            Self::Bookmarked => "bookmarked",
        }
    }
}

/// Browse order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BrowseSort {
    #[default]
    FetchedDesc,
    WordsDesc,
    KudosDesc,
    TitleAsc,
}

impl BrowseSort {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some("words_desc") => Self::WordsDesc,
            Some("kudos_desc") => Self::KudosDesc,
            Some("title_asc") => Self::TitleAsc,
            _ => Self::FetchedDesc,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FetchedDesc => "fetched_desc",
            Self::WordsDesc => "words_desc",
            Self::KudosDesc => "kudos_desc",
            Self::TitleAsc => "title_asc",
        }
    }
}

/// Filters for `GET /works` and `GET /tags/{tag}/works`.
#[derive(Debug, Clone)]
pub struct WorkFilter {
    pub rating: Option<String>,
    pub fandom: Option<String>,
    pub tag: Option<String>,
    pub language: Option<String>,
    pub reading_status: ReadingStatus,
    pub sort: BrowseSort,
    pub page: i64,
    pub per_page: i64,
}

impl Default for WorkFilter {
    fn default() -> Self {
        Self {
            rating: None,
            fandom: None,
            tag: None,
            language: None,
            reading_status: ReadingStatus::All,
            sort: BrowseSort::FetchedDesc,
            page: 1,
            per_page: 20,
        }
    }
}

/// One page of cached works plus the facet values for the sidebar.
#[derive(Debug, Clone)]
pub struct BrowseResult {
    pub works: Vec<StoredWork>,
    pub total: usize,
    pub page: i64,
    pub per_page: i64,
    pub fandoms: Vec<(String, usize)>,
    pub ratings: Vec<(String, usize)>,
    pub languages: Vec<(String, usize)>,
}

/// A subscription ledger row joined with the cached work title, when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionEntry {
    pub target_kind: String,
    pub target_id: String,
    pub subscription_id: Option<i64>,
    pub state: String,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub updated_at: i64,
}

/// A bookmark ledger row joined with the cached work title, when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkEntry {
    pub work_id: String,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub bookmark_id: Option<i64>,
    pub created_at: i64,
}

/// One work with either saved progress, a reading flag, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingEntry {
    pub work_id: String,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub progress_chapter: Option<i64>,
    pub reading_state: Option<String>,
    pub updated_at: i64,
}

/// Counters for the dashboard footer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub works: i64,
    pub chapters: i64,
    pub subscriptions: i64,
    pub bookmarks: i64,
    pub kudos: i64,
    pub recent_http_errors: i64,
}

impl Store {
    /// Fandoms present in the cache, most cached works first.
    pub async fn list_fandoms(&self) -> Result<Vec<(String, usize)>> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for work in self.all_works().await? {
            for fandom in work.tags.fandoms {
                *counts.entry(fandom).or_insert(0) += 1;
            }
        }
        let mut fandoms: Vec<(String, usize)> = counts.into_iter().collect();
        fandoms.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(fandoms)
    }

    pub async fn browse_works(&self, account: &str, filter: &WorkFilter) -> Result<BrowseResult> {
        let all = self.all_works().await?;
        let progress = self.progress_map().await?;
        let flags = self.reading_flags(account).await?;
        let bookmarked = self.bookmarked_ids(account).await?;

        let fandoms = facet(&all, |work| work.tags.fandoms.iter());
        let ratings = facet(&all, |work| work.rating.iter());
        let languages = facet(&all, |work| work.language.iter());

        let mut works: Vec<StoredWork> = all
            .into_iter()
            .filter(|work| {
                filter.rating.as_ref().is_none_or(|rating| {
                    work.rating
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(rating))
                }) && filter.language.as_ref().is_none_or(|language| {
                    work.language
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(language))
                }) && filter.fandom.as_ref().is_none_or(|fandom| {
                    work.tags
                        .fandoms
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(fandom))
                }) && filter.tag.as_ref().is_none_or(|tag| {
                    let all_tags = work
                        .tags
                        .fandoms
                        .iter()
                        .chain(&work.tags.relationships)
                        .chain(&work.tags.characters)
                        .chain(&work.tags.additional)
                        .chain(&work.tags.warnings)
                        .chain(&work.tags.categories);
                    all_tags
                        .into_iter()
                        .any(|value| value.eq_ignore_ascii_case(tag))
                }) && match filter.reading_status {
                    ReadingStatus::All => true,
                    ReadingStatus::InProgress => progress.contains_key(&work.ao3_id),
                    ReadingStatus::ForLater => flags
                        .get(&work.ao3_id)
                        .is_some_and(|state| state == "for_later"),
                    ReadingStatus::Read => {
                        flags.get(&work.ao3_id).is_some_and(|state| state == "read")
                    }
                    ReadingStatus::Bookmarked => bookmarked.contains(&work.ao3_id),
                }
            })
            .collect();

        match filter.sort {
            BrowseSort::FetchedDesc => {
                works.sort_by(|a, b| {
                    b.fetched_at
                        .cmp(&a.fetched_at)
                        .then(b.ao3_id.cmp(&a.ao3_id))
                });
            }
            BrowseSort::WordsDesc => works.sort_by(|a, b| {
                b.word_count
                    .unwrap_or(0)
                    .cmp(&a.word_count.unwrap_or(0))
                    .then(a.ao3_id.cmp(&b.ao3_id))
            }),
            BrowseSort::KudosDesc => works.sort_by(|a, b| {
                b.kudos
                    .unwrap_or(0)
                    .cmp(&a.kudos.unwrap_or(0))
                    .then(a.ao3_id.cmp(&b.ao3_id))
            }),
            BrowseSort::TitleAsc => works.sort_by(|a, b| {
                a.title
                    .to_ascii_lowercase()
                    .cmp(&b.title.to_ascii_lowercase())
            }),
        }

        let total = works.len();
        let per_page = filter.per_page.clamp(1, 100);
        let page = filter.page.max(1);
        let skip = usize::try_from((page - 1).saturating_mul(per_page)).unwrap_or(usize::MAX);
        let works = works
            .into_iter()
            .skip(skip)
            .take(per_page as usize)
            .collect();

        Ok(BrowseResult {
            works,
            total,
            page,
            per_page,
            fandoms,
            ratings,
            languages,
        })
    }

    pub async fn list_subscriptions(&self, account: &str) -> Result<Vec<SubscriptionEntry>> {
        let rows = sqlx::query(
            "SELECT s.target_kind, s.target_id, s.subscription_id, s.state, s.updated_at,
                    w.title, w.authors_json
             FROM subscriptions s
             LEFT JOIN works w ON s.target_kind = 'Work' AND w.ao3_id = s.target_id
             WHERE s.account = ?1
             ORDER BY (s.state = 'active') DESC, s.updated_at DESC",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("list subscriptions: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| SubscriptionEntry {
                target_kind: row.get("target_kind"),
                target_id: row.get("target_id"),
                subscription_id: row.get("subscription_id"),
                state: row.get("state"),
                title: row.get("title"),
                authors: authors_from_row(row),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    pub async fn list_bookmarks(&self, account: &str) -> Result<Vec<BookmarkEntry>> {
        let rows = sqlx::query(
            "SELECT b.work_id, b.bookmark_id, b.created_at, w.title, w.authors_json
             FROM bookmarks b
             LEFT JOIN works w ON w.ao3_id = b.work_id
             WHERE b.account = ?1
             ORDER BY b.created_at DESC",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("list bookmarks: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| BookmarkEntry {
                work_id: row.get("work_id"),
                title: row.get("title"),
                authors: authors_from_row(row),
                bookmark_id: row.get("bookmark_id"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    pub async fn list_reading_history(&self, account: &str) -> Result<Vec<ReadingEntry>> {
        let progress = self.progress_map().await?;
        let mut entries: HashMap<String, ReadingEntry> = HashMap::new();

        for (work_id, (chapter, updated_at)) in progress {
            entries.insert(
                work_id.clone(),
                ReadingEntry {
                    work_id,
                    title: None,
                    authors: Vec::new(),
                    progress_chapter: Some(chapter),
                    reading_state: None,
                    updated_at,
                },
            );
        }

        let rows = sqlx::query(
            "SELECT f.work_id, f.state, f.updated_at, w.title, w.authors_json
             FROM reading_flags f
             LEFT JOIN works w ON w.ao3_id = f.work_id
             WHERE f.account = ?1",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("list reading flags: {e}")))?;
        for row in &rows {
            let work_id: String = row.get("work_id");
            let entry = entries
                .entry(work_id.clone())
                .or_insert_with(|| ReadingEntry {
                    work_id,
                    title: None,
                    authors: Vec::new(),
                    progress_chapter: None,
                    reading_state: None,
                    updated_at: 0,
                });
            entry.reading_state = row.get("state");
            entry.updated_at = entry.updated_at.max(row.get("updated_at"));
            entry.title = row.get("title");
            entry.authors = authors_from_row(row);
        }

        let mut entries: Vec<ReadingEntry> = entries.into_values().collect();
        entries.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.work_id.cmp(&b.work_id))
        });
        Ok(entries)
    }

    pub async fn cache_stats(&self) -> Result<CacheStats> {
        let works = self.count_table("works").await?;
        let chapters = self.count_table("chapters").await?;
        let subscriptions = self.count_table("subscriptions").await?;
        let bookmarks = self.count_table("bookmarks").await?;
        let kudos = self.count_table("kudos_sent").await?;
        let recent_http_errors = self.count_table("http_error_log").await?;
        Ok(CacheStats {
            works,
            chapters,
            subscriptions,
            bookmarks,
            kudos,
            recent_http_errors,
        })
    }

    async fn count_table(&self, table: &str) -> Result<i64> {
        // Literal statements only: sqlx 0.9 requires `&'static str`, and the
        // match keeps an accidental table name from ever reaching SQL.
        let query = match table {
            "works" => "SELECT COUNT(*) FROM works",
            "chapters" => "SELECT COUNT(*) FROM chapters",
            "subscriptions" => "SELECT COUNT(*) FROM subscriptions",
            "bookmarks" => "SELECT COUNT(*) FROM bookmarks",
            "kudos_sent" => "SELECT COUNT(*) FROM kudos_sent",
            "http_error_log" => "SELECT COUNT(*) FROM http_error_log",
            other => return Err(Error::Store(format!("unknown table {other}"))),
        };
        sqlx::query_scalar(query)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("count {table}: {e}")))
    }

    async fn progress_map(&self) -> Result<HashMap<String, (i64, i64)>> {
        let rows = sqlx::query(
            "SELECT work_id, chapter_idx, updated_at FROM progress WHERE account_id = 'local'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("progress map: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    row.get("work_id"),
                    (row.get("chapter_idx"), row.get("updated_at")),
                )
            })
            .collect())
    }

    async fn reading_flags(&self, account: &str) -> Result<HashMap<String, String>> {
        let rows = sqlx::query("SELECT work_id, state FROM reading_flags WHERE account = ?1")
            .bind(account)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("reading flags: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| (row.get("work_id"), row.get("state")))
            .collect())
    }

    async fn bookmarked_ids(&self, account: &str) -> Result<HashSet<String>> {
        let rows = sqlx::query("SELECT work_id FROM bookmarks WHERE account = ?1")
            .bind(account)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("bookmark ids: {e}")))?;
        Ok(rows.iter().map(|row| row.get("work_id")).collect())
    }
}

fn authors_from_row(row: &sqlx::sqlite::SqliteRow) -> Vec<String> {
    row.get::<Option<String>, _>("authors_json")
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Counts values of one facet across all works (case preserved, sorted).
fn facet<'a, I>(
    works: &'a [StoredWork],
    values: impl Fn(&'a StoredWork) -> I,
) -> Vec<(String, usize)>
where
    I: Iterator<Item = &'a String>,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for work in works {
        for value in values(work) {
            *counts.entry(value.clone()).or_insert(0) += 1;
        }
    }
    let mut facets: Vec<(String, usize)> = counts.into_iter().collect();
    facets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    facets
}
