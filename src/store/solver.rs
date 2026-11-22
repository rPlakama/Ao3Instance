//! Solver outcomes and the encrypted Cloudflare clearance.
//!
//! `solver_event` is an audit trail (no HTML, no cookies); `solver_clearance`
//! holds one AES-256-GCM ciphertext at most, bound to its purpose by the
//! `ao3/solver` AAD in `crypto`.

use sqlx::Row;

use crate::error::{Error, Result};

use super::Store;

/// One solver outcome. Safe to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverEventRow {
    pub id: i64,
    pub outcome: String,
    pub target_path: String,
    pub status: Option<u16>,
    pub latency_ms: Option<u64>,
    pub created_at: i64,
}

/// The persisted clearance, still encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClearance {
    pub ciphertext: String,
    pub user_agent: String,
    pub expires_at: i64,
}

impl Store {
    /// Inserts one outcome and prunes beyond `retention` rows.
    pub async fn record_solver_event(
        &self,
        outcome: &str,
        target_path: &str,
        status: Option<u16>,
        latency_ms: Option<u64>,
        retention: i64,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Store(format!("begin solver event tx: {e}")))?;
        sqlx::query(
            "INSERT INTO solver_event (outcome, target_path, status, latency_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(outcome)
        .bind(target_path)
        .bind(status.map(i64::from))
        .bind(latency_ms.map(opt_i64))
        .bind(crate::time::now_secs())
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("insert solver event: {e}")))?;
        sqlx::query(
            "DELETE FROM solver_event WHERE id NOT IN
             (SELECT id FROM solver_event ORDER BY id DESC LIMIT ?1)",
        )
        .bind(retention.max(1))
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("prune solver events: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("commit solver event: {e}")))?;
        Ok(())
    }

    pub async fn recent_solver_events(&self, limit: i64) -> Result<Vec<SolverEventRow>> {
        let rows = sqlx::query(
            "SELECT id, outcome, target_path, status, latency_ms, created_at
             FROM solver_event ORDER BY id DESC LIMIT ?1",
        )
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select solver events: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| SolverEventRow {
                id: row.get("id"),
                outcome: row.get("outcome"),
                target_path: row.get("target_path"),
                status: row
                    .get::<Option<i64>, _>("status")
                    .map(|value| u16::try_from(value).unwrap_or(u16::MAX)),
                latency_ms: row
                    .get::<Option<i64>, _>("latency_ms")
                    .map(|value| u64::try_from(value).unwrap_or(u64::MAX)),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    /// Successful solves since `since` (rolling-hour budget).
    pub async fn solves_since(&self, since: i64) -> Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM solver_event WHERE outcome = 'solved' AND created_at >= ?1",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("count solver events: {e}")))
    }

    /// Timestamp of the most recent successful solve, for the cooldown.
    pub async fn last_solved_at(&self) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT created_at FROM solver_event WHERE outcome = 'solved'
             ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("last solver event: {e}")))?;
        Ok(row.map(|row| row.get("created_at")))
    }

    pub async fn save_solver_clearance(
        &self,
        ciphertext: &str,
        user_agent: &str,
        expires_at: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO solver_clearance (id, ciphertext, user_agent, expires_at, created_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               ciphertext = excluded.ciphertext,
               user_agent = excluded.user_agent,
               expires_at = excluded.expires_at,
               created_at = excluded.created_at",
        )
        .bind(ciphertext)
        .bind(user_agent)
        .bind(expires_at)
        .bind(crate::time::now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("save solver clearance: {e}")))?;
        Ok(())
    }

    pub async fn load_solver_clearance(&self) -> Result<Option<StoredClearance>> {
        let row = sqlx::query(
            "SELECT ciphertext, user_agent, expires_at FROM solver_clearance WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("load solver clearance: {e}")))?;
        Ok(row.map(|row| StoredClearance {
            ciphertext: row.get("ciphertext"),
            user_agent: row.get("user_agent"),
            expires_at: row.get("expires_at"),
        }))
    }

    pub async fn clear_solver_clearance(&self) -> Result<()> {
        sqlx::query("DELETE FROM solver_clearance")
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("clear solver clearance: {e}")))?;
        Ok(())
    }
}

fn opt_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
