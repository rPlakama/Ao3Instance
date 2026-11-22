-- W4: the write spine. Intent is recorded before a request leaves the process;
-- the audit row records what actually happened; the subscription ledger is the
-- client's own pre-check because AO3 has no uniqueness constraint on
-- subscriptions (docs/ao3-write-api.md §4.3).

CREATE TABLE IF NOT EXISTS write_intent (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account     TEXT NOT NULL,
    action      TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id   TEXT NOT NULL,
    state       TEXT NOT NULL DEFAULT 'pending',
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS write_audit (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id  INTEGER REFERENCES write_intent(id),
    outcome    TEXT NOT NULL,
    evidence   TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_write_audit_created ON write_audit (created_at DESC);

CREATE TABLE IF NOT EXISTS subscriptions (
    account         TEXT NOT NULL,
    target_kind     TEXT NOT NULL,
    target_id       TEXT NOT NULL,
    subscription_id INTEGER,
    state           TEXT NOT NULL CHECK (state IN ('active','removed')),
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (account, target_kind, target_id)
);
