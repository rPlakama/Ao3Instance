-- Local-only difference from AO3: which cached works were opened/read recently.
CREATE TABLE IF NOT EXISTS access_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id     TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('read', 'cached')),
    accessed_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_access_log_accessed ON access_log(accessed_at DESC);
