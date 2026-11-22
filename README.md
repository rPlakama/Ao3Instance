this project is designed to run as a webapplication that can be acessed thought login;, providing a frontend for AO3 with additional capabilities, such as local caching for fanfics (in server) and saving scroll position of they, and other capabilities, even standing after the AO3 is down, providing a cool and elegant fanfic consumation.

Features:

 - Accounts
 - Caching
 - Local storage of fanfics that user acess
 - Full IMPL of Ao3
 - Search <--> AO3 (server)
 - Full Reader, EPUB (Main) <-- as caching
 - Optional edge-challenge fallback through a host-provided FlareSolverr
   (never a build dependency; the app works without it)
 - Structured HTTP diagnostics at `/diagnostics`
 - In Rust
 - SQLX as DB;


 Arch ref1; https://github.com/otwcode/otwarchive
 Arch ref2; https://github.com/Kareadita/Kavita <-- ePUB reading desing & features;

 This project shall utilizate flake.nix && Kunobi Kache (https://ninja.kunobi.com/kache) for Local builds. The project shall also use strict Clippy rules.

## Running it

```
nix develop -c cargo run --bin ao3i-server
# then open http://127.0.0.1:8099/
```

Paste an AO3 work id (or a `/works/<id>` URL) on the start page: the work is fetched
once, parsed and cached in SQLite, and served by a local reader. Re-opening a cached work
never touches archiveofourown.org, so it keeps working offline.

Configuration (environment, read once at startup): `AO3I_BIND`, `AO3I_PORT`,
`AO3I_DATA_DIR`, `AO3I_DB_FILE`, `AO3I_BASE_URL` (sandbox), `AO3I_USER_AGENT`,
`AO3I_WRITE_ENABLED`, `AO3I_MIN_SPACING_MS`, `AO3I_LOG_HTTP_ERRORS`,
`AO3I_HTTP_LOG_RETENTION`, and the optional `AO3I_FLARESOLVERR*` knobs. See
`docs/operator.md`.

Operator guide: [docs/operator.md](docs/operator.md). Failure behaviour:
[docs/failure-matrix.md](docs/failure-matrix.md).

Current state: M1 vertical slice (open → cache → read) plus AO3 session management
(cookie import or password login, encrypted jar), chapter-level reading progress/resume,
local full-text search over the cache, conditional refresh from AO3, a write path
(subscriptions, kudos, comments, bookmarks and AO3 reading flags) behind an explicit
confirmation step, an intent/audit ledger and a kill switch
(`AO3I_WRITE_ENABLED=true`), redacted HTTP diagnostics at `/diagnostics`, and an
optional challenge fallback that calls a host-provided FlareSolverr on an explicit
click only. Tests: `nix develop -c cargo test`.
Design and decisions: `docs/init-audit.md`, `docs/ao3-write-api.md`, `docs/adr/`.
