# AO3Instance — Implementation Plan: 1:1 AO3 Frontend, HTTP Error Diagnostics & Cloudflare Fallback

> **Status:** Plan only (no source edits applied yet).  
> **Scope note:** This plan covers the **1:1 AO3-styled frontend across all segments**, **structured HTTP error logging & diagnostics**, and an **opt-in Cloudflare challenge fallback through a host-provided FlareSolverr service**.  
> **FlareSolverr posture (read first):** FlareSolverr is **not a build dependency and never becomes one**. It is a Python/Chromium proxy server the *host* installs and supervises (on NixOS: `services.flaresolverr.enable = true;`, port `8191`). `Cargo.toml`, `Cargo.lock`, `flake.nix` (`buildInputs`, `nativeBuildInputs`, devShell, wrapper), and `nix build` must contain **no** reference to it and must keep working when it is absent. The app never spawns, downloads, or vendors it; it only calls the already-running loopback HTTP API (`http://127.0.0.1:8191/v1`) at runtime, and degrades to today's honest-`403` behavior when that endpoint is unreachable.  
> **Still excluded:** automatic or background challenge solving. A solve is always an explicit user action for a single page — never a retry loop, never a crawler, never a write. The existing guarantees are unchanged: offline-first reads (`I3`), honest `AO3I_USER_AGENT` for direct traffic (`docs/adr/0002-access-posture.md`), strict redaction (`docs/security-standards.md` §5), and one upstream request in flight.

---

## 0. Architectural Constraints & Invariants

Every task below must preserve the repository's existing contracts:

1. **Server-rendered HTML (no SPA):** Matches `docs/write-client-plan.md` D5. Progressive enhancement only; every core flow (login, open, read, search, filter, writes, confirmation) works without JavaScript.
2. **Untrusted content isolation (`T1` / `I10`):** Author HTML (`summary`, chapter body, chapter notes) continues to be sanitized with `ammonia` and rendered inside a sandboxed `<iframe>` served with strict `CHAPTER_CSP` (`src/server.rs:37-38`). Styling the reader 1:1 to AO3 must never weaken the iframe sandbox or CSP.
3. **Zero GPLv2 code copying (`docs/ao3-internals.md` §1):** The gitignored `otwarchive/` checkout is reference-only. All HTML markup, CSS rules, and SVG icons in `AO3Instance` must be authored cleanly from scratch.
4. **Strict redaction (`docs/security-standards.md` §5):** HTTP error logs (stderr and SQLite) must **never** record `_otwarchive_session`, `user_credentials`, `Cookie`, `Set-Cookie`, passwords, `authenticity_token`, or raw upstream HTML/comment bodies.
5. **Offline-first guarantee (`I3` / `G8`):** Every browse, search, fandom, tag, reader, dashboard, audit, and diagnostics route resolves strictly from SQLite without touching upstream.
6. **Strict Clippy & Modularity (`Cargo.toml`, `G9`):**
   - `unwrap_used = "deny"`, `unused_result_ok = "deny"`, `allow_attributes_without_reason = "deny"`, `todo = "deny"`, `dbg_macro = "deny"`.
   - Keep every source file ≤ 800 LOC and ≤ 60 functions (splitting `src/server.rs` [1,403 LOC] and `src/render.rs` [723 LOC] first).
7. **Nix packaging (`flake.nix`):** `allowTrees` includes `src`, `tests`, `migrations`, `scripts`. Embed static CSS/SVG assets under `src/render/assets/` with `include_str!` so `nix build` and `nix run .` remain self-contained single-binary builds.
8. **FlareSolverr is a runtime system service, never a build input (`dependency-logic`):** no crate, no vendored Python, no flake input, no `buildInputs`/`nativeBuildInputs`/devShell/wrapper change, no `build.rs` probe. The whole integration is one loopback HTTP endpoint plus host provisioning documented in `docs/operator.md`. `nix build .#default` and every offline test must pass with FlareSolverr absent, and the app must never spawn or supervise it.
9. **Challenge solving is explicit, read-only, bounded and redacted:** only an explicit `POST /solve` for a single allowlisted read may contact the solver — never a `GET`, never a write, never an automatic retry, never a batch operation. Solver cookies and Cloudflare clearance tokens are secrets: encrypted at rest through `src/crypto.rs`, and never logged, rendered, audited, or `Debug`-formatted. The direct (`AO3I_USER_AGENT`) path stays the default and the only write path.

---

## Phase 1: Modular Refactor (Prep for Frontend & Diagnostics)

> **Goal:** Break `src/render.rs` and `src/server.rs` into focused submodules with zero behavioral change, so all existing tests stay green before adding new UI segments and HTTP error logging.

- [ ] **1.1 Split `src/render.rs` into `src/render/` module tree**
  - [ ] Convert `src/render.rs` → `src/render/mod.rs` (re-exporting `escape`, `sanitize`, `ReadContext`, and page renderers so callers stay clean).
  - [ ] Extract shared chrome & utilities into `src/render/chrome.rs` (`layout`, `format_utc`, `checked_label`, `civil_from_days`, `now_secs`).
  - [ ] Extract individual segment renderers:
    - `src/render/home.rs` (`home_page`)
    - `src/render/work.rs` (`read_page`, `chapter_document`)
    - `src/render/session.rs` (`session_page`)
    - `src/render/search.rs` (`search_page`)
    - `src/render/audit.rs` (`confirm_page`, `audit_page`, `error_page`)
  - [ ] Move unit tests from `src/render.rs` to their respective submodules.
- [ ] **1.2 Split `src/server.rs` into `src/server/` module tree**
  - [ ] Convert `src/server.rs` → `src/server/mod.rs` (`AppState`, `router`, `run`, `shutdown`, `health`).
  - [ ] Extract `src/server/error.rs` (`AppError`, `Handled<T>`, `IntoResponse`, `From<Error>`).
  - [ ] Extract `src/server/reads.rs` (`home`, `open_work`, `fetch_work_html`, `read_work`, `read_chapter`, `search`, `save_progress`, gate & ID helpers).
  - [ ] Extract `src/server/session.rs` (`session_page`, `session_import`, `session_login`, `session_probe`, `session_adult`, `session_logout`, `persist_session`, cookie validation).
  - [ ] Extract `src/server/writes.rs` (`write_preview`, `write_subscribe`, `write_unsubscribe`, `write_kudos`, `write_reading`, `write_comment_preview`, `write_comment`, `write_bookmark`, `ensure_pseud`, `notice_text`, `audit`).
- [ ] **1.3 Verify Phase 1 baseline**
  - [ ] Run `nix develop -c cargo clippy --all-targets -- -D warnings` and `nix develop -c cargo test` — verify 100% existing tests pass with zero warnings.

---

## Phase 2: Structured HTTP Error Logging & Diagnostics

> **Goal:** Capture rich, secret-redacted diagnostic records for every upstream HTTP failure (non-2xx, unexpected redirects, rate limits, edge blocks, transport/DNS/TLS/timeout/body-cap errors) and inbound server 4xx/5xx error, logged both to `stderr` (for terminal / systemd journal) and persisted to a bounded SQLite table viewable in the UI.

### 2.1 Configuration & Schema

