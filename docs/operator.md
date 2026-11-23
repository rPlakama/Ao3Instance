# Operator guide

AO3Instance is a self-hosted, single-operator reader/writer for archiveofourown.org.
This page is for whoever runs it.

## Requirements

- Nix (flakes enabled). There is no other supported build path.
- A writable data directory (defaults to `$XDG_CONFIG_HOME/ao3instance`).
- Outbound HTTPS to archiveofourown.org (and to the sandbox if `AO3I_BASE_URL` is set).
- *Optional:* a host-provided FlareSolverr service for edge challenges. It is
  **not** a build dependency: the app works, builds and tests with it absent.

## Run

```sh
nix develop -c cargo run --bin ao3i-server
# or: nix run .        (package build)
# AO3Instance ready at http://127.0.0.1:8099/
```

Bind loopback by default. If you expose it, put it behind a TLS reverse proxy and
add authentication in front: the app has **no user model beyond the AO3 session**
and must not be exposed to the open internet as-is.

## Configuration

| env | default | meaning |
| --- | --- | --- |
| `AO3I_BIND` | `127.0.0.1` | listen address |
| `AO3I_PORT` | `8099` | listen port |
| `AO3I_DATA_DIR` | `$XDG_CONFIG_HOME/ao3instance` | data directory |
| `AO3I_DB_FILE` | `<data_dir>/ao3i.db` | SQLite cache |
| `AO3I_KEY_FILE` | `<data_dir>/key` | AES-256-GCM key (0600) |
| `AO3I_BASE_URL` | `https://archiveofourown.org` | upstream origin (sandbox override) |
| `AO3I_USER_AGENT` | built-in | **set a project URL + contact before public use** |
| `AO3I_WRITE_ENABLED` | `false` | master switch for all POST/PATCH/DELETE |
| `AO3I_MIN_SPACING_MS` | `2000` | floor between upstream requests |
| `AO3I_LOG_HTTP_ERRORS` | `true` | one structured HTTP-error line on stderr |
| `AO3I_HTTP_LOG_RETENTION` | `500` | rows kept in `http_error_log` |
| `AO3I_FLARESOLVERR` | `auto` | `auto` \| `on` \| `off` for the optional fallback |
| `AO3I_FLARESOLVERR_URL` | `http://127.0.0.1:8191` | solver origin; loopback-only unless `ALLOW_REMOTE` |
| `AO3I_FLARESOLVERR_ALLOW_REMOTE` | `false` | permit a non-loopback solver host (container/bridge) |
| `AO3I_FLARESOLVERR_SEND_COOKIES` | `false` | allow the AO3 jar to reach the solver (still per-solve opt-in) |
| `AO3I_FLARESOLVERR_MAX_TIMEOUT_MS` | `60000` | challenge-solving budget, `5000..=180000` |

`AO3I_KEY_FILE` and `AO3I_DATA_DIR` cannot be set from a `.env` for security
reasons; the app reads environment variables only.

## Optional: Cloudflare fallback (host-provided FlareSolverr)

AO3's edge sometimes answers a read with `403`/`503`. With this service running,
the error page offers an explicit **Solve & cache this page** button. The app
never starts, installs or requires FlareSolverr; it only calls the local `/v1`
API.

On NixOS, one line makes it available to the system:

```nix
services.flaresolverr.enable = true;   # default port 8191, firewall stays closed
```

(or run `ghcr.io/flaresolverr/flaresolverr` published to `127.0.0.1:8191`).
**Never expose port 8191 to a network**: the API can drive a browser anywhere
its host can reach. A container/bridge address needs
`AO3I_FLARESOLVERR_ALLOW_REMOTE=true`; the default only accepts loopback.

Limits and behaviour:

- One solve at a time, 5 min cooldown, 6 successful solves per rolling hour.
- Reads only. Writes never go through the solver.
- The AO3 session is sent to the browser only when both
  `AO3I_FLARESOLVERR_SEND_COOKIES=true` and the user ticks the per-solve box.
- The returned clearance is encrypted at rest and dropped on the next `403`;
  it is never displayed. `/diagnostics` shows reachability, version, clearance
  age and recent outcomes (never the cookie).
- Set `AO3I_FLARESOLVERR=off` to disable the whole path; `auto` stays quiet when
  the service is absent, `on` also warns about it in `/diagnostics`.

## Pages

