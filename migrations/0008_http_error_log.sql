-- Phase 2: structured, redacted HTTP diagnostics. Upstream failures (transport,
-- non-2xx, unexpected redirects) and inbound 4xx/5xx responses both land here so
-- /diagnostics can explain an edge block without reading the journal.
--
-- `transport` records which path produced the event: the direct client or the
-- host-provided FlareSolverr (Phase 2.5). No column may hold a cookie, CSRF
-- token, password or response body; `diag::redact_message` enforces it.

CREATE TABLE IF NOT EXISTS http_error_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    direction     TEXT NOT NULL CHECK(direction IN ('upstream', 'inbound')),
    transport     TEXT NOT NULL DEFAULT 'direct' CHECK(transport IN ('direct', 'solver')),
    method        TEXT NOT NULL,
    path          TEXT NOT NULL,
    status        INTEGER,
    error_kind    TEXT NOT NULL,
    message       TEXT NOT NULL,
    latency_ms    INTEGER,
    retry_after   INTEGER,
    location_path TEXT,
    server_header TEXT,
    cf_ray        TEXT,
    content_type  TEXT,
    created_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_http_error_log_created ON http_error_log(created_at DESC);