- [ ] **2.1.1 Add diagnostic configuration knobs in `src/config.rs` & `src/consts.rs`**
  - [ ] Add `HTTP_ERROR_LOG_RETENTION: i64 = 500` in `src/consts.rs`.
  - [ ] Add optional env vars in `src/config.rs` (`Config::from_pairs`):
    - `AO3I_LOG_HTTP_ERRORS` (bool, default `true`) — controls structured stderr emission for HTTP errors.
    - `AO3I_HTTP_LOG_RETENTION` (positive integer, default `500`) — max rows retained in SQLite `http_error_log`.
  - [ ] Add unit tests in `src/config.rs` for defaults, overrides, and invalid values.
- [ ] **2.1.2 Add SQLite migration `migrations/0008_http_error_log.sql`**
  - [ ] Create table `http_error_log`:
    ```sql
    CREATE TABLE IF NOT EXISTS http_error_log (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        direction     TEXT NOT NULL CHECK(direction IN ('upstream', 'inbound')),
        transport     TEXT NOT NULL DEFAULT 'direct' CHECK(transport IN ('direct', 'solver')),
        method        TEXT NOT NULL,
        path          TEXT NOT NULL,
        status        INTEGER,              -- NULL on transport/connect/timeout error
        error_kind    TEXT NOT NULL,        -- e.g. 'network', 'rate_limited', 'blocked_by_edge', 'upstream_changed', 'bad_gateway', ...
        message       TEXT NOT NULL,        -- redacted human/diagnostic summary
        latency_ms    INTEGER,
        retry_after   INTEGER,
        location_path TEXT,                 -- path-only, query stripped
        server_header TEXT,                 -- safe diagnostic header (e.g. 'cloudflare', 'nginx')
        cf_ray        TEXT,                 -- Cloudflare Ray ID if present (safe, operator-useful)
        content_type  TEXT,                 -- response Content-Type if present
        created_at    INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_http_error_log_created ON http_error_log(created_at DESC);
    ```
  - [ ] `transport` records which path produced the event — the direct `Client` or the host-provided FlareSolverr (§2.5); it ships in `0008` because that migration is not written yet.
  - [ ] Verify migration idempotency in `src/db.rs` tests.

### 2.2 Redaction & Diagnostic Logger (`src/diag.rs`)

- [ ] **2.2.1 Create `src/diag.rs` and wire into `src/lib.rs`**
  - [ ] Define `HttpDirection` (`Upstream`, `Inbound`) and `HttpErrorEvent` struct with all safe diagnostic fields.
  - [ ] Implement `sanitize_path(raw: &str) -> String`:
    - Strips scheme/host if present and strips query values for sensitive keys (or retains only path + allowlisted safe query keys like `view_full_work`, `chapter`).
    - Truncates overly long paths safely on UTF-8 boundaries.
  - [ ] Implement `sanitize_location(raw: Option<&str>) -> Option<String>`:
    - Extracts path segment only; never retains query parameters or tokens.
  - [ ] Implement `redact_message(msg: &str) -> String`:
    - Defensive scrubber ensuring cookie names/values (`_otwarchive_session=...`, `user_credentials=...`), `authenticity_token=...`, or control chars/newlines (`\r`, `\n`) cannot inject fake log lines or leak credentials.
  - [ ] Implement `emit_stderr(event: &HttpErrorEvent)` formatting a single-line structured log (RFC3339/UTC timestamp + `level=ERROR component=http direction=... method=... path=... status=... kind=... latency_ms=... retry_after=... cf_ray=... server=... msg="..."`) so it is greppable and systemd/journald-friendly with zero extra crates.
  - [ ] Add comprehensive unit tests in `src/diag.rs`:
    - Path & location sanitization strips query tokens and CRLF.
    - Redaction tests verify `_otwarchive_session`, `user_credentials`, passwords, and CSRF tokens never appear in emitted log lines.

### 2.3 Upstream HTTP Instrumentation (`src/ao3/client.rs` & `src/ao3/session.rs`)

- [ ] **2.3.1 Extend `ao3::client::Response` with safe diagnostic metadata**
  - [ ] Add fields to `Response` (all safe to format in `Debug`):
    - `pub method: String`
    - `pub path: String` (sanitized)
    - `pub latency_ms: u64`
    - `pub server_header: Option<String>` (`Server` header)
    - `pub cf_ray: Option<String>` (`cf-ray` header)
    - `pub content_type: Option<String>` (`Content-Type` header)
    - `pub transport: Transport` (`Direct` | `Solver`, §2.5.3 — safe to format)
  - [ ] Keep `Response`'s hand-written `Debug` hiding `body` and `set_cookies` (verify existing test `response_debug_hides_body_and_cookies` still passes).
- [ ] **2.3.2 Add diagnostic hook / sink to `ao3::client::Client`**
  - [ ] Measure `started = Instant::now()` around `request.send().await` and body streaming in `Client::send`.
  - [ ] Pass `method` and `path` into `Client::send` (instead of losing them inside `RequestBuilder`).
  - [ ] On `reqwest` transport error (`send` or `chunk`):
    - Classify error detail (connect refused, timeout, DNS, TLS, body read) via `reqwest::Error` inspection methods (`is_timeout()`, `is_connect()`, `is_request()`, `is_body()`).
    - Build `HttpErrorEvent` (`direction: Upstream`, `status: None`, `error_kind: "network"`, `latency_ms`) and emit via configured logger / callback.
  - [ ] On oversized body (`content-length` cap or streaming cap):
    - Emit `HttpErrorEvent` (`error_kind: "body_cap_exceeded"`, `status: Some(status)`).
  - [ ] On HTTP status ≥ 400 (including `403`, `404`, `422`, `429`, `500`, `502`, `503`, `504`):
    - Classify `error_kind`: `429` → `"rate_limited"`, `403 | 503` → `"blocked_by_edge"`, `404` → `"not_found"`, `422` → `"validation"`, `5xx` → `"upstream_server_error"`, other `4xx` → `"upstream_client_error"`.
    - Build `HttpErrorEvent` with `status`, `retry_after`, `server_header`, `cf_ray`, `content_type`, `latency_ms` and emit + buffer/forward to persistence.
  - [ ] On unexpected/auth-failure 3xx redirects (`/users/login`, `/lost_cookie`, `/auth_error`):
    - Record diagnostic event with `location_path` (path only) and `error_kind` (`"redirect_login"`, `"redirect_lost_cookie"`, `"redirect_auth_error"`).
  - [ ] Tag every event with `transport` (`direct`/`solver`) and, for solver-originated failures, use the `solver_*` kinds from §2.5.7.3; the `403`/`503` path stays `blocked_by_edge` and is what offers the explicit solve action.
  - [ ] Store an optional `Arc<dyn Fn(HttpErrorEvent) + Send + Sync>` or `tokio::sync::mpsc::Sender<HttpErrorEvent>` (bounded channel, capacity 256, drop-newest on saturation per `init-audit.md` A7) on `Client` so `ao3::client` remains decoupled from `Store` while `server` drains events into SQLite cleanly.
