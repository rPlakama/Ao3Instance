//! SQLite storage: pool setup, pragmas and migrations.
//!
//! One file, one migrator (`migrations/`), WAL mode. Durability tiering and the
//! blob store arrive with the write pipeline; this is the M1 read cache.

use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

use crate::error::{Error, Result};

/// Opens (creating if needed) the database and runs migrations.
pub async fn open(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Store(format!("create db dir {}: {e}", parent.display())))?;
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
        .synchronous(SqliteSynchronous::Normal)
        // Keep the cache small: this daemon also renders pages from the same box.
        .pragma("cache_size", "-2048")
        .pragma("mmap_size", "268435456")
        .pragma("temp_store", "MEMORY");

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .map_err(|e| Error::Store(format!("open db {}: {e}", path.display())))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| Error::Store(format!("migrate db: {e}")))?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_and_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("ao3i-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        let pool = open(&path).await.unwrap();
        pool.close().await;
        // Re-opening runs migrations again; they must be idempotent.
        let pool = open(&path).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        pool.close().await;
        drop(std::fs::remove_dir_all(&dir));
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let dir = std::env::temp_dir().join(format!("ao3i-db-fk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = open(&dir.join("test.db")).await.unwrap();
        let result = sqlx::query(
            "INSERT INTO chapters (work_id, idx, body_html) VALUES ('missing', 1, 'x')",
        )
        .execute(&pool)
        .await;
        assert!(result.is_err(), "FK must be enforced");
        pool.close().await;
        drop(std::fs::remove_dir_all(&dir));
    }
}
