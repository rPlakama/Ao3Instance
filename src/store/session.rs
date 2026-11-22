//! Persisted AO3 session: the encrypted cookie jar and the comment pseud.

use super::{SavedSession, Store, now_secs};
use sqlx::Row;

use crate::error::{Error, Result};

impl Store {
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
}
