-- Persistent cache for live AO3 pages (search results, tag pages, account
-- lists, media/fandom directories, home modules, public skins).
--
-- Default TTL is 30 minutes (`consts::PAGE_CACHE_TTL_SECS`); stale rows also
-- serve as an offline fallback when AO3 is unreachable.
CREATE TABLE IF NOT EXISTS page_cache (
    cache_key  TEXT PRIMARY KEY,
    account    TEXT NOT NULL DEFAULT '',
    body       TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_page_cache_fetched ON page_cache (fetched_at);
CREATE INDEX IF NOT EXISTS idx_page_cache_account ON page_cache (account);
