-- W3: one persisted AO3 session (encrypted jar) and reading progress hooks.
-- The jar is AES-256-GCM ciphertext from crypto.rs (AAD "ao3/jar"); the key
-- lives in the 0600 key file, never in the database.

CREATE TABLE IF NOT EXISTS sessions (
    account_id TEXT PRIMARY KEY,        -- AO3 login
    jar_enc    TEXT NOT NULL,           -- hex(nonce||tag||ct) of the whole cookie jar
    updated_at INTEGER NOT NULL
);
