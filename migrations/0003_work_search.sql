-- W3: local full-text search over cached works (title, authors, tags).
-- Kept in sync by triggers so there is one writer (the works table) and no
-- application-side reindexing to forget.

CREATE VIRTUAL TABLE IF NOT EXISTS work_search USING fts5(
    title,
    authors,
    tags,
    work_id UNINDEXED,
    tokenize = 'unicode61'
);

CREATE TRIGGER IF NOT EXISTS works_search_ai AFTER INSERT ON works BEGIN
    INSERT INTO work_search (title, authors, tags, work_id)
    VALUES (new.title, new.authors_json, new.tags_json, new.ao3_id);
END;

CREATE TRIGGER IF NOT EXISTS works_search_au AFTER UPDATE ON works BEGIN
    DELETE FROM work_search WHERE work_id = old.ao3_id;
    INSERT INTO work_search (title, authors, tags, work_id)
    VALUES (new.title, new.authors_json, new.tags_json, new.ao3_id);
END;

CREATE TRIGGER IF NOT EXISTS works_search_ad AFTER DELETE ON works BEGIN
    DELETE FROM work_search WHERE work_id = old.ao3_id;
END;

-- Backfill rows cached before this migration existed.
INSERT INTO work_search (title, authors, tags, work_id)
SELECT title, authors_json, tags_json, ao3_id FROM works;
