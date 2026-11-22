-- M1 storage: cached works, their chapters, and reading progress.
-- One migration dir, one migrator (docs/init-audit.md A6).

CREATE TABLE IF NOT EXISTS works (
    ao3_id        TEXT PRIMARY KEY,
    title         TEXT NOT NULL,
    authors_json  TEXT NOT NULL,
    summary       TEXT,
    tags_json     TEXT NOT NULL,          -- {fandoms, relationships, characters, additional, warnings, categories}
    rating        TEXT,
    language      TEXT,
    word_count    INTEGER,
    kudos         INTEGER,
    comment_count INTEGER,
    chapter_count INTEGER NOT NULL DEFAULT 0,
    content_hash  TEXT NOT NULL,
    fetched_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chapters (
    work_id   TEXT NOT NULL REFERENCES works(ao3_id) ON DELETE CASCADE,
    idx       INTEGER NOT NULL,           -- 1-based, as displayed
    title     TEXT,
    body_html TEXT NOT NULL,              -- raw from upstream; sanitized at render time
    notes_html TEXT,
    PRIMARY KEY (work_id, idx)
);

-- Content-anchored reading position (docs/init-audit.md I7). Single local user
-- for now; the column is here so multi-account lands without a migration.
CREATE TABLE IF NOT EXISTS progress (
    account_id TEXT NOT NULL DEFAULT 'local',
    work_id    TEXT NOT NULL REFERENCES works(ao3_id) ON DELETE CASCADE,
    chapter_idx INTEGER NOT NULL DEFAULT 1,
    block_idx   INTEGER NOT NULL DEFAULT 0,
    char_offset INTEGER NOT NULL DEFAULT 0,
    pct         REAL NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, work_id)
);

CREATE INDEX IF NOT EXISTS idx_works_fetched_at ON works (fetched_at DESC);
