//! Cached work rows, full-text search and the local reading position.

use super::{
    Progress, SearchFilter, SearchPage, Store, StoredChapter, StoredWork, TagGroups, WorkSummary,
    now_secs,
};
use crate::ao3::work::WorkPage;
use sqlx::Row;

use crate::error::{Error, Result};

impl Store {
    /// Upserts a fetched work and replaces its chapters in one transaction.
    pub async fn upsert_work(&self, page: &WorkPage, content_hash: &str) -> Result<()> {
        let authors = serde_json::to_string(&page.authors)
            .map_err(|e| Error::Store(format!("serialize authors: {e}")))?;
        let tags = TagGroups {
            fandoms: page.fandoms.clone(),
            relationships: page.relationships.clone(),
            characters: page.characters.clone(),
            additional: page.additional_tags.clone(),
            warnings: page.warnings.clone(),
            categories: page.categories.clone(),
        };
        let tags_json = serde_json::to_string(&tags)
            .map_err(|e| Error::Store(format!("serialize tags: {e}")))?;
        let fetched_at = now_secs();

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Store(format!("begin tx: {e}")))?;

        sqlx::query(
            "INSERT INTO works (ao3_id, title, authors_json, summary, tags_json, rating, language,
                                published_at, complete, word_count, kudos, comment_count, hits,
                                bookmarks, chapter_count, content_hash, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(ao3_id) DO UPDATE SET
               title = excluded.title,
               authors_json = excluded.authors_json,
               -- A partial page (chapter 2+) has no summary block; keep the cached one.
               summary = COALESCE(excluded.summary, works.summary),
               tags_json = excluded.tags_json,
               rating = excluded.rating,
               language = excluded.language,
               published_at = excluded.published_at,
               complete = excluded.complete,
               word_count = excluded.word_count,
               kudos = excluded.kudos,
               comment_count = excluded.comment_count,
               hits = excluded.hits,
               bookmarks = excluded.bookmarks,
               chapter_count = excluded.chapter_count,
               content_hash = excluded.content_hash,
               fetched_at = excluded.fetched_at",
        )
        .bind(&page.ao3_id)
        .bind(&page.title)
        .bind(&authors)
        .bind(page.summary.as_deref())
        .bind(&tags_json)
        .bind(page.rating.as_deref())
        .bind(page.language.as_deref())
        .bind(page.published_at.as_deref())
        .bind(page.complete.map(i64::from))
        .bind(page.word_count.map(|v| v as i64))
        .bind(page.kudos.map(|v| v as i64))
        .bind(page.comment_count.map(|v| v as i64))
        .bind(page.hits.map(|v| v as i64))
        .bind(page.bookmarks.map(|v| v as i64))
        .bind(page.chapters.len() as i64)
        .bind(content_hash)
        .bind(fetched_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("upsert work: {e}")))?;

        // Replace chapters: a re-fetch can change count and content.
        sqlx::query("DELETE FROM chapters WHERE work_id = ?1")
            .bind(&page.ao3_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Store(format!("clear chapters: {e}")))?;

        for chapter in &page.chapters {
            sqlx::query(
                "INSERT INTO chapters (work_id, idx, title, body_html, notes_html)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(&page.ao3_id)
            .bind(chapter.index as i64)
            .bind(chapter.title.as_deref())
            .bind(&chapter.body_html)
            .bind(chapter.notes_html.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Store(format!("insert chapter: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("commit upsert: {e}")))?;
        Ok(())
    }

    /// Cached work ids, for deciding whether an audit target can link locally.
    pub async fn cached_work_ids(&self) -> Result<std::collections::HashSet<String>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT ao3_id FROM works")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("cached work ids: {e}")))?;
        Ok(rows.into_iter().collect())
    }

    /// Number of cached works, for the chrome footer.
    pub async fn count_works(&self) -> Result<i64> {
        sqlx::query_scalar("SELECT COUNT(*) FROM works")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("count works: {e}")))
    }

    pub async fn get_work(&self, ao3_id: &str) -> Result<Option<StoredWork>> {
        let row = sqlx::query(
            "SELECT ao3_id, title, authors_json, summary, tags_json, rating, language,
                    published_at, complete, word_count, kudos, comment_count, hits,
                    bookmarks, chapter_count, content_hash, fetched_at
             FROM works WHERE ao3_id = ?1",
        )
        .bind(ao3_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select work: {e}")))?;
        row.as_ref().map(stored_work_from_row).transpose()
    }

    /// Every cached work, for the offline browse/facet pages.
    pub async fn all_works(&self) -> Result<Vec<StoredWork>> {
        let rows = sqlx::query(
            "SELECT ao3_id, title, authors_json, summary, tags_json, rating, language,
                    published_at, complete, word_count, kudos, comment_count, hits,
                    bookmarks, chapter_count, content_hash, fetched_at
             FROM works ORDER BY fetched_at DESC, ao3_id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select all works: {e}")))?;
        rows.iter().map(stored_work_from_row).collect()
    }

    pub async fn get_chapters(&self, ao3_id: &str) -> Result<Vec<StoredChapter>> {
        let rows = sqlx::query(
            "SELECT idx, title, body_html, notes_html FROM chapters
             WHERE work_id = ?1 ORDER BY idx ASC",
        )
        .bind(ao3_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select chapters: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| StoredChapter {
                idx: row.get("idx"),
                title: row.get("title"),
                body_html: row.get("body_html"),
                notes_html: row.get("notes_html"),
            })
            .collect())
    }

    pub async fn list_works(&self, limit: i64) -> Result<Vec<WorkSummary>> {
        let rows = sqlx::query(
            "SELECT w.ao3_id, w.title, w.authors_json, w.chapter_count,
                    p.chapter_idx AS progress_chapter, p.pct AS progress_pct
             FROM works w
             LEFT JOIN progress p ON p.work_id = w.ao3_id AND p.account_id = 'local'
             ORDER BY w.fetched_at DESC, w.ao3_id DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("list works: {e}")))?;

        let mut works = Vec::with_capacity(rows.len());
        for row in rows {
            let authors_json: String = row.get("authors_json");
            let authors: Vec<String> = serde_json::from_str(&authors_json).unwrap_or_default();
            works.push(WorkSummary {
                ao3_id: row.get("ao3_id"),
                title: row.get("title"),
                authors,
                chapter_count: row.get("chapter_count"),
                progress_chapter: row.get("progress_chapter"),
                progress_pct: row
                    .get::<Option<f64>, _>("progress_pct")
                    .map(|pct| pct.round() as i64),
            });
        }
        Ok(works)
    }

    /// Full-text search over cached works. Returns recent-first ties broken by
    /// rank; an empty query is an empty result, never "everything".
    pub async fn search_works(&self, query: &str, limit: i64) -> Result<Vec<WorkSummary>> {
        let Some(fts) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT w.ao3_id, w.title, w.authors_json, w.chapter_count,
                    p.chapter_idx AS progress_chapter, p.pct AS progress_pct
             FROM work_search
             JOIN works w ON w.ao3_id = work_search.work_id
             LEFT JOIN progress p ON p.work_id = w.ao3_id AND p.account_id = 'local'
             WHERE work_search MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )
        .bind(fts)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("search works: {e}")))?;

        let mut works = Vec::with_capacity(rows.len());
        for row in rows {
            let authors_json: String = row.get("authors_json");
            let authors: Vec<String> = serde_json::from_str(&authors_json).unwrap_or_default();
            works.push(WorkSummary {
                ao3_id: row.get("ao3_id"),
                title: row.get("title"),
                authors,
                chapter_count: row.get("chapter_count"),
                progress_chapter: row.get("progress_chapter"),
                progress_pct: row
                    .get::<Option<f64>, _>("progress_pct")
                    .map(|pct| pct.round() as i64),
            });
        }
        Ok(works)
    }

    /// The recent works list with full rows, for the shared blurb card.
    pub async fn list_works_detailed(&self, limit: i64) -> Result<Vec<StoredWork>> {
        let mut works = Vec::new();
        for summary in self.list_works(limit).await? {
            if let Some(work) = self.get_work(&summary.ao3_id).await? {
                works.push(work);
            }
        }
        Ok(works)
    }

    /// One page of plain FTS hits (no extra filters), kept for callers that only
    /// need ranked results.
    pub async fn search_works_page(
        &self,
        query: &str,
        page: i64,
        per_page: i64,
    ) -> Result<SearchPage> {
        self.search_works_filtered(query, &SearchFilter::default(), page, per_page)
            .await
    }

    /// AO3-style work search over the cache.
    ///
    /// A non-blank query starts from ranked FTS candidates; a blank query with
    /// at least one filter starts from every cached work. Filters that the cache
    /// does not model (hits/bookmarks/completion) are absent from
    /// [`SearchFilter`] rather than accepted and ignored.
    pub async fn search_works_filtered(
        &self,
        query: &str,
        filter: &SearchFilter,
        page: i64,
        per_page: i64,
    ) -> Result<SearchPage> {
        let per_page = per_page.clamp(1, 100);
        let page = page.max(1);
        let query = query.trim();
        let fts = fts_query(query);
        let has_query = fts.is_some();
        if !has_query && !filter.has_filters() {
            return Ok(SearchPage {
                works: Vec::new(),
                total: 0,
                page,
                per_page,
            });
        }

        let mut candidates: Vec<StoredWork> = if let Some(fts) = fts {
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT work_id FROM work_search WHERE work_search MATCH ?1
                 ORDER BY rank LIMIT 1000",
            )
            .bind(&fts)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("search candidates: {e}")))?;
            let mut works = Vec::new();
            for id in ids {
                if let Some(work) = self.get_work(&id).await? {
                    works.push(work);
                }
            }
            works
        } else {
            self.all_works().await?
        };

        candidates.retain(|work| search_matches(work, filter));

        let ascending = filter.direction.as_deref() == Some("asc");
        match filter.sort.as_deref().unwrap_or("best_match") {
            "words" => candidates.sort_by_key(|work| work.word_count.unwrap_or(0)),
            "kudos" => candidates.sort_by_key(|work| work.kudos.unwrap_or(0)),
            "title" => candidates.sort_by_cached_key(|work| work.title.to_ascii_lowercase()),
            "date" => candidates.sort_by_key(|work| work.fetched_at),
            // Best match keeps the FTS rank order; a filter-only search is
            // recent-first, which is the useful default for a local cache.
            _ => {
                if !has_query {
                    candidates.sort_by_key(|work| std::cmp::Reverse(work.fetched_at));
                }
            }
        }
        let explicit_sort = filter
            .sort
            .as_deref()
            .is_some_and(|sort| sort != "best_match");
        if explicit_sort && !ascending {
            candidates.reverse();
        }

        let total = candidates.len() as i64;
        let skip = usize::try_from((page - 1).saturating_mul(per_page)).unwrap_or(usize::MAX);
        let works = candidates
            .into_iter()
            .skip(skip)
            .take(per_page as usize)
            .collect();
        Ok(SearchPage {
            works,
            total,
            page,
            per_page,
        })
    }

    /// Search hits with the full cached row, so the renderer can use the same
    /// blurb card as browse. Reuses [`Self::search_works`] for ranking.
    pub async fn search_works_detailed(&self, query: &str, limit: i64) -> Result<Vec<StoredWork>> {
        let mut works = Vec::new();
        for summary in self.search_works(query, limit).await? {
            if let Some(work) = self.get_work(&summary.ao3_id).await? {
                works.push(work);
            }
        }
        Ok(works)
    }

    /// Records that a work was checked upstream without changing its content.
    pub async fn touch_checked(&self, ao3_id: &str) -> Result<()> {
        sqlx::query("UPDATE works SET fetched_at = ?2 WHERE ao3_id = ?1")
            .bind(ao3_id)
            .bind(now_secs())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("touch checked: {e}")))?;
        Ok(())
    }

    // ---- reading progress (chapter anchor) ----

    pub async fn save_progress(&self, work_id: &str, chapter_idx: i64, pct: i64) -> Result<()> {
        let pct = pct.clamp(0, 100);
        sqlx::query(
            "INSERT INTO progress (account_id, work_id, chapter_idx, pct, updated_at)
             VALUES ('local', ?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, work_id) DO UPDATE SET
               chapter_idx = excluded.chapter_idx,
               pct = excluded.pct,
               updated_at = excluded.updated_at",
        )
        .bind(work_id)
        .bind(chapter_idx)
        .bind(pct as f64)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("save progress: {e}")))?;
        Ok(())
    }

    pub async fn get_progress(&self, work_id: &str) -> Result<Option<Progress>> {
        let row = sqlx::query(
            "SELECT chapter_idx, pct FROM progress WHERE account_id = 'local' AND work_id = ?1",
        )
        .bind(work_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("get progress: {e}")))?;
        Ok(row.map(|row| Progress {
            chapter_idx: row.get("chapter_idx"),
            pct: row.get::<f64, _>("pct").round().clamp(0.0, 100.0) as i64,
        }))
    }
}