`/` AO3-shaped home (favorites, cached news, continue reading, open-work box) ·
`/works` cached library with filters ·
`/fandoms` A–Z directory · `/dashboard` statistics and shortcuts ·
`/subscriptions`, `/bookmarks`, `/reading` local ledgers ·
`/read/<id>` reader (chapter jump, Entire Work) · `/session` AO3 login/import ·
`/works/search` AO3 Work Search: one live AO3 request per submit, results shown AO3-style (not cached); `?local=1` searches the cache · `/tags/search` cached tag search · `/tags/<tag>/works` AO3-shaped tag page · `/audit` write ledger ·
`/diagnostics` HTTP failures, solver state and clear · `/about` instance notes.
`/users/login`, `/works/search` and `/works/{id}` mirror AO3 URLs; uncached
works show an offline miss card instead of fetching on a `GET`.

Display skins: the footer switches Classic / Reversi with `POST /skin` (a
single-operator global preference); with no explicit choice, Reversi follows the
OS `prefers-color-scheme`.

## First use

1. Open `http://127.0.0.1:8099/`.
2. Open a public work by id or URL: it is fetched once, cached and readable offline.
3. For restricted or adult works and all writes: go to **Session**.
   - **Cookie import (recommended)**: sign in at archiveofourown.org, copy
     `_otwarchive_session` and `user_credentials` from DevTools → Application →
     Cookies, paste them, and import. Nothing else is stored but the encrypted jar.
   - **Password login** is available but the password is never persisted; after the
     AO3 session expires you sign in again.
4. Writes stay disabled until `AO3I_WRITE_ENABLED=true`. Every write shows a
   confirmation page and is recorded in **Audit**.

## Data on disk

| path | contents |
| --- | --- |
| `ao3i.db` | caches, chapters, progress, session ciphertext, write ledger, subscriptions |
| `key` | 32-byte AES key, mode 0600, created once |
| `key` sibling files | none |

Backup: copy the data directory while the server is stopped. **The database alone
cannot decrypt the session**; if you back up the database and the key together, treat
the backup as being as sensitive as the AO3 account itself. Losing the key means
re-importing the session; cached works and progress are unaffected.

## Writes: what to expect

- One action = one confirmation = one upstream request. No retries for comments,
  bookmarks or subscriptions; kudos duplicates are detected from AO3's own message.
- Client budgets: comments 3/15 min (AO3's own limit), kudos 10/min,
  subscriptions 20/h, bookmarks 10/min. `Retry-After` parks the action.
- The audit log (`/audit`) is append-only and stores targets and outcomes — never
  payloads, cookies or passwords.
- To stop all writes immediately: restart with `AO3I_WRITE_ENABLED=false` (default).
  The in-process kill switch is `AccountSession::kill` (used by tests); there is no
  UI button for it yet.

## Politeness

The client is a user agent, not a crawler (ADR-0002): user-initiated fetches only,
no background crawling, no `/downloads/` automation, honest UA, one write at a time.
If AO3 ever asks you to stop, disable writes and stop the process; the audit log
shows exactly what was done.

## Troubleshooting

| symptom | cause | action |
| --- | --- | --- |
| `AO3 is rate limiting this instance` (429) | too many requests from your IP | wait for `Retry-After`; the client parks the action |
| `AO3 refused the request (403/503)` | bot challenge / edge block | stop for a while; do not hammer; if intended, use the solve card or check `/diagnostics` |
| "local FlareSolverr is unavailable" | host service not running / wrong port | start `services.flaresolverr` or check `AO3I_FLARESOLVERR_URL` |
| "hit a captcha it cannot solve" | Cloudflare captcha | open the page in a real browser; import fresh cookies |
| 429 from `/solve` | local cooldown or hourly budget | wait; the message says how long |
| Adult-content gate message | no `view_adult` consent in the session | Session → Enable adult content (requires a session) |
| Restricted work message | no signed-in session | Session → import cookies |
| `Session is not usable; re-authenticate` | cookie expired or incomplete | re-import cookies |
| writes 403 | kill switch off | set `AO3I_WRITE_ENABLED=true` and restart |
| key permission error at startup | data dir/key mode changed | restore `0600` key and owner-only dir; never chmod shared dirs |
| `AO3 did not show a pseud to comment with` | pseud page drift or wrong account | verify `GET /users/<you>/pseuds` in the browser |

## Limitations (documented, not bugs)

- The FlareSolverr fallback is for reads only, one page per explicit click; it is
  not a crawler and cannot be enabled for writes.

- Chapter-level reading progress only; no scroll/block anchor yet (I7 refinement).
- Bookmarks and comments are created, not edited or deleted from the app.
- No OPDS, no batch import, no collection management.
- The local text is a cache for personal offline reading; do not redistribute it.
