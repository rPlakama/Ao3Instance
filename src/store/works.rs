//! Cached work rows, full-text search and the local reading position.

use super::{Store, StoredChapter, StoredWork, TagGroups, WorkSummary, now_secs};
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
                                word_count, kudos, comment_count, chapter_count, content_hash, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(ao3_id) DO UPDATE SET
               title = excluded.title,
               authors_json = excluded.authors_json,
               -- A partial page (chapter 2+) has no summary block; keep the cached one.
               summary = COALESCE(excluded.summary, works.summary),
               tags_json = excluded.tags_json,
               rating = excluded.rating,
               language = excluded.language,
               word_count = excluded.word_count,
               kudos = excluded.kudos,
               comment_count = excluded.comment_count,
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
        .bind(page.word_count.map(|v| v as i64))
        .bind(page.kudos.map(|v| v as i64))
        .bind(page.comment_count.map(|v| v as i64))
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
                    word_count, kudos, comment_count, chapter_count, content_hash, fetched_at
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
                    word_count, kudos, comment_count, chapter_count, content_hash, fetched_at
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
            "SELECT w.ao3_id, w.title, w.authors_json, w.chapter_count, p.chapter_idx AS progress_chapter
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
                    p.chapter_idx AS progress_chapter
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
            });
        }
        Ok(works)
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

    pub async fn save_progress(&self, work_id: &str, chapter_idx: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO progress (account_id, work_id, chapter_idx, updated_at)
             VALUES ('local', ?1, ?2, ?3)
             ON CONFLICT(account_id, work_id) DO UPDATE SET
               chapter_idx = excluded.chapter_idx,
               updated_at = excluded.updated_at",
        )
        .bind(work_id)
        .bind(chapter_idx)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("save progress: {e}")))?;
        Ok(())
    }

    pub async fn get_progress(&self, work_id: &str) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT chapter_idx FROM progress WHERE account_id = 'local' AND work_id = ?1",
        )
        .bind(work_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("get progress: {e}")))?;
        Ok(row.map(|row| row.get("chapter_idx")))
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
        word_count: row.get::<Option<i64>, _>("word_count").map(|v| v as u64),
        kudos: row.get::<Option<i64>, _>("kudos").map(|v| v as u64),
        comment_count: row.get::<Option<i64>, _>("comment_count").map(|v| v as u64),
        chapter_count: row.get::<i64, _>("chapter_count") as usize,
        content_hash: row.get("content_hash"),
        fetched_at: row.get("fetched_at"),
    })
}
