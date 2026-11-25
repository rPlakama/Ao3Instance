//! Cached AO3 comment threads (`migrations/0014_work_comments.sql`).
//!
//! AO3 comments hang off a *commentable*: the work itself (Entire Work view, or
//! a one-chapter work) or one chapter of a multi-chapter work. The cache is
//! keyed by that scope, so the reader shows the same thread AO3 shows for the
//! page it is reading. A refresh replaces the scope's rows atomically; the
//! caller must never call this after a failed fetch (the old copy stays).

use sqlx::Row;

use crate::ao3::comments::Comment;
use crate::error::{Error, Result};
use crate::time::now_secs;

use super::Store;

/// Scope for comments attached to the work itself.
pub const WORK_SCOPE: &str = "work";

/// Scope for comments attached to one AO3 chapter.
pub fn chapter_scope(ao3_chapter_id: &str) -> String {
    format!("chapter:{ao3_chapter_id}")
}

/// A cached comment plus the commentable scope it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentWithScope {
    pub scope: String,
    pub comment: Comment,
}

/// Metadata for one cached commentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentSetInfo {
    /// AO3's visible-comment total for that commentable, when the page said.
    pub total: Option<u64>,
    pub fetched_at: i64,
    /// The thread was paginated upstream: only the first page is cached.
    pub has_more: bool,
}

impl Store {
    /// Replaces every cached comment for `work_id`/`scope` with `comments`.
    ///
    /// The delete and the inserts commit together, so a crash can never leave a
    /// half-replaced thread.
    pub async fn replace_comments(
        &self,
        work_id: &str,
        scope: &str,
        comments: &[Comment],
        total: Option<u64>,
        has_more: bool,
    ) -> Result<()> {
        let fetched_at = now_secs();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Store(format!("begin comments tx: {e}")))?;

        sqlx::query("DELETE FROM work_comments WHERE work_id = ?1 AND scope = ?2")
            .bind(work_id)
            .bind(scope)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Store(format!("clear comments: {e}")))?;

        for comment in comments {
            sqlx::query(
                "INSERT INTO work_comments
                   (work_id, scope, seq, comment_id, parent_id, depth, author,
                    author_login, posted_at, edited_at, body_html)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .bind(work_id)
            .bind(scope)
            .bind(comment.seq)
            .bind(&comment.comment_id)
            .bind(comment.parent_id.as_deref())
            .bind(comment.depth)
            .bind(comment.author.as_deref())
            .bind(comment.author_login.as_deref())
            .bind(comment.posted_at.as_deref())
            .bind(comment.edited_at.as_deref())
            .bind(&comment.body_html)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Store(format!("insert comment: {e}")))?;
        }

        sqlx::query(
            "INSERT INTO work_comment_sets (work_id, scope, total, fetched_at, has_more)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(work_id, scope) DO UPDATE SET
               total = excluded.total,
               fetched_at = excluded.fetched_at,
               has_more = excluded.has_more",
        )
        .bind(work_id)
        .bind(scope)
        .bind(
            total
                .map(i64::try_from)
                .transpose()
                .map_err(|_| Error::Store("comment total does not fit in SQLite INTEGER".into()))?,
        )
        .bind(fetched_at)
        .bind(i64::from(has_more))
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("upsert comment set: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("commit comments: {e}")))?;
        Ok(())
    }

    /// Cached comments for one commentable, in AO3's document order.
    /// Finds one cached comment (with its scope) for reply/edit/delete actions.
    pub async fn find_comment(
        &self,
        work_id: &str,
        comment_id: &str,
    ) -> Result<Option<CommentWithScope>> {
        let row = sqlx::query(
            "SELECT scope, seq, comment_id, parent_id, depth, author, author_login,
                    posted_at, edited_at, body_html
             FROM work_comments
             WHERE work_id = ?1 AND comment_id = ?2
             LIMIT 1",
        )
        .bind(work_id)
        .bind(comment_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("find comment: {e}")))?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(CommentWithScope {
            scope: row.try_get("scope").map_err(store_err)?,
            comment: Comment {
                seq: row.try_get("seq").map_err(store_err)?,
                comment_id: row.try_get("comment_id").map_err(store_err)?,
                parent_id: row.try_get("parent_id").map_err(store_err)?,
                depth: row.try_get("depth").map_err(store_err)?,
                author: row.try_get("author").map_err(store_err)?,
                author_login: row.try_get("author_login").map_err(store_err)?,
                posted_at: row.try_get("posted_at").map_err(store_err)?,
                edited_at: row.try_get("edited_at").map_err(store_err)?,
                body_html: row.try_get("body_html").map_err(store_err)?,
            },
        }))
    }

    pub async fn get_comments(&self, work_id: &str, scope: &str) -> Result<Vec<Comment>> {
        let rows = sqlx::query(
            "SELECT seq, comment_id, parent_id, depth, author, author_login,
                    posted_at, edited_at, body_html
             FROM work_comments
             WHERE work_id = ?1 AND scope = ?2
             ORDER BY seq ASC",
        )
        .bind(work_id)
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select comments: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| Comment {
                seq: row.get("seq"),
                comment_id: row.get("comment_id"),
                parent_id: row.get("parent_id"),
                depth: row.get("depth"),
                author: row.get("author"),
                author_login: row.get("author_login"),
                posted_at: row.get("posted_at"),
                edited_at: row.get("edited_at"),
                body_html: row.get("body_html"),
            })
            .collect())
    }

    /// Whether this commentable was ever fetched, and when.
    pub async fn get_comment_set(
        &self,
        work_id: &str,
        scope: &str,
    ) -> Result<Option<CommentSetInfo>> {
        let row = sqlx::query(
            "SELECT total, fetched_at, has_more FROM work_comment_sets
             WHERE work_id = ?1 AND scope = ?2",
        )
        .bind(work_id)
        .bind(scope)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select comment set: {e}")))?;

        Ok(row.map(|row| CommentSetInfo {
            total: row
                .get::<Option<i64>, _>("total")
                .and_then(|value| u64::try_from(value).ok()),
            fetched_at: row.get("fetched_at"),
            has_more: row.get::<i64, _>("has_more") != 0,
        }))
    }
}

fn store_err(error: sqlx::Error) -> Error {
    Error::Store(format!("comment row: {error}"))
}
