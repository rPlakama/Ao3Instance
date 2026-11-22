//! Work cache: store a parsed work, read it back, list what is cached.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::ao3::work::WorkPage;
use crate::error::{Error, Result};

/// AO3's tag groups, stored together as one JSON column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagGroups {
    #[serde(default)]
    pub fandoms: Vec<String>,
    #[serde(default)]
    pub relationships: Vec<String>,
    #[serde(default)]
    pub characters: Vec<String>,
    #[serde(default)]
    pub additional: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWork {
    pub ao3_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub summary: Option<String>,
    pub tags: TagGroups,
    pub rating: Option<String>,
    pub language: Option<String>,
    pub word_count: Option<u64>,
    pub kudos: Option<u64>,
    pub comment_count: Option<u64>,
    pub chapter_count: usize,
    pub content_hash: String,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChapter {
    pub idx: i64,
    pub title: Option<String>,
    pub body_html: String,
    pub notes_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub target: String,
    pub outcome: String,
    pub evidence: Option<String>,
    pub created_at: i64,
}

impl AuditEntry {
    /// Convenience for tests and templates: never `None`.
    pub fn evidence_or_empty(&self) -> &str {
        self.evidence.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSession {
    pub account: String,
    pub jar_enc: String,
    pub pseud_id: Option<i64>,
    pub pseud_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionRow {
    pub target_kind: String,
    pub target_id: String,
    pub subscription_id: Option<i64>,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkSummary {
    pub ao3_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub chapter_count: i64,
    /// Chapter to resume at, when a reading position is stored.
    pub progress_chapter: Option<i64>,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

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

        let Some(row) = row else {
            return Ok(None);
        };
        let authors_json: String = row.get("authors_json");
        let tags_json: String = row.get("tags_json");
        let authors: Vec<String> = serde_json::from_str(&authors_json)
            .map_err(|e| Error::Store(format!("corrupt authors json: {e}")))?;
        let tags: TagGroups = serde_json::from_str(&tags_json)
            .map_err(|e| Error::Store(format!("corrupt tags json: {e}")))?;
        Ok(Some(StoredWork {
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
        }))
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
}

impl Store {
    // ---- write ledger (intent before the request, audit after) ----

    pub async fn record_intent(
        &self,
        account: &str,
        action: &str,
        target_kind: &str,
        target_id: &str,
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO write_intent (account, action, target_kind, target_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(account)
        .bind(action)
        .bind(target_kind)
        .bind(target_id)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("record intent: {e}")))?;
        Ok(result.last_insert_rowid())
    }

    pub async fn finish_intent(
        &self,
        intent_id: i64,
        state: &str,
        outcome: &str,
        evidence: Option<&str>,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Store(format!("begin tx: {e}")))?;
        sqlx::query("UPDATE write_intent SET state = ?2 WHERE id = ?1")
            .bind(intent_id)
            .bind(state)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Store(format!("update intent: {e}")))?;
        sqlx::query(
            "INSERT INTO write_audit (intent_id, outcome, evidence, created_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(intent_id)
        .bind(outcome)
        .bind(evidence)
        .bind(now_secs())
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("insert audit: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("commit audit: {e}")))?;
        Ok(())
    }

    pub async fn recent_audit(&self, limit: i64) -> Result<Vec<AuditEntry>> {
        let rows = sqlx::query(
            "SELECT a.id,
                    COALESCE(i.action, '?') AS action,
                    COALESCE(i.target_kind || ' ' || i.target_id, '?') AS target,
                    a.outcome,
                    a.evidence,
                    a.created_at
             FROM write_audit a
             LEFT JOIN write_intent i ON i.id = a.intent_id
             ORDER BY a.id DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("recent audit: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| AuditEntry {
                id: row.get("id"),
                action: row.get("action"),
                target: row.get("target"),
                outcome: row.get("outcome"),
                evidence: row.get("evidence"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // ---- subscription ledger (our own duplicate pre-check) ----

    pub async fn get_subscription(
        &self,
        account: &str,
        target_kind: &str,
        target_id: &str,
    ) -> Result<Option<SubscriptionRow>> {
        let row = sqlx::query(
            "SELECT target_kind, target_id, subscription_id, state FROM subscriptions
             WHERE account = ?1 AND target_kind = ?2 AND target_id = ?3",
        )
        .bind(account)
        .bind(target_kind)
        .bind(target_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("get subscription: {e}")))?;
        Ok(row.map(|row| SubscriptionRow {
            target_kind: row.get("target_kind"),
            target_id: row.get("target_id"),
            subscription_id: row.get("subscription_id"),
            state: row.get("state"),
        }))
    }

    pub async fn upsert_subscription(
        &self,
        account: &str,
        target_kind: &str,
        target_id: &str,
        subscription_id: Option<i64>,
        state: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO subscriptions (account, target_kind, target_id, subscription_id, state, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account, target_kind, target_id) DO UPDATE SET
               subscription_id = excluded.subscription_id,
               state = excluded.state,
               updated_at = excluded.updated_at",
        )
        .bind(account)
        .bind(target_kind)
        .bind(target_id)
        .bind(subscription_id)
        .bind(state)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("upsert subscription: {e}")))?;
        Ok(())
    }

    // ---- kudos receipts and reading flags ----

    pub async fn mark_kudos(&self, account: &str, work_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO kudos_sent (account, work_id, created_at) VALUES (?1, ?2, ?3)",
        )
        .bind(account)
        .bind(work_id)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("mark kudos: {e}")))?;
        Ok(())
    }

    pub async fn has_kudos(&self, account: &str, work_id: &str) -> Result<bool> {
        let row = sqlx::query("SELECT 1 FROM kudos_sent WHERE account = ?1 AND work_id = ?2")
            .bind(account)
            .bind(work_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("has kudos: {e}")))?;
        Ok(row.is_some())
    }

    pub async fn set_reading_state(&self, account: &str, work_id: &str, state: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO reading_flags (account, work_id, state, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account, work_id) DO UPDATE SET
               state = excluded.state,
               updated_at = excluded.updated_at",
        )
        .bind(account)
        .bind(work_id)
        .bind(state)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("set reading state: {e}")))?;
        Ok(())
    }

    pub async fn get_reading_state(&self, account: &str, work_id: &str) -> Result<Option<String>> {
        let row =
            sqlx::query("SELECT state FROM reading_flags WHERE account = ?1 AND work_id = ?2")
                .bind(account)
                .bind(work_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| Error::Store(format!("get reading state: {e}")))?;
        Ok(row.map(|row| row.get("state")))
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

    // ---- persisted AO3 session (encrypted jar) ----

    pub async fn save_session(&self, account: &str, jar_enc: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions (account_id, jar_enc, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id) DO UPDATE SET
               jar_enc = excluded.jar_enc,
               updated_at = excluded.updated_at",
        )
        .bind(account)
        .bind(jar_enc)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("save session: {e}")))?;
        Ok(())
    }

    /// Most recently saved session (jar is still encrypted).
    pub async fn load_session(&self) -> Result<Option<SavedSession>> {
        let row = sqlx::query(
            "SELECT account_id, jar_enc, pseud_id, pseud_name FROM sessions
             ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("load session: {e}")))?;
        Ok(row.map(|row| SavedSession {
            account: row.get("account_id"),
            jar_enc: row.get("jar_enc"),
            pseud_id: row.get("pseud_id"),
            pseud_name: row.get("pseud_name"),
        }))
    }

    /// Remembers the pseud used for comments on the persisted session.
    pub async fn set_session_pseud(
        &self,
        account: &str,
        pseud_id: i64,
        pseud_name: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET pseud_id = ?2, pseud_name = ?3
             WHERE account_id = ?1",
        )
        .bind(account)
        .bind(pseud_id)
        .bind(pseud_name)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("set session pseud: {e}")))?;
        Ok(())
    }

    pub async fn delete_session(&self) -> Result<()> {
        sqlx::query("DELETE FROM sessions")
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("delete session: {e}")))?;
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
fn fts_query(raw: &str) -> Option<String> {
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

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ao3::work::Chapter;
    use crate::db;

    fn page() -> WorkPage {
        WorkPage {
            ao3_id: "123".into(),
            title: "A Test Work".into(),
            authors: vec!["tester".into()],
            summary: Some("summary".into()),
            fandoms: vec!["Fandom".into()],
            relationships: vec![],
            characters: vec![],
            additional_tags: vec!["Fluff".into()],
            warnings: vec!["No Archive Warnings Apply".into()],
            categories: vec!["F/M".into()],
            rating: Some("General Audiences".into()),
            language: Some("English".into()),
            word_count: Some(1234),
            kudos: Some(7),
            comment_count: Some(2),
            chapter_count: 2,
            chapters: vec![
                Chapter {
                    index: 1,
                    title: Some("One".into()),
                    body_html: "<p>one</p>".into(),
                    notes_html: None,
                },
                Chapter {
                    index: 2,
                    title: Some("Two".into()),
                    body_html: "<p>two</p>".into(),
                    notes_html: Some("<p>note</p>".into()),
                },
            ],
            comment_form_present: true,
        }
    }

    async fn test_store(tag: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("ao3i-store-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = db::open(&dir.join("test.db")).await.unwrap();
        (Store::new(pool), dir)
    }

    #[tokio::test]
    async fn upsert_round_trip_and_replace() {
        let (store, dir) = test_store("roundtrip").await;
        store.upsert_work(&page(), "hash1").await.unwrap();

        let work = store.get_work("123").await.unwrap().expect("stored");
        assert_eq!(work.title, "A Test Work");
        assert_eq!(work.authors, vec!["tester".to_string()]);
        assert_eq!(work.tags.fandoms, vec!["Fandom".to_string()]);
        assert_eq!(work.tags.additional, vec!["Fluff".to_string()]);
        assert_eq!(work.chapter_count, 2);

        let chapters = store.get_chapters("123").await.unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[1].body_html, "<p>two</p>");
        assert_eq!(chapters[1].notes_html.as_deref(), Some("<p>note</p>"));

        // Re-fetch with fewer chapters must replace, not append.
        let mut second = page();
        second.chapters.truncate(1);
        second.title = "Renamed".into();
        store.upsert_work(&second, "hash2").await.unwrap();
        let work = store.get_work("123").await.unwrap().expect("stored");
        assert_eq!(work.title, "Renamed");
        assert_eq!(store.get_chapters("123").await.unwrap().len(), 1);

        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[tokio::test]
    async fn list_returns_recent_first() {
        let (store, dir) = test_store("list").await;
        let mut a = page();
        a.ao3_id = "1".into();
        let mut b = page();
        b.ao3_id = "2".into();
        b.title = "Second".into();
        store.upsert_work(&a, "h1").await.unwrap();
        store.upsert_work(&b, "h2").await.unwrap();

        let works = store.list_works(10).await.unwrap();
        assert_eq!(works.len(), 2);
        assert_eq!(works[0].ao3_id, "2");

        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[tokio::test]
    async fn session_and_progress_round_trip() {
        let (store, dir) = test_store("session").await;
        assert!(store.load_session().await.unwrap().is_none());

        store.save_session("someone", "ciphertext-1").await.unwrap();
        store.save_session("someone", "ciphertext-2").await.unwrap();
        let saved = store.load_session().await.unwrap().expect("session");
        assert_eq!(saved.account, "someone");
        assert_eq!(saved.jar_enc, "ciphertext-2");
        assert_eq!(saved.pseud_id, None);
        store
            .set_session_pseud("someone", 111, "Main Pseud")
            .await
            .unwrap();
        let saved = store.load_session().await.unwrap().expect("session");
        assert_eq!(saved.pseud_id, Some(111));
        assert_eq!(saved.pseud_name.as_deref(), Some("Main Pseud"));
        store.delete_session().await.unwrap();
        assert!(store.load_session().await.unwrap().is_none());

        store.upsert_work(&page(), "h").await.unwrap();
        assert!(store.get_progress("123").await.unwrap().is_none());
        store.save_progress("123", 2).await.unwrap();
        assert_eq!(store.get_progress("123").await.unwrap(), Some(2));
        let works = store.list_works(10).await.unwrap();
        assert_eq!(works[0].progress_chapter, Some(2));

        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[tokio::test]
    async fn search_finds_title_author_and_tags() {
        let (store, dir) = test_store("search").await;
        store.upsert_work(&page(), "h1").await.unwrap();
        let mut other = page();
        other.ao3_id = "999".into();
        other.title = "Another Story".into();
        other.authors = vec!["secondwriter".into()];
        other.additional_tags = vec!["Angst".into()];
        store.upsert_work(&other, "h2").await.unwrap();

        let by_title = store.search_works("Test", 10).await.unwrap();
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].ao3_id, "123");

        let by_author = store.search_works("secondwriter", 10).await.unwrap();
        assert_eq!(by_author[0].ao3_id, "999");

        let by_tag = store.search_works("angst", 10).await.unwrap();
        assert_eq!(by_tag[0].ao3_id, "999");

        // No results, blank query, and FTS syntax characters as data.
        assert!(
            store
                .search_works("nothingmatchesthis", 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(store.search_works("   ", 10).await.unwrap().is_empty());
        assert!(
            store
                .search_works("\"; DROP TABLE works; --", 10)
                .await
                .unwrap()
                .is_empty()
        );

        // Update triggers keep the index in step.
        let mut renamed = page();
        renamed.title = "Renamed Work".into();
        renamed.authors = vec!["writer2".into()];
        store.upsert_work(&renamed, "h3").await.unwrap();
        assert_eq!(store.search_works("Renamed", 10).await.unwrap().len(), 1);
        // Both the old title and the old author are gone from the index.
        assert!(store.search_works("Test", 10).await.unwrap().is_empty());
        assert!(store.search_works("tester", 10).await.unwrap().is_empty());

        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[tokio::test]
    async fn touch_checked_moves_fetched_at_forward() {
        let (store, dir) = test_store("touch").await;
        store.upsert_work(&page(), "h1").await.unwrap();
        let before = store
            .get_work("123")
            .await
            .unwrap()
            .expect("work")
            .fetched_at;
        sqlx::query("UPDATE works SET fetched_at = 1")
            .execute(store.pool())
            .await
            .unwrap();
        store.touch_checked("123").await.unwrap();
        let after = store
            .get_work("123")
            .await
            .unwrap()
            .expect("work")
            .fetched_at;
        assert!(after > 1 && after >= before);
        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[test]
    fn fts_query_quotes_tokens() {
        assert_eq!(
            fts_query("hello world").as_deref(),
            Some("\"hello\"* \"world\"*")
        );
        assert_eq!(fts_query("a\"b").as_deref(), Some("\"a\"\"b\"*"));
        assert_eq!(fts_query("  "), None);
    }

    #[tokio::test]
    async fn write_ledger_and_subscription_round_trip() {
        let (store, dir) = test_store("writes").await;
        let intent = store
            .record_intent("someone", "subscribe", "Work", "123")
            .await
            .unwrap();
        store
            .finish_intent(intent, "done", "subscribed", Some("item 42"))
            .await
            .unwrap();
        let audit = store.recent_audit(10).await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].action, "subscribe");
        assert_eq!(audit[0].target, "Work 123");
        assert_eq!(audit[0].outcome, "subscribed");

        assert!(
            store
                .get_subscription("someone", "Work", "123")
                .await
                .unwrap()
                .is_none()
        );
        store
            .upsert_subscription("someone", "Work", "123", Some(42), "active")
            .await
            .unwrap();
        let row = store
            .get_subscription("someone", "Work", "123")
            .await
            .unwrap()
            .expect("ledger row");
        assert_eq!(row.subscription_id, Some(42));
        assert_eq!(row.state, "active");

        store
            .upsert_subscription("someone", "Work", "123", None, "removed")
            .await
            .unwrap();
        let row = store
            .get_subscription("someone", "Work", "123")
            .await
            .unwrap()
            .expect("ledger row");
        assert_eq!(row.state, "removed");

        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[tokio::test]
    async fn kudos_and_reading_state_round_trip() {
        let (store, dir) = test_store("kudos").await;
        assert!(!store.has_kudos("someone", "123").await.unwrap());
        store.mark_kudos("someone", "123").await.unwrap();
        store.mark_kudos("someone", "123").await.unwrap(); // idempotent
        assert!(store.has_kudos("someone", "123").await.unwrap());
        assert!(!store.has_kudos("other", "123").await.unwrap());

        assert!(
            store
                .get_reading_state("someone", "123")
                .await
                .unwrap()
                .is_none()
        );
        store
            .set_reading_state("someone", "123", "for_later")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_reading_state("someone", "123")
                .await
                .unwrap()
                .as_deref(),
            Some("for_later")
        );
        store
            .set_reading_state("someone", "123", "read")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_reading_state("someone", "123")
                .await
                .unwrap()
                .as_deref(),
            Some("read")
        );
        store.pool().close().await;
        drop(std::fs::remove_dir_all(dir));
    }

    #[test]
    fn hashes_are_stable_and_distinct() {
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }
}
