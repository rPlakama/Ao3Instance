//! Write ledger (intent before the request, audit after), subscriptions,
//! kudos, bookmarks and reading flags — the local record of public actions.

use super::{AuditEntry, Store, SubscriptionRow, now_secs};
use sqlx::Row;

use crate::error::{Error, Result};

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

    pub async fn mark_bookmark(
        &self,
        account: &str,
        work_id: &str,
        bookmark_id: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO bookmarks (account, work_id, bookmark_id, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account, work_id) DO UPDATE SET bookmark_id = excluded.bookmark_id",
        )
        .bind(account)
        .bind(work_id)
        .bind(bookmark_id)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("mark bookmark: {e}")))?;
        Ok(())
    }

    pub async fn has_bookmark(&self, account: &str, work_id: &str) -> Result<bool> {
        let row = sqlx::query("SELECT 1 FROM bookmarks WHERE account = ?1 AND work_id = ?2")
            .bind(account)
            .bind(work_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("has bookmark: {e}")))?;
        Ok(row.is_some())
    }

    /// Drops the local bookmark receipt after AO3 deleted the bookmark.
    pub async fn remove_bookmark(&self, account: &str, work_id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM bookmarks WHERE account = ?1 AND work_id = ?2")
            .bind(account)
            .bind(work_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("remove bookmark: {e}")))?;
        Ok(result.rows_affected() > 0)
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
}