- [ ] **2.3.3 Add `Store` methods for HTTP error logs (`src/store.rs`)**
  - [ ] `Store::record_http_error(&self, event: &HttpErrorEvent, retention: i64) -> Result<()>` — inserts row and prunes rows beyond `retention` (`DELETE FROM http_error_log WHERE id NOT IN (SELECT id FROM http_error_log ORDER BY id DESC LIMIT ?1)`).
  - [ ] `Store::recent_http_errors(&self, limit: i64, direction: Option<&str>) -> Result<Vec<HttpErrorRow>>`.
  - [ ] `Store::clear_http_errors(&self) -> Result<u64>` (explicit operator action via POST).
  - [ ] Add unit tests in `src/store.rs` verifying insertion, filtering by direction, and retention pruning.

### 2.4 Inbound Server HTTP Error Instrumentation (`src/server/`)

- [ ] **2.4.1 Instrument `AppError` and inbound HTTP error responses**
  - [ ] Add Axum middleware (`axum::middleware::from_fn_with_state`) capturing request `Method`, sanitized `Uri` path, elapsed `latency_ms`, and response `StatusCode`.
  - [ ] Whenever the response status is `4xx` or `5xx`, emit a structured `HttpErrorEvent` (`direction: Inbound`) to `stderr` (if enabled) and send to the bounded SQLite diagnostic writer.
  - [ ] Enrich `AppError` with an optional diagnostic hint (e.g., for `403`/`503`: remind operator to check `/diagnostics`, verify `AO3I_USER_AGENT`, wait before retrying, refresh browser session cookies on `/session`, and — only when a reachable solver is configured — offer the explicit `Solve & cache` action from §2.5.6 with no automatic retry; for `429`: show exact `Retry-After` seconds).
  - [ ] Verify existing error substrings asserted by `tests/failure_matrix.rs` (`"refused the request"`, `"redirected this work"`, `"rate limiting"`, `"request failed"`) remain intact.

---

## Phase 2.5: Cloudflare Challenge Fallback via host-provided FlareSolverr

> **Goal:** When AO3's edge answers a **single user-initiated read** with a Cloudflare interstitial (`403`/`503`), let the user resolve that one page through the FlareSolverr service the host already runs, then cache it and continue offline as usual. The result is an observable, budgeted, opt-in fallback — not a crawler and not a bypass loop.  
> **Hard dependency rule (`dependency-logic`):** FlareSolverr is a *system* service, not a build input. No Rust crate, no vendored Python, no flake input, no `build.rs` probe, no wrapper `PATH` change. The entire integration is one loopback HTTP endpoint plus host provisioning documented in `docs/operator.md`.  
> **Integration shape:** the app speaks HTTP to the running service; it does **not** shell out to a `flaresolverr` binary either. Process supervision belongs to systemd (or the container runtime), and spawning a Chromium-per-solve child from inside the web server would be a lifecycle and security liability. `PATH` is irrelevant — the loopback origin is the contract.

### 2.5.1 Configuration & discovery (`src/config.rs`, `src/consts.rs`)

