-- W6: bookmark receipts. AO3 allows more than one bookmark per work by the same
-- user, so the local ledger is what keeps the reader from double-posting.

CREATE TABLE IF NOT EXISTS bookmarks (
    account     TEXT NOT NULL,
    work_id     TEXT NOT NULL,
    bookmark_id INTEGER,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (account, work_id)
);
