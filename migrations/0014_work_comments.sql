-- Reader comments: a cached copy of AO3's public comment threads.
--
-- AO3 attaches comments to a "commentable": the work itself (Entire Work view,
-- or a one-chapter work) or one chapter of a multi-chapter work. The cache keys
-- comments by that scope so a reader always sees the same thread AO3 shows for
-- the URL they are on. Rows are replaced wholesale when the thread is refreshed.
--
-- Comment bodies are untrusted author content. They are stored raw (like chapter
-- text) and sanitized by the render layer; they are never written to the HTTP
-- diagnostics log.

-- AO3 chapter ids are what the comment fetch needs: `works/:id/chapters/:chapter_id`.
-- The work page's chapter header links carry them (`chapters/_chapter.html.erb`).
ALTER TABLE chapters ADD COLUMN ao3_chapter_id TEXT;

-- One row per fetched commentable: proves "we looked" even when there are zero
-- comments, and carries the visible-comment total AO3 displayed.
CREATE TABLE IF NOT EXISTS work_comment_sets (
    work_id    TEXT NOT NULL,
    scope      TEXT NOT NULL,
    total      INTEGER,
    fetched_at INTEGER NOT NULL,
    has_more   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (work_id, scope)
);

CREATE TABLE IF NOT EXISTS work_comments (
    work_id      TEXT NOT NULL,
    scope        TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    comment_id   TEXT NOT NULL,
    parent_id    TEXT,
    depth        INTEGER NOT NULL,
    author       TEXT,
    author_login TEXT,
    posted_at    TEXT,
    edited_at    TEXT,
    body_html    TEXT NOT NULL,
    PRIMARY KEY (work_id, scope, comment_id)
);

CREATE INDEX IF NOT EXISTS idx_work_comments_order
    ON work_comments (work_id, scope, seq);