/// Quotes user input for FTS5: every whitespace token becomes a quoted prefix
/// term, so syntax characters are data, not query operators. `None` for blank.
pub(super) fn fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

/// Stable content hash for change detection (`openssl` is already a dependency
/// for the secret store, so no extra crate).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = openssl::sha::sha256(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// One work against the search filters (case-insensitive substring matching,
/// which is how AO3 treats free-text tag/creator fields in practice).
fn search_matches(work: &StoredWork, filter: &SearchFilter) -> bool {
    let contains = |value: &str, needle: &Option<String>| {
        needle.as_deref().is_none_or(|needle| {
            value
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        })
    };
    let any_tag = |tags: &[String], needle: &Option<String>| {
        needle.as_deref().is_none_or(|needle| {
            tags.iter().any(|tag| {
                tag.to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            })
        })
    };
    let exact = |value: Option<&str>, needle: &Option<String>| {
        needle
            .as_deref()
            .is_none_or(|needle| value.is_some_and(|value| value.eq_ignore_ascii_case(needle)))
    };

    contains(&work.title, &filter.title)
        && (filter.creator.is_none()
            || work.authors.iter().any(|author| {
                filter.creator.as_deref().is_some_and(|needle| {
                    author
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
                })
            }))
        && any_tag(&work.tags.fandoms, &filter.fandom)
        && any_tag(&work.tags.characters, &filter.characters)
        && any_tag(&work.tags.relationships, &filter.relationships)
        && any_tag(&work.tags.additional, &filter.additional_tags)
        && any_tag(&work.tags.warnings, &filter.warning)
        && any_tag(&work.tags.categories, &filter.category)
        && exact(work.rating.as_deref(), &filter.rating)
        && exact(work.language.as_deref(), &filter.language)
        && filter
            .words_min
            .is_none_or(|min| work.word_count.unwrap_or(0) >= min)
        && filter
            .words_max
            .is_none_or(|max| work.word_count.unwrap_or(0) <= max)
        && filter
            .kudos_min
            .is_none_or(|min| work.kudos.unwrap_or(0) >= min)
        && filter
            .comments_min
            .is_none_or(|min| work.comment_count.unwrap_or(0) >= min)
        && (!filter.single_chapter || work.chapter_count == 1)
}

