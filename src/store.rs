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
                    word_count, kudos, comment_count, chapter_count, fetched_at
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

    /// Most recently saved session: `(account, encrypted jar)`.
    pub async fn load_session(&self) -> Result<Option<(String, String)>> {
        let row = sqlx::query(
            "SELECT account_id, jar_enc FROM sessions ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("load session: {e}")))?;
        Ok(row.map(|row| (row.get("account_id"), row.get("jar_enc"))))
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
        let (account, jar) = store.load_session().await.unwrap().expect("session");
        assert_eq!(account, "someone");
        assert_eq!(jar, "ciphertext-2");
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

    #[test]
    fn hashes_are_stable_and_distinct() {
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }
}