- [ ] **2.5.1.1 Add solver knobs** (all optional; invalid values rejected in the existing `Error::Config` style):
  - [ ] `AO3I_FLARESOLVERR` — `auto` (**default**) | `on` | `off`. `auto`: probe at startup (and lazily after a block) and offer the solver only when reachable, silent when not. `on`: same, but `/diagnostics` and the challenge card show a persistent "configured but unreachable" warning. `off`: never probe, never contact; `POST /solve` answers `403` with an explanation.
  - [ ] `AO3I_FLARESOLVERR_URL` — default `http://127.0.0.1:8191`; normalized to an origin (no path/query). Must be `http://` or `https://`, and its host must be loopback (`127.0.0.0/8`, `::1`, `localhost`) unless `AO3I_FLARESOLVERR_ALLOW_REMOTE=1`. `services.flaresolverr` always listens on `0.0.0.0` internally, so a container/bridge address needs that explicit opt-in. Anything else → `Error::Config`.
  - [ ] `AO3I_FLARESOLVERR_ALLOW_REMOTE` — default `false`; exposing the solver port is a remote-browser-abuse hole, so this must be a deliberate act.
  - [ ] `AO3I_FLARESOLVERR_SEND_COOKIES` — default `false`: whether the AO3 jar may ever be handed to the solver; the per-solve checkbox still gates each request (§2.5.6).
  - [ ] `AO3I_FLARESOLVERR_MAX_TIMEOUT_MS` — default `60000` (FlareSolverr's own `maxTimeout`); accept `5000..=180000`, reject out-of-range with `Error::Config`.
- [ ] **2.5.1.2 Constants in `src/consts.rs`:**
  - `FLARESOLVERR_DEFAULT_URL: &str = "http://127.0.0.1:8191"`
  - `FLARESOLVERR_PROBE_TIMEOUT: Duration = 2 s`; `FLARESOLVERR_SOLVE_TIMEOUT: Duration = 120 s`
  - `FLARESOLVERR_MAX_TIMEOUT_MS: u64 = 60_000`
  - `FLARESOLVERR_COOLDOWN: Duration = 5 min`; `FLARESOLVERR_MAX_SOLVES_PER_HOUR: usize = 6`
  - `FLARESOLVERR_CLEARANCE_TTL: Duration = 30 min`
  - `SOLVER_EVENT_RETENTION: i64 = 200`
- [ ] **2.5.1.3 Keep `Config::from_pairs` pure** (no `std::env` outside `from_env`): add `pub solver: SolverConfig { mode, origin, allow_remote, send_cookies, max_timeout }` plus a `SolverMode` enum with `Display`.
- [ ] **2.5.1.4 Tests in `src/config.rs`:** default `auto` + loopback origin; every override; `off`; invalid mode; `ftp://`, empty, URL-with-path; remote host rejected without `ALLOW_REMOTE` and accepted with it; timeout bounds.

### 2.5.2 Solver client (`src/ao3/solver.rs`)

- [ ] **2.5.2.1 Types (secrets opaque by construction):**
  - `SolverStatus { mode, reachable, version: Option<String>, checked_at: i64 }` — safe to render and log.
  - `SolverCookie` — hand-written `Debug`/`Display` print `name`/`domain` only; `value` is a `SecretString` and never serialized into diagnostics.
  - `SolveOutcome { status, html, cookies: Vec<SolverCookie>, user_agent: String, latency_ms }` — `Debug` prints `html_len`, cookie count, status only (mirrors `Response`'s pattern).
- [ ] **2.5.2.2 `Solver::probe()`** — `POST <origin>/v1` with `{"cmd":"sessions.list"}` inside `FLARESOLVERR_PROBE_TIMEOUT`; reachable iff HTTP 2xx **and** the body parses as JSON (FlareSolverr does not guarantee a `status` field on every command, so only an explicit `status:"error"` counts as unreachable); capture `version` best-effort. Any failure becomes `SolverStatus { reachable: false }`. Probing is never fatal and never touches AO3 (it only talks to the local service).
- [ ] **2.5.2.3 `Solver::solve_get(target, cookies, max_timeout_ms)`** — `POST <origin>/v1` with:
  ```json
  {"cmd":"request.get","url":"<origin+path>","maxTimeout":60000,"disableMedia":true,
   "cookies":[{"name":"_otwarchive_session","value":"…"}]}
  ```
  - `status:"ok"` → read `solution.status`, `solution.response`, `solution.cookies`, `solution.userAgent`, `solution.url`.
  - `status:"error"` → `Error::SolverFailed(redact_message(&message))`; a message mentioning captcha → `Error::SolverCaptcha`.
  - transport/JSON failure → `Error::SolverUnavailable`; non-2xx → `Error::SolverFailed` carrying the status only (never the body).
  - Reject `solution.response` larger than `MAX_BODY_BYTES` **before** parsing (`solver_body_cap`); missing/empty solution → `SolverFailed`.
  - One-shot solves only: no `sessions.create`/`sessions.destroy` in v1 (fresh, short-lived browser; bounded memory, no cross-request browser state).
- [ ] **2.5.2.4 Redaction:** the solver's `message` and `solution.url` pass through `diag::sanitize_path`/`redact_message`. Cookie values, `cf_clearance`, `__cf_bm`, response HTML, and the solver request body are **never** logged, audited, rendered, or stored in plaintext.
- [ ] **2.5.2.5 Tests in `src/ao3/solver.rs`:** valid/invalid JSON; `status:error` + captcha; non-2xx; timeout (mock `delay`); oversized `solution.response`; empty solution; `Debug` hides cookie values and HTML.

### 2.5.3 Transport & session integration (`src/ao3/session.rs`, `src/ao3/client.rs`)

- [ ] **2.5.3.1 Add `Transport { Direct, Solver }`** to the `Response` diagnostic fields (§2.3.1) and to `http_error_log.transport` (§2.1.2) so a block is traceable to the path that produced it.
- [ ] **2.5.3.2 `AccountSession::solve_read(path, send_cookies) -> Result<Response>`** — the only solver entry point:
  - refuse when the session is `Killed` (`Error::Killed`) or mode is `off` (`Error::SolverDisabled`);
  - compose the absolute URL from `base_url` + an already-validated path (the route handler owns the allowlist);
  - pass jar cookies only when the per-solve checkbox **and** `AO3I_FLARESOLVERR_SEND_COOKIES` are both true;
  - on success, inspect `solution.url`'s path: `/users/login` → `Error::LoggedOut`, `/lost_cookie` → `Error::SessionIncomplete` (the browser follows redirects, so a `200` alone lies);
  - merge only cookies whose domain matches the AO3 host into the jar (`Jar::apply_all` semantics), and drop the CSRF token exactly as a direct response would;
  - install `cf_clearance`/`__cf_bm` + `userAgent` into the clearance store (§2.5.4) with `FLARESOLVERR_CLEARANCE_TTL`, then return a `Response { status, body, transport: Solver, .. }` for the existing parser/cache path.
- [ ] **2.5.3.3 Clearance window:** while a clearance is active, **direct** user-initiated reads may use the solver-reported browser `user_agent` plus the clearance cookie — the single sanctioned UA override, scoped to the window and cleared on expiry, on any `403`, or on jar change. Without an active clearance, direct traffic keeps the honest `AO3I_USER_AGENT` (ADR-0002 §5), and writes always use it.
- [ ] **2.5.3.4 Read-only scope guard:** solver support is reads only. `post_form`, `patch_form`, `delete`, `delete_with_csrf`, and every write handler keep the direct `Client`; `request.post` is never sent to the solver in v1. A `403` on a write shows diagnostics + browser-authentication guidance, not a solve button (replaying a write in a browser could double-post and bypasses the governor and the CSRF flow).
- [ ] **2.5.3.5 Update the ADR trail:** add `docs/adr/0007-flaresolverr-fallback.md` amending ADR-0002 §5 (scoped UA override) and its "challenge is terminal" consequence: still terminal *automatically*, but the operator may explicitly spend one bounded solve per page. State the risk plainly (Cloudflare protects AO3 from abuse; this must not become scraping) and the rollback (`AO3I_FLARESOLVERR=off`).

### 2.5.4 Clearance storage (`migrations/0009_flaresolverr.sql`, `src/store.rs`)

- [ ] **2.5.4.1 Migration** (idempotent, in the style of `0001`–`0007`):
  ```sql
  CREATE TABLE IF NOT EXISTS solver_event (
      id          INTEGER PRIMARY KEY AUTOINCREMENT,
      outcome     TEXT NOT NULL CHECK(outcome IN ('solved','failed','unavailable','captcha','timeout')),
      target_path TEXT NOT NULL,          -- sanitized, query stripped
      status      INTEGER,
      latency_ms  INTEGER,
      created_at  INTEGER NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_solver_event_created ON solver_event(created_at DESC);

  CREATE TABLE IF NOT EXISTS solver_clearance (
      id         INTEGER PRIMARY KEY CHECK (id = 1),
      ciphertext BLOB NOT NULL,           -- AES-256-GCM(cf_clearance + __cf_bm), same key file
      user_agent TEXT NOT NULL,           -- browser UA bound to the clearance (not secret)
      expires_at INTEGER NOT NULL,
      created_at INTEGER NOT NULL
  );
  ```
- [ ] **2.5.4.2 `Store` methods:** `record_solver_event` (insert + prune beyond `SOLVER_EVENT_RETENTION`), `recent_solver_events(limit)`, `solves_since(ts) -> i64` (budget), `save_solver_clearance` / `load_solver_clearance` / `clear_solver_clearance`. Clearance goes through `src/crypto.rs` (same AES-256-GCM secret store and key file as the session); a corrupt or expired row is treated as absent **and deleted**.
- [ ] **2.5.4.3 Tests in `src/store.rs`:** record/prune/filter, hourly budget count, ciphertext contains no `cf_clearance` plaintext, corrupted row ignored + cleared, migration idempotency alongside the existing migration tests.

### 2.5.5 Routes & handlers (`src/server/solver.rs`)

- [ ] **2.5.5.1 `POST /solve`** — form fields:
  - `target` — a work id or an allowlisted AO3 read path (`/works/<id>`, `/works/<id>/chapters/<n>`, `/users/<name>/pseuds`); anything else is `400`. No arbitrary URLs, no write paths — the app composes the absolute URL itself.
  - `next` — local redirect target (must start with `/`; default `/read/<id>`).
  - `send_cookies` — checkbox, effective only with `AO3I_FLARESOLVERR_SEND_COOKIES=true`.
  - Outcomes: `303` + `.flash.notice` "resolved via local FlareSolverr; cached offline" on success; `409` when a solve is already running; `429` when cooldown/hourly budget is exhausted (include the wait-until time); `503` solver unavailable (`auto`/`on`); `502` solver failed/captcha; `403` when mode is `off`. Every failure keeps the user on an AO3-styled page with `/diagnostics` and `/session` links — never an automatic retry.
- [ ] **2.5.5.2** `POST /diagnostics/solver/probe` — re-probe locally; no AO3 traffic. `POST /diagnostics/solver/clearance` — `clear_solver_clearance` + drop the in-memory copy.
- [ ] **2.5.5.3 Single-flight & politeness:** a `Semaphore(1)`/`AtomicBool` guard in `AppState` so two tabs cannot launch two browsers; the existing `MIN_REQUEST_SPACING` floor applies before the solve; the solve counts as one upstream request in the audit trail; a `Retry-After` from the preceding `403`/`429` keeps `/solve` disabled until it elapses.
- [ ] **2.5.5.4 GET handlers stay network-free:** the challenge card renders from stored state; solving only happens in the `POST`. `GET /works/<id>` for an uncached work must still never touch upstream (`I3`).

### 2.5.6 UI (`src/render/solver.rs`, `src/render/chrome.rs`)

- [ ] **2.5.6.1 Challenge card** (wherever a user-initiated read returns `403`/`503`): AO3 `.flash.caution` explaining the edge refusal, a solver status chip (`ready` / `unreachable` / `disabled`), the `Solve & cache this page` form (§2.5.5.1), the cookie-sharing checkbox with an explicit warning, and the pre-existing links to `/session#cookie-import`, `/diagnostics`, and AO3 in the browser. No promise of success.
- [ ] **2.5.6.2** Extend the `/session` troubleshooting callout (§4.3.2) with the same solve action next to "verify in a browser and re-import cookies".
- [ ] **2.5.6.3 `/diagnostics` Solver panel:** mode, endpoint (loopback origin only), reachability + version + last probe, clearance active (age/expiry only, never the token), hourly budget used, last 20 `solver_event` rows, and `Probe now` / `Forget clearance` buttons.
- [ ] **2.5.6.4 `PageContext` gains `solver: &'a SolverSummary`** (mode, reachable, clearance_active) so header, footer, diagnostics and session render one consistent state; the footer gains a solver status line. Zero JavaScript; `<details>`/forms only, like the rest of the plan.

### 2.5.7 Budgets, safety & observability

- [ ] **2.5.7.1 Budgets:** `5 min` cooldown between solves and at most `6` successful solves per rolling hour (`solves_since`), checked *before* contacting the solver. Exhausted → `429` with wait-until and zero solver traffic.
- [ ] **2.5.7.2 Kill switch:** `Killed`/`AccountSession::kill()` blocks `/solve` with the same semantics as every other network method; `AO3I_FLARESOLVERR=off` is the config-level equivalent.
- [ ] **2.5.7.3 Diagnostics:** solver failures go to `http_error_log` with `transport='solver'` and `error_kind` in `solver_unreachable | solver_timeout | solver_captcha | solver_error | solver_body_cap`; successes go only to `solver_event` (`outcome='solved'`). The single-line stderr format gains `transport=solver`; never cookies, UA, HTML, or solver request bodies. `cf_ray`/`server` are stored when the browser exposes them, omitted otherwise.
- [ ] **2.5.7.4 No hammering:** one solve per explicit click, no retry, no background loop that touches AO3 (the probe talks only to the local solver), and no solver use for `/downloads/`, search paging, or any batch operation (ADR-0002/0004 unchanged).

### 2.5.8 Tests (`tests/flaresolverr.rs`, using the in-house mock in `tests/support`)

- [ ] **2.5.8.1 Happy path:** mock AO3 returns `403` + Cloudflare markers; mock solver `/v1` returns a valid work page + `cf_clearance`; `POST /solve` → `303` → `GET /read/<id>` serves sanitized prose from SQLite; `solver_event` has `outcome=solved`; no cookie value appears in stderr, DB, HTML, or audit.
- [ ] **2.5.8.2 Absent solver:** `AO3I_FLARESOLVERR_URL=http://127.0.0.1:1` in `auto` → the challenge card renders guidance, requests finish without hangs, app stays healthy; `on` behaves the same but flags unreachable in `/diagnostics`; `off` → no probe at all and `/solve` = `403`.
- [ ] **2.5.8.3 Typed failures:** captcha message → `.flash.error` + `solver_captcha`; malformed JSON → `solver_error` (no panic); oversized `solution.response` → `solver_body_cap` with nothing cached; solver HTTP 500 → `solver_error`.
- [ ] **2.5.8.4 Cookie sharing:** unchecked → solver request carries no `cookies`; checked (imported jar, `SEND_COOKIES=true`) → cookies present, yet still absent from all logs/DB/rendered pages.
- [ ] **2.5.8.5 Budget & single-flight:** a second concurrent `POST /solve` → `409`; the seventh solve in the hour → `429` and the mock solver's request count does not move; a preceding `Retry-After` suppresses the action.
- [ ] **2.5.8.6 Read-only guard:** `POST /write/kudos`, `/write/comment`, `/write/bookmark` with a configured solver → zero `/v1` requests.
- [ ] **2.5.8.7 Offline invariant:** the `tests/ao3_frontend.rs` route walk with a configured-but-idle solver and unreachable upstream performs zero `/v1` calls.

### 2.5.9 Build-vs-runtime gate (`scripts/`, `flake.nix` untouched)

- [ ] **2.5.9.1** Add `scripts/check-no-solver-build-dep.sh`: fails if `flaresolverr` is referenced from `Cargo.toml`, `Cargo.lock`, `flake.nix`, or `flake.lock` (comment allowlist explicit in the script), and fails if `nix path-info -r .#default` yields a store path containing `flaresolverr`.
- [ ] **2.5.9.2** Keep `flake.nix` unchanged: no `flaresolverr` in `nativeDeps`, `buildInputs`, `nativeBuildInputs`, `devShells.default`, or any wrapper. Provisioning lives in `docs/operator.md` and is squarely the host's job — `pkgs.flaresolverr` plus the `services.flaresolverr` NixOS module (`nixos/modules/services/misc/flaresolverr.nix`: `enable = true;`, default port `8191`, firewall closed), or the loopback-published container. It is a system package/service, **not** something this project builds or ships.
- [ ] **2.5.9.3** Prove the absence case: `nix build .#default` on a host without FlareSolverr, then run the binary with the service stopped and walk the offline routes (Phase 5.2 suite).

---

## Phase 3: 1:1 AO3 Design System & Global Site Chrome

> **Goal:** Establish an original, faithful AO3 visual design system (palette, typography, layout grid, icons, buttons, blurbs, flash banners, and optional Reversi dark skin) shared across every page.

- [ ] **3.1 Author AO3-faithful stylesheet (`src/render/assets/ao3.css`)**
  - [ ] **Color tokens & skins:**
    - Default AO3 Light theme: `--ao3-red: #900`, `--ao3-red-dark: #700`, `--ao3-bg: #fff`, `--ao3-surface: #eee`, `--ao3-border: #ddd`, `--ao3-border-strong: #ccc`, `--ao3-text: #2a2a2a`, `--ao3-muted: #555`, `--ao3-link: #111`, `--ao3-link-visited: #666`, `--ao3-notice-bg: #d1e1ef`, `--ao3-notice-border: #c2d2df`, `--ao3-error-bg: #efd1d1`, `--ao3-error-border: #900`, `--ao3-caution-bg: #ffe34e`.
    - Optional **Reversi** dark skin (`[data-skin="reversi"]` or `prefers-color-scheme: dark` override): `#333` background, `#222` boxes, `#eee` text, `#d9534f` / `#900` crimson accents, `#444` borders.
  - [ ] **Typography:**
    - UI/body: `'Lucida Grande', 'Lucida Sans Unicode', Verdana, Helvetica, sans-serif` at `100%` / `0.875rem` base.
    - Headings (`h1`, `h2`, `h3`, `.heading`): `Georgia, serif`.
  - [ ] **Core AO3 component styles:**
    - `#header`: top white/surface strip with crimson wordmark (`h1.heading a`), beta/local-cache tag, `#greeting` user area + `#login` dropdown panel, and full-width crimson primary nav bar (`ul.primary.navigation`) with right-aligned `#search` quick-search input.
    - `#inner`: centered container with optional left `#dashboard` sidebar (`13em`) or right `#work-filters` sidebar (`16em`) + `#main` content region.
    - `.actions`: beveled pill-like AO3 action buttons (`background: linear-gradient(#fff, #ddd)`, `border: 1px solid #bbb`, `border-radius: 0.25em`, crimson hover state).
    - `.flash.notice`, `.flash.error`, `.flash.caution`: full-width AO3 notification banners.
    - `.work.blurb.group`: bordered work card with `div.header.module` (title, byline, fandoms, datetime, 2×2 `ul.required-tags` icon grid), `ul.tags.commas` (`li.warnings` bold, `li.relationships` shaded `#eee` tag highlight, `li.characters`, `li.freeforms`), `blockquote.userstuff.summary`, and right-aligned `dl.stats`.
    - `dl.work.meta.group`: boxed AO3 work metadata definition list (`dt` left column, `dd` right column, horizontal `dl.stats` row).
    - `#workskin`: centered preface (`h2.title.heading`, `h3.byline.heading`, `div.summary.module`, `div.notes.module`), chapter header, prose container, and bottom `#feedback` / `#add_comment`.
    - `#footer`: crimson `#900` multi-column AO3 footer (`ul.navigation.actions` columns: About, Operator, Development, Status).
    - Responsive media queries (`@media (max-width: 62em)` / `42em`) matching AO3's mobile collapse behavior.
- [ ] **3.2 Serve static stylesheet & 2×2 required-tag SVG icons**
  - [ ] Add `GET /assets/ao3.css` route serving `include_str!("render/assets/ao3.css")` with `Content-Type: text/css; charset=utf-8`, `Cache-Control: public, max-age=3600`, and `X-Content-Type-Options: nosniff`.
  - [ ] Author original inline SVG icon helpers in `src/render/icons.rs` for AO3's 2×2 `required-tags` square:
    - **Rating:** General Audiences (G, green), Teen And Up (T, yellow), Mature (M, orange), Explicit (E, crimson), Not Rated (blank/muted).
    - **Warnings:** Archive Warnings Apply (`!`, crimson), Choose Not To Use (`?`, orange), No Warnings Apply (muted slash), External/Unknown.
    - **Category:** F/M, M/M, F/F, Gen, Multi, Other (color-coded quadrant badges).
    - **Status:** Complete (green checkmark), Work in Progress (blue/orange slash), Offline Cached indicator.
  - [ ] Add skin preference toggle (`POST /skin` setting a local `ao3i_skin=default|reversi` cookie with `HttpOnly; SameSite=Lax; Path=/`) so users can switch between Classic AO3 and Reversi dark mode.
- [ ] **3.3 Implement Global Chrome (`src/render/chrome.rs`)**
  - [ ] Define `PageContext<'a>` passed to every page renderer:
    - `title: &'a str`
    - `active_nav: NavSection` (`Home`, `Fandoms`, `Browse`, `Search`, `Dashboard`, `Session`, `Audit`, `Diagnostics`)
    - `account: &'a str`
    - `session_state: &'a str`
    - `write_enabled: bool`
    - `skin: &'a str`
    - `cached_count: i64`
    - `solver: &'a SolverSummary` (mode, reachable, clearance_active — §2.5.6.4; safe fields only)
  - [ ] Build `#header`:
    - Brand: `<h1 class="heading"><a href="/"><span>AO3Instance</span><sup>local</sup></a></h1>`
    - `#greeting`:
      - If signed in (`!account.is_empty()`): `"Hi, <account>!"` dropdown (`<details class="dropdown">`, pure HTML/CSS — works without JS!) linking to Dashboard, Subscriptions, Bookmarks, Reading List, Session, and Log Out.
      - If signed out: `"Log In"` dropdown (`<details id="login" class="dropdown">`) containing an inline AO3-style login form posting to `/session/login` plus a direct link to `/session#cookie-import`.
    - Primary crimson bar (`ul.primary.navigation`):
      - `Fandoms` (`/fandoms`)
      - `Browse` (`/works`)
      - `Search` (`/works/search`)
      - `Open Work` (`/#open-work`)
      - `Dashboard` (`/dashboard`)
      - `Audit` (`/audit`)
      - `Diagnostics` (`/diagnostics`)
      - Right-aligned `#search` quick-search form (`GET /search?q=...`).
  - [ ] Build `#footer` with 4 AO3-style columns (Customize/Skin toggle, About AO3Instance, Operator & Cache Status, Diagnostics, Audit & Solver Status — mode/reachability/clearance age only, never the clearance value).

---

## Phase 4: 1:1 AO3 Frontend — All Segments & Routes

> **Goal:** Implement every user-facing AO3 segment with 1:1 visual layout and ergonomics while backing all reads strictly from SQLite and keeping every upstream read/write strictly user-initiated and gated.

### 4.1 Store Enrichment for Rich Blurbs, Browse & Dashboard (`src/store.rs`)

- [ ] **4.1.1 Enrich `WorkSummary` (or add `WorkBlurb`) in `src/store.rs`**
  - [ ] Return all fields needed for AO3 `.work.blurb.group` cards: `ao3_id`, `title`, `authors`, `summary`, `tags: TagGroups`, `rating`, `language`, `word_count`, `kudos`, `comment_count`, `chapter_count`, `fetched_at`, `progress_chapter`, plus optional `reading_state`, `bookmarked`, `subscribed`.
  - [ ] Keep existing `list_works` and `search_works` signatures compatible (or update all call sites cleanly).
- [ ] **4.1.2 Add offline Browse / Filter / Fandom / Tag / Dashboard queries in `src/store.rs`**
  - [ ] `Store::browse_works(&self, filter: &WorkFilter) -> Result<BrowseResult>`:
    - Filters: `rating`, `warning`, `category`, `fandom`, `tag`, `language`, `reading_status` (`all`, `in_progress`, `for_later`, `read`, `bookmarked`), `sort` (`fetched_desc`, `words_desc`, `kudos_desc`, `title_asc`), `page`, `per_page` (default 20).
    - Also computes facet counts for the right-hand filter sidebar.
  - [ ] `Store::list_fandoms(&self) -> Result<Vec<(String, usize)>>` (grouped from cached works' `tags_json`).
  - [ ] `Store::list_subscriptions(&self, account: &str) -> Result<Vec<SubscriptionEntry>>` (joined with cached work titles when present).
  - [ ] `Store::list_bookmarks(&self, account: &str) -> Result<Vec<BookmarkEntry>>` (joined with cached works).
  - [ ] `Store::list_reading_history(&self, account: &str) -> Result<Vec<ReadingEntry>>` (combining `progress` and `reading_flags` with cached works).
  - [ ] `Store::cache_stats(&self) -> Result<CacheStats>` (total works, total chapters, total words, total subscriptions/bookmarks/kudos, recent HTTP error count).
  - [ ] Add unit tests in `src/store.rs` covering every new query and filter edge case.

### 4.2 Segment 1 — Home / Splash Page (`/`)

- [ ] **4.2.1 Implement AO3-style Splash Home (`src/render/home.rs`)**
  - [ ] Top `.splash` hero row:
    - Left: AO3-style welcome / offline cache summary ("A local archive of your own" — cached work count, offline readiness badge, write-mode status pill).
    - Right: Prominent **Open / Cache a Work** box (`#open-work`, `POST /open`) accepting an AO3 work ID or `/works/<id>` URL.
  - [ ] Flash notification bar (`.flash.notice`) for post-write / redirect notices.
  - [ ] **Cached Fandoms** module (`.favorite.module` — top cached fandoms with work counts linking to `/works?fandom=...`, plus "All Cached Fandoms →" link to `/fandoms`).
  - [ ] **Continue Reading** strip (when works have saved `progress_chapter`, quick resume cards).
  - [ ] **Recently Cached Works** list rendered with full AO3 `.work.blurb.group` cards (`src/render/blurb.rs`).

### 4.3 Segment 2 — Login, Logout & Session Management (`/users/login` & `/session`)

- [ ] **4.3.1 Add route alias `GET /users/login` → AO3-style login/session view**
  - [ ] Support both `/users/login` and `/session` (keeping `/session` as canonical so all existing tests/links work seamlessly).
- [ ] **4.3.2 Redesign `src/render/session.rs` as 1:1 AO3 Login & Session**
  - [ ] **Status Header & Flash Slots:** `.flash.notice` / `.flash.error`, AO3-style session status card (`state`, canonical username, jar completeness, adult consent status, active pseud).
  - [ ] **Quick Actions Bar (`ul.actions`):** Verify Session Now (`POST /session/probe`), Enable Adult Content (`POST /session/adult`), Log Out (`GET /users/logout/confirm` → `POST /session/logout` matching AO3's `confirm_logout` pattern while preserving direct `POST /session/logout` compatibility).
  - [ ] **Tab/Fieldset 1 — Password Login (`#login-form`):** 1:1 AO3 `users/sessions/new` form layout (`User name or email`, `Password`, `Remember me / Allow adult content`, `Log In` button) with clear note per `ADR-0006` that passwords are never persisted at rest.
  - [ ] **Tab/Fieldset 2 — Cookie Import (`#cookie-import`, Recommended):** Step-by-step browser instructions (`_otwarchive_session` + `user_credentials` from DevTools → Application → Cookies), AES-256-GCM encryption note, adult opt-in checkbox, and `Import Session` button.
  - [ ] **Troubleshooting / Access-Error Callout:** When `session_state == "blocked"` or after a `403`/`503`, render a clear `.flash.caution` explaining that AO3's edge refused the request, linking to `/diagnostics`, and guiding the user to verify/sign in via their browser and re-import fresh session cookies. When a solver is configured and reachable (§2.5), the same callout offers the explicit `Solve & cache` form with the cookie-sharing checkbox; it never retries on its own.

### 4.4 Segment 3 — Work & Reader Page (`/works/{id}` & `/read/{id}`)

- [ ] **4.4.1 Add AO3 URL aliases in `src/server/reads.rs`**
  - [ ] `GET /works/{id}` → renders the same reader as `GET /read/{id}` (if cached; if not cached, shows a clean AO3-styled "Not cached yet" card with a one-click `POST /open` button to fetch & cache it — preserving `I3` offline invariant so `GET` never blocks on upstream!).
  - [ ] `GET /works/{id}/chapters/{idx}` → redirects/serves `/read/{id}?chapter={idx}`.
  - [ ] Support `?view_full_work=true` query mode locally (renders all cached chapters stacked sequentially, each in its own sandboxed iframe or combined sanitized chapter document with strict `CHAPTER_CSP`).
- [ ] **4.4.2 Rebuild Work / Reader Template (`src/render/work.rs`) 1:1 to AO3**
  - [ ] **Top Action Bar (`ul.work.navigation.actions`):**
    - `Entire Work` / `Chapter by Chapter` toggle
    - `← Previous Chapter` / `Next Chapter →`
    - `Chapter Index` inline `<form method="get">` with `<select name="chapter">` dropdown + `Go` button (zero-JS chapter jump!)
    - `Bookmark` / `Bookmarked`
    - `Mark for Later` / `Mark as Read`
    - `Subscribe` / `Unsubscribe`
    - `Save Place` (progress anchor form)
    - `Refresh from AO3` (user-initiated `POST`; a `403`/`503` renders the §2.5.6 challenge card — no automatic solver retry)
    - `Open on AO3 ↗`
  - [ ] **Work Metadata Box (`dl.work.meta.group`):**
    - Exact AO3 row order: `Rating:`, `Archive Warning:`, `Category:`, `Fandom:`, `Relationship:`, `Character:`, `Additional Tags:`, `Language:`, `Stats:` (`Checked:`, `Words:`, `Chapters:`, `Comments:`, `Kudos:`).
    - Every tag renders as `<a class="tag" href="/works?tag=...">` linking into local offline filtering.
    - Stale cache badge (`> 7 days`) styled cleanly next to `Checked:`.
  - [ ] **Workskin Preface (`#workskin > div.preface.group`):**
    - Centered `h2.title.heading`, `h3.byline.heading` (author links filter cached works by author), `div.summary.module` (`<h3 class="heading">Summary:</h3><blockquote class="userstuff">...</blockquote>`), resume banner (`.flash.notice`) if `progress != current`.
  - [ ] **Chapter Container (`#chapters`) & Sandboxed Iframe:**
    - Render AO3-style `div.chapter.preface.group` (`h3.title`: `Chapter N: Title`) above the sandboxed `<iframe class="chapter" sandbox referrerpolicy="no-referrer" ...>`.
    - Update `chapter_document()` CSS inside the iframe to match AO3's `#workskin div.userstuff` typography (`Georgia, serif`, `1.125em` / `1.65` line-height, paragraph spacing, `<hr>`, `<blockquote>`, `notes` block) and accept `?skin=reversi` so dark mode matches seamlessly **without relaxing `CHAPTER_CSP` or adding any scripts**.
  - [ ] **Bottom Feedback & Comment Box (`#feedback`):**
    - `ul.actions`: `↑ Top`, `← Previous Chapter`, `Next Chapter →`, `Kudos ♥` (routes through `/write/preview?kind=kudos&id=...`, or shows `kudos given` receipt badge), `Bookmark`, `Mark for Later` / `Mark as Read`.
    - `#add_comment`: AO3-style comment fieldset ("Comment as `<pseud>`", 10,000-char limit note, textarea, `Preview Comment` submit button posting to `/write/comment/preview`, or "Sign in & enable writes to comment" notice when disabled).

### 4.5 Segment 4 — Browse Works, Fandoms & Tags (`/works`, `/fandoms`, `/tags/{tag}/works`)

- [ ] **4.5.1 Implement Browse & Filter Page (`GET /works`, `GET /tags/{tag}/works`)**
  - [ ] Render `src/render/browse.rs` with AO3's classic two-column layout:
    - Main column: heading (`N Cached Works [in Tag/Fandom]`), active filter chips with clear links, pagination (`ol.pagination.actions`), and `.work.blurb.group` list.
    - Right sidebar (`form#work-filters.filters`): `Sort by`, `Rating`, `Archive Warnings`, `Categories`, `Fandoms`, `Language`, `Reading Status`, and crimson `Sort and Filter` submit button (100% server-rendered `GET` form).
- [ ] **4.5.2 Implement Fandoms Directory (`GET /fandoms`)**
  - [ ] Alphabetical index jump bar (`A–Z`, `#`) and grouped fandom list with cached work counts linking to `/works?fandom=...`.

### 4.6 Segment 5 — Work Search (`/search` & `/works/search`)

- [ ] **4.6.1 Upgrade Search Page (`src/render/search.rs`)**
  - [ ] Alias `GET /works/search` to the search handler alongside `GET /search`.
  - [ ] Render AO3-style Work Search fieldset: `Any Field` (FTS5 query), plus optional filter selectors (`Rating`, `Fandom`, `Language`, `Sort by`) and clear provenance banner (`Local Cache — Offline Search`).
  - [ ] Render hits using the shared `.work.blurb.group` component (`src/render/blurb.rs`) instead of plain list items.
  - [ ] Keep distinct empty states: blank query prompt vs "0 cached matches for `<query>`" with quick link to open/cache a work by ID.

### 4.7 Segment 6 — User Dashboard, Lists, Confirmations, Audit & Diagnostics

- [ ] **4.7.1 Implement AO3 Left-Sidebar Dashboard Layout (`src/render/dashboard.rs`)**
  - [ ] Shared `#dashboard` left navigation column:
    - `Dashboard` (`/dashboard`) — account/session overview, cache statistics, quick links, recent progress.
    - `Cached Library` (`/works`)
    - `Subscriptions` (`/subscriptions`) — lists active/removed local subscription ledger rows with Unsubscribe action.
    - `Bookmarks` (`/bookmarks`) — lists local bookmarked works as AO3 blurbs.
    - `Marked for Later & History` (`/reading`) — lists works with saved reading progress or `for_later`/`read` flags.
    - `Session & Preferences` (`/session`)
    - `Write Audit Log` (`/audit`) — styled as an AO3 `.index` table with outcome badges (`done`, `failed`, `already-done`).
    - `HTTP Diagnostics` (`/diagnostics`) — filterable table (`All` | `Upstream` | `Inbound`) of recent `http_error_log` entries showing timestamp, direction, transport, method, sanitized path, HTTP status badge, error kind, latency (`ms`), `Retry-After`, `Server` / `CF-Ray`, and diagnostic message, plus an explicit `Clear Logs` (`POST /diagnostics/clear`) button and the Solver panel from §2.5.6.3.
- [ ] **4.7.2 Restyle Confirmation & Error Pages (`src/render/audit.rs`)**
  - [ ] Style `/write/preview` and `/write/comment/preview` as AO3 `.caution` confirmation boxes with clear target naming, exact comment preview, and `Confirm` / `Cancel` `.actions` buttons.
  - [ ] Style `error_page` (`400`, `403`, `404`, `502`, `500`) as AO3 `system.errors` pages with crimson error banner, actionable next steps (e.g., `/session`, `/diagnostics`, `/`), and zero secret/body leakage.

---

## Phase 5: Documentation, Gates & Verification

- [ ] **5.1 Update Project Documentation (`G7` docs parity)**
  - [ ] Update `README.md` with the 1:1 AO3 frontend segments, routes, Reversi dark skin, HTTP error diagnostic logging, and the optional host-provided FlareSolverr fallback — stating plainly that FlareSolverr is not a build dependency, is never spawned by the app, and that every feature works without it.
  - [ ] Update `docs/operator.md` with new env vars (`AO3I_LOG_HTTP_ERRORS`, `AO3I_HTTP_LOG_RETENTION`, `AO3I_FLARESOLVERR`, `AO3I_FLARESOLVERR_URL`, `AO3I_FLARESOLVERR_SEND_COOKIES`, `AO3I_FLARESOLVERR_MAX_TIMEOUT_MS`, `AO3I_FLARESOLVERR_ALLOW_REMOTE`), `/diagnostics` usage, stderr log format, and the solver provisioning section (§2.5.9: `services.flaresolverr.enable = true;`, port `8191`, firewall closed, container alternative, "never expose 8191").
  - [ ] Add `docs/adr/0007-flaresolverr-fallback.md` (amends ADR-0002 §5 and the "challenge is terminal" consequence; records the Cloudflare/ToS risk, the read-only + budget limits, and the rollback `AO3I_FLARESOLVERR=off`).
  - [ ] Update `docs/failure-matrix.md` to note HTTP error diagnostic logging and `/diagnostics` visibility for each failure row, plus new solver rows (`solver_unreachable`, `solver_captcha`, `solver_body_cap`, `solver_budget`) with their user-visible messages and no-secret assertions.
- [ ] **5.2 Automated Test Suite**
  - [ ] Add `tests/http_diagnostics.rs`:
    - Trigger upstream `403`, `429` (with `Retry-After` & `cf-ray`), `503`, `302 /users/login`, and connection-refused errors against mock `Server`.
    - Assert `http_error_log` captures each event with accurate status, `error_kind`, `transport`, `latency_ms`, `retry_after`, `cf_ray`, and sanitized path.
    - Assert `_otwarchive_session`, `user_credentials`, `authenticity_token`, and response HTML bodies **never** appear in `http_error_log` rows or formatted log strings.
    - Assert `GET /diagnostics` renders recorded entries and `POST /diagnostics/clear` empties them.
  - [ ] Add `tests/flaresolverr.rs` — the §2.5.8 matrix (happy path, absent solver, typed failures, cookie-sharing redaction, budget/single-flight, read-only guard).
  - [ ] Add `tests/ao3_frontend.rs`:
    - Walk every segment offline (`/` , `/works`, `/fandoms`, `/search`, `/works/search`, `/users/login`, `/session`, `/read/{id}`, `/works/{id}`, `/dashboard`, `/subscriptions`, `/bookmarks`, `/reading`, `/audit`, `/diagnostics`, `/assets/ao3.css`) with unreachable upstream (`127.0.0.1:1`) — assert `200 OK`, zero hangs, zero solver `/v1` calls, and key AO3 landmarks (`#header`, `#footer`, `.primary.navigation`, `.work.blurb`, `dl.work.meta`, `CHAPTER_CSP` on chapter iframe).
    - Verify uncached `GET /works/999999` returns offline "not cached" page without touching upstream.
    - Verify `POST /skin` toggles `reversi` and persists across pages.
    - Verify all existing integration suites (`tests/failure_matrix.rs`, `tests/session_and_progress.rs`, `tests/search_and_refresh.rs`, `tests/writes*.rs`) remain green without regressions.
- [ ] **5.3 Final Quality & Security Gates**
  - [ ] `nix develop -c cargo fmt --check`
  - [ ] `nix develop -c cargo clippy --all-targets -- -D warnings`
  - [ ] `nix develop -c cargo test`
  - [ ] `nix build .#default` (must succeed on a host with FlareSolverr absent, and with it stopped at runtime)
  - [ ] `scripts/check-ao3-routes.sh`
  - [ ] `scripts/check-no-solver-build-dep.sh` (§2.5.9) — `Cargo.toml`, `Cargo.lock`, `flake.nix`, `flake.lock`, and the `nix build` closure stay solver-free
  - [ ] Modularity check (`wc -l src/**/*.rs` — confirm every file ≤ 800 LOC).
  - [ ] Manual browser walkthrough: verify visual 1:1 parity across Light & Reversi themes on desktop and mobile viewports (splash, login dropdown, `/users/login`, `/works` with filter sidebar, `/read/{id}` with chapter iframe & comment box, 403 challenge card with solver ready/absent, `/dashboard`, `/audit`, `/diagnostics` including the Solver panel).
