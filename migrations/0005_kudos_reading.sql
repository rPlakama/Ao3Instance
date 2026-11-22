-- W5: kudos receipts and AO3 reading flags. Both are per-account local mirrors
-- of an upstream fact; the UI uses them to avoid offering duplicate actions.

CREATE TABLE IF NOT EXISTS kudos_sent (
    account    TEXT NOT NULL,
    work_id    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account, work_id)
);

CREATE TABLE IF NOT EXISTS reading_flags (
    account    TEXT NOT NULL,
    work_id    TEXT NOT NULL,
    state      TEXT NOT NULL CHECK (state IN ('for_later', 'read')),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account, work_id)
);
