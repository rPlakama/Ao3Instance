-- Phase 2.5: the optional host-provided FlareSolverr fallback.
--
-- `solver_event` records outcomes only (no HTML, no cookies, no session).
-- `solver_clearance` stores the Cloudflare clearance AES-256-GCM-encrypted with
-- the same key file as the AO3 session; `ciphertext` is hex(encrypt(...)).

CREATE TABLE IF NOT EXISTS solver_event (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    outcome     TEXT NOT NULL CHECK(outcome IN ('solved','failed','unavailable','captcha','timeout')),
    target_path TEXT NOT NULL,
    status      INTEGER,
    latency_ms  INTEGER,
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_solver_event_created ON solver_event(created_at DESC);

CREATE TABLE IF NOT EXISTS solver_clearance (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    ciphertext TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
