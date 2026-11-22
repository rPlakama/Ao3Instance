//! The bounded `http_error_log`: insert with retention pruning, recent rows for
//! `/diagnostics`, and the explicit clear action.
//!
//! Rows are built from [`HttpErrorEvent`], whose constructor has already
//! sanitized the path and redacted the message. This module never accepts a raw
//! body, cookie or header without that scrub.

use sqlx::Row;

use crate::diag::HttpErrorEvent;
use crate::error::{Error, Result};

use super::Store;

/// One persisted diagnostic event. All fields are safe to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpErrorRow {
    pub id: i64,
    pub direction: String,
    pub transport: String,
    pub method: String,
    pub path: String,
    pub status: Option<u16>,
    pub error_kind: String,
    pub message: String,
    pub latency_ms: Option<u64>,
    pub retry_after: Option<u64>,
    pub location_path: Option<String>,
    pub server_header: Option<String>,
    pub cf_ray: Option<String>,
    pub content_type: Option<String>,
    pub created_at: i64,
}

impl HttpErrorRow {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Self {
        Self {
            id: row.get("id"),
            direction: row.get("direction"),
            transport: row.get("transport"),
            method: row.get("method"),
            path: row.get("path"),
            status: row
                .get::<Option<i64>, _>("status")
                .map(|value| u16::try_from(value).unwrap_or(u16::MAX)),
            error_kind: row.get("error_kind"),
            message: row.get("message"),
            latency_ms: row
                .get::<Option<i64>, _>("latency_ms")
                .map(|value| u64::try_from(value).unwrap_or(u64::MAX)),
            retry_after: row
                .get::<Option<i64>, _>("retry_after")
                .map(|value| u64::try_from(value).unwrap_or(u64::MAX)),
            location_path: row.get("location_path"),
            server_header: row.get("server_header"),
            cf_ray: row.get("cf_ray"),
            content_type: row.get("content_type"),
            created_at: row.get("created_at"),
        }
    }
}

impl Store {
    /// Inserts one event and prunes everything beyond `retention` rows.
    pub async fn record_http_error(&self, event: &HttpErrorEvent, retention: i64) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Store(format!("begin http error tx: {e}")))?;

        sqlx::query(
            "INSERT INTO http_error_log
             (direction, transport, method, path, status, error_kind, message, latency_ms,
              retry_after, location_path, server_header, cf_ray, content_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .bind(event.direction.as_str())
        .bind(event.transport.as_str())
        .bind(&event.method)
        .bind(&event.path)
        .bind(event.status.map(i64::from))
        .bind(event.error_kind)
        .bind(&event.message)
        .bind(event.latency_ms.map(opt_i64))
        .bind(event.retry_after.map(opt_i64))
        .bind(event.location_path.as_deref())
        .bind(event.server_header.as_deref())
        .bind(event.cf_ray.as_deref())
        .bind(event.content_type.as_deref())
        .bind(event.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("insert http error: {e}")))?;

        sqlx::query(
            "DELETE FROM http_error_log WHERE id NOT IN
             (SELECT id FROM http_error_log ORDER BY id DESC LIMIT ?1)",
        )
        .bind(retention.max(1))
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Store(format!("prune http errors: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| Error::Store(format!("commit http error: {e}")))?;
        Ok(())
    }

    /// Newest-first rows, optionally narrowed to `upstream` or `inbound`.
    pub async fn recent_http_errors(
        &self,
        limit: i64,
        direction: Option<&str>,
    ) -> Result<Vec<HttpErrorRow>> {
        let rows = sqlx::query(
            "SELECT id, direction, transport, method, path, status, error_kind, message,
                    latency_ms, retry_after, location_path, server_header, cf_ray,
                    content_type, created_at
             FROM http_error_log
             WHERE (?1 IS NULL OR direction = ?1)
             ORDER BY id DESC LIMIT ?2",
        )
        .bind(direction)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Store(format!("select http errors: {e}")))?;

        Ok(rows.iter().map(HttpErrorRow::from_row).collect())
    }

    /// Operator action: empty the table. Returns the number of rows removed.
    pub async fn clear_http_errors(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM http_error_log")
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Store(format!("clear http errors: {e}")))?;
        Ok(result.rows_affected())
    }
}

fn opt_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