/// One `works` row (the column list is repeated by `get_work`/`all_works`).
fn stored_work_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<StoredWork> {
    let authors_json: String = row.get("authors_json");
    let tags_json: String = row.get("tags_json");
    let authors: Vec<String> = serde_json::from_str(&authors_json)
        .map_err(|e| Error::Store(format!("corrupt authors json: {e}")))?;
    let tags: TagGroups = serde_json::from_str(&tags_json)
        .map_err(|e| Error::Store(format!("corrupt tags json: {e}")))?;
    Ok(StoredWork {
        ao3_id: row.get("ao3_id"),
        title: row.get("title"),
        authors,
        summary: row.get("summary"),
        tags,
        rating: row.get("rating"),
        language: row.get("language"),
        published_at: row.get("published_at"),
        complete: row.get::<Option<i64>, _>("complete").map(|v| v != 0),
        word_count: row.get::<Option<i64>, _>("word_count").map(|v| v as u64),
        kudos: row.get::<Option<i64>, _>("kudos").map(|v| v as u64),
        comment_count: row.get::<Option<i64>, _>("comment_count").map(|v| v as u64),
        hits: row.get::<Option<i64>, _>("hits").map(|v| v as u64),
        bookmarks: row.get::<Option<i64>, _>("bookmarks").map(|v| v as u64),
        chapter_count: row.get::<i64, _>("chapter_count") as usize,
        content_hash: row.get("content_hash"),
        fetched_at: row.get("fetched_at"),
    })
}
