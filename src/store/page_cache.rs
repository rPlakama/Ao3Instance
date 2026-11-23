//! Persistent SQLite cache for live AO3 pages (search results, tag pages,
//! subscriptions, bookmarks, reading history, fandoms, home, public skins).

use sqlx::Row;

use super::{Store, now_secs};
use crate::consts;
use crate::error::{Error, Result};

/// One cached AO3 HTML response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPage {
    pub body: String,
    pub fetched_at: i64,
}

impl CachedPage {
    /// True when the entry is within the 30-minute freshness window.
    pub fn is_fresh(&self, now: i64) -> bool {
        now.saturating_sub(self.fetched_at) < consts::PAGE_CACHE_TTL_SECS
    }
}

/// Builds a stable cache key scoping `path` to `account` (empty for signed-out).
pub fn page_cache_key(account: &str, path: &str) -> String {
    format!("{account}|{path}")
}

impl Store {
    pub async fn get_cached_page(&self, account: &str, path: &str) -> Result<Option<CachedPage>> {
        let key = page_cache_key(account, path);
        let row = sqlx::query("SELECT body, fetched_at FROM page_cache WHERE cache_key = ?1")
            .bind(&key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("select page_cache: {e}")))?;
        Ok(row.map(|row| CachedPage {
            body: row.get("body"),
            fetched_at: row.get("fetched_at"),
        }))
    }

    pub async fn put_cached_page(&self, account: &str, path: &str, body: &str) -> Result<()> {
        let key = page_cache_key(account, path);
        let now = now_secs();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Store(format!("begin page_cache tx: {e}")))?;
        sqlx::query(
            "INSERT INTO page_cache (cache_key, account, body, fetched_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(cache_key) DO UPDATE SET
               account = excluded.account,
               body = excluded.body,
               fetched_at = excluded.fetched_at",
        )
        .bind(&key)
        .bind(account)
        .bind(body)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("upsert page_cache: {e}")))?;
        sqlx::query(
            "DELETE FROM page_cache WHERE cache_key NOT IN
             (SELECT cache_key FROM page_cache ORDER BY fetched_at DESC LIMIT ?1)",
        )
        .bind(consts::PAGE_CACHE_MAX_ROWS)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("prune page_cache: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("commit page_cache tx: {e}")))?;
        Ok(())
    }

    /// Drops cached pages whose path begins with `path_prefix` for `account`.
    pub async fn invalidate_cached_prefix(&self, account: &str, path_prefix: &str) -> Result<()> {
        let prefix = page_cache_key(account, path_prefix);
        sqlx::query(
            "DELETE FROM page_cache
             WHERE account = ?1 AND substr(cache_key, 1, length(?2)) = ?2",
        )
        .bind(account)
        .bind(&prefix)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("invalidate page_cache prefix: {e}")))?;
        Ok(())
    }

    /// Drops every cached page belonging to `account` (used on login/logout).
    pub async fn invalidate_account_pages(&self, account: &str) -> Result<()> {
        sqlx::query("DELETE FROM page_cache WHERE account = ?1")
            .bind(account)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("invalidate account page_cache: {e}")))?;
        Ok(())
    }
}
