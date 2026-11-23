//! Local access history: the only deliberate difference from AO3's data model.

use sqlx::Row;

use super::Store;
use crate::error::{Error, Result};

/// One work access, joined with the cached title when available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessEntry {
    pub work_id: String,
    pub title: Option<String>,
    pub kind: String,
    pub accessed_at: i64,
}

impl Store {
    pub async fn record_access(&self, work_id: &str, kind: &str) -> Result<()> {
        sqlx::query("INSERT INTO access_log (work_id, kind, accessed_at) VALUES (?1, ?2, ?3)")
            .bind(work_id)
            .bind(kind)
            .bind(crate::time::now_secs())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("record access: {e}")))?;
        Ok(())
    }

    pub async fn recent_accesses(&self, limit: i64, since: i64) -> Result<Vec<AccessEntry>> {
        let rows = sqlx::query(
            "SELECT a.work_id, a.kind, a.accessed_at, w.title
             FROM access_log a
             LEFT JOIN works w ON w.ao3_id = a.work_id
             WHERE a.accessed_at >= ?1
             ORDER BY a.id DESC LIMIT ?2",
        )
        .bind(since)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("recent accesses: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| AccessEntry {
                work_id: row.get("work_id"),
                title: row.get("title"),
                kind: row.get("kind"),
                accessed_at: row.get("accessed_at"),
            })
            .collect())
    }
}
