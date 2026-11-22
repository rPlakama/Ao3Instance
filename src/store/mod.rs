//! Work cache: store a parsed work, read it back, list what is cached.
//!
//! The tables divide by job: works and reading progress ([`works`]), the write
//! ledger and local receipts ([`writes`]), and the persisted AO3 session
//! ([`session`]). This module owns the shared row types and the `Store` handle.

mod errors;
mod lists;
mod session;
mod solver;
#[cfg(test)]
mod tests;
mod works;
mod writes;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

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
/// Optional filters for offline search; `sort = None` keeps FTS rank order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilter {
    pub rating: Option<String>,
    pub fandom: Option<String>,
    pub language: Option<String>,
    pub sort: Option<String>,
}

/// One page of offline search hits plus the total match count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    pub works: Vec<StoredWork>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
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
}

pub(super) use crate::time::now_secs;
pub use errors::HttpErrorRow;
pub use lists::{
    BookmarkEntry, BrowseResult, BrowseSort, CacheStats, ReadingEntry, ReadingStatus,
    SubscriptionEntry, WorkFilter,
};
pub use solver::{SolverEventRow, StoredClearance};
pub use works::sha256_hex;
