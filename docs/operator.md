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
| `AO3I_FLARESOLVERR` | `auto` | `auto` \| `on` \| `off` for the host-provided solver |
| `AO3I_FLARESOLVERR_URL` | `http://127.0.0.1:8191` | solver origin; loopback-only unless `ALLOW_REMOTE` |
| `AO3I_FLARESOLVERR_ALLOW_REMOTE` | `false` | permit a non-loopback solver host (container/bridge) |
| `AO3I_FLARESOLVERR_SEND_COOKIES` | `true` | send the AO3 cookie jar to the local solver |
| `AO3I_FLARESOLVERR_MAX_TIMEOUT_MS` | `60000` | challenge-solving budget, `5000..=180000` |

`AO3I_KEY_FILE` and `AO3I_DATA_DIR` cannot be set from a `.env` for security
reasons; the app reads environment variables only.

## Host-provided FlareSolverr

Whenever a host-provided FlareSolverr is reachable (and `AO3I_FLARESOLVERR` is
not `off`), AO3Instance always routes **every** upstream interaction with AO3
(reads, search, tags, home, public skins, login, session verification, CSRF
fetches, and writes) through it and falls back to the direct HTTP client only
when FlareSolverr is absent or unreachable. The app never starts, installs or
requires FlareSolverr; it only calls the local `/v1` API.

On NixOS, one line makes it available to the system:

```nix
services.flaresolverr.enable = true;   # default port 8191, firewall stays closed
```

(or run `ghcr.io/flaresolverr/flaresolverr` published to `127.0.0.1:8191`).
**Never expose port 8191 to a network**: the API can drive a browser anywhere
its host can reach. A container/bridge address needs
`AO3I_FLARESOLVERR_ALLOW_REMOTE=true`; the default only accepts loopback.

Limits and behaviour:

- Serialized one solve at a time (no parallel Chromium launches).
- Handles both `GET` (`request.get`) and form writes (`request.post`, using
  Rails' `_method` override for `PATCH`/`DELETE`).
- Session cookies are forwarded to the local solver by default
  (`AO3I_FLARESOLVERR_SEND_COOKIES=true`) so authenticated pages, session
  checks, and writes work through it.
- The returned clearance is encrypted at rest and dropped on the next `403`;
  it is never displayed. `/diagnostics` shows reachability, version, clearance
  age and recent outcomes (never the cookie).
- Set `AO3I_FLARESOLVERR=off` to disable the whole path; `auto` stays quiet when
  the service is absent and re-probes periodically, `on` also warns about it in
  `/diagnostics`.

## Pages

`/` AO3-shaped home (favorites, cached news, continue reading, open-work box; 30-min cached) ·
`/works` cached library with filters ·
`/fandoms` live AO3 `/media` directory and `/media/<slug>/fandoms` A–Z lists (30-min cached, offline fallback) ·
`/dashboard` live AO3 subscriptions, bookmarks, reading history and local cache stats ·
`/subscriptions`, `/bookmarks`, `/reading` (`?show=to-read` for Marked for Later) live AO3 account lists (30-min cached, offline fallback) ·
`/my/works`, `/my/works/drafts` the signed-in author's works and drafts, read from AO3's
own `/users/<login>/works[/drafts]` (30-min cached, offline notice, `?page=N`,
`?refresh=1`; drafts warn that AO3 deletes unposted drafts after ~29 days; posted rows
offer **Edit Work** when writes are enabled) ·
`/my/works/<id>/chapters` one work's chapter manager: AO3's own chapter index from
`/works/<id>/navigate` (signed-in, 30-min cached, `?refresh=1`) with chapter titles and
dates, plus Edit/Add Chapter links when writes are enabled ·
`/write/chapter/new?work_id=<id>` and `/write/chapter/edit?work_id=<id>&chapter_id=<id>`
the chapter editor (signed in and writes enabled; blank for a new chapter with the next
number suggested; an edit is prefilled from AO3's chapter form) ·
`POST /write/chapter/preview` local validation + confirmation (no upstream write) ·
`POST /write/chapter` the one chapter create/update request ·
`/write/work/edit?work_id=<id>` a posted work's metadata/tag editor (signed in and writes
enabled; prefilled from AO3's own `/works/<id>/edit`; confirmation before the write; chapter
text is edited from the chapter manager, not here) ·
`POST /write/work/edit/preview` local validation + confirmation (no upstream write) ·
`POST /write/work/edit` the one metadata-update request ·
`GET /write/preview?kind=work-publish|work-delete&id=<id>` local confirmation ·
`POST /write/work/publish` one confirmed `PUT /works/<id>/post_draft` ·
`POST /write/work/delete` one confirmed `DELETE /works/<id>` ·
`/read/<id>` reader (chapter jump, Entire Work; `?show_comments=1` shows the cached comment
thread; the Comments action (when nothing is cached) or the "Refresh comments" button under
a cached thread POSTs to `/read/<id>/comments`, which fetches it once from AO3) ·
`/session` AO3 login/import (logout is local-only) ·
`/works/search` AO3 Work Search (30-min cached, `?refresh=1` to force refresh, `?local=1` searches cached works) ·
`/tags/search` live AO3 tag search (30-min cached, offline fallback) ·
`/tags/<tag>/works` live AO3 tag page (30-min cached, offline fallback) ·
`/open/<id>` one-click cache+read ·
`/works/new` new-work composer (signed in **and** writes enabled; local preview, then one
confirmed post; the header **Post** link points here) · `POST /write/work/preview` local
confirmation (no upstream write) · `POST /write/work` the one create request ·
`/audit` write ledger ·
`/diagnostics` HTTP failures, solver state and clear · `/about` instance notes.
`/users/login`, `/works/search` and `/works/{id}` mirror AO3 URLs; uncached
works show an offline miss card instead of fetching on a `GET`.

Appearance: `/session#appearance` and the footer offer Auto, Classic, Reversi,
Snow, Frost, Cloud, Sepia and Midnight via `POST /skin`; `/themes` can import a
public AO3 site skin from `archiveofourown.org/skins`. The choice and imported
CSS are stored in the local `settings` table and survive restarts; the default
is Classic. Imported skin CSS is trusted local styling: it can override layout,
so use `/themes/clear` to return to the built-in look.

## NixOS module

`nixosModules.default` defines `services.ao3instance.dataDir`,
`services.ao3instance.port`, `services.ao3instance.openFirewall`, and
`services.ao3instance.flaresolverrPort`:

```nix
services.ao3instance = {
  enable = true;
  dataDir = "/var/lib/ao3instance";
  port = 8099;
  flaresolverrPort = 8191; # optional; sets AO3I_FLARESOLVERR_URL=http://127.0.0.1:<port>
};
```

It creates `/var/lib/ao3instance` (0700, owned by the `ao3instance` system
user), runs the package binary with `AO3I_DATA_DIR`/`AO3I_PORT`, and restarts on
failure. With `openFirewall = true` the module also opens the configured port
and sets `AO3I_BIND=0.0.0.0`; by default it stays loopback-only. The app holds
an AO3 session, so only expose it on trusted networks or behind a reverse
proxy. Back up the whole data directory (SQLite database plus AES key).
The flake check `nixos-module` boots a VM, waits for the port, checks `/health`,
and verifies that `ao3i.db` and `key` are created.

## First use

1. Open `http://127.0.0.1:8099/`.
2. Open a public work by id or URL: it is fetched once, cached and readable offline.
3. For restricted or adult works and all writes: go to **Session**.
   - **Cookie import (recommended)**: sign in at archiveofourown.org, copy
     `_otwarchive_session` and `user_credentials` from DevTools → Application →
     Cookies, paste them, and import. Nothing else is stored but the encrypted jar.
   - **Password login** is available but the password is never persisted; after the
     AO3 session expires you sign in again.
4. Writes stay disabled until `AO3I_WRITE_ENABLED=true`. Kudos and work
   subscriptions post straight from the reader; comments and bookmarks show a
   confirmation page first. The signed-in menu and the dashboard sidebar gain
   Writing entries (My Works, My Drafts, and — with writes enabled — Post a new
   work); My Works rows link to the chapter manager (**Chapters**, and
   **Add Chapter** for a work still in progress once writes are enabled) and, once
   writes are enabled, to a posted work's metadata editor (**Edit Work**). Every
   write is recorded in **Audit**.

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

- The reader only offers write forms in one state: signed in **and** writes enabled
  (`AO3I_WRITE_ENABLED=true`). When they are off it says "Writes are disabled"
  instead of pretending you are signed out; when the saved cookie jar can no
  longer authenticate it says so and links to the cookie import. Both states are
  also shown on `/session`, together with the current write switch.
- Enabling adult content from `/session` requires a working session. The toggle
  no longer writes an empty/wiped jar: an unusable stored session is cleared at
  startup (logged as `cannot authenticate ...; cleared, re-import required`) and
  you re-import cookies instead. Copy **both** `_otwarchive_session` and
  `user_credentials`; a jar missing either is rejected with an explicit message.

- One action = one confirmation = one upstream request. Comments and bookmarks
  confirm on a page that names the target; reader kudos and work
  subscribe/unsubscribe post directly the way AO3's own buttons do — the click
  is the deliberate action, and the kudos and subscribe forms carry the hidden
  `confirm=yes` token (the same-origin guard applies to every mutating route).
  No retries for comments, bookmarks or subscriptions; kudos duplicates are
  detected from AO3's own message.
- Posting a new work (`/works/new`, also the header **Post** link) composes AO3's
  own form, validates against AO3's limits, shows a confirmation page naming the
  work and pseud, then sends exactly one `POST /works`: `post_button` publishes
  it, `save_button` saves it as an AO3 draft. It is never retried — a timeout is
  recorded as ambiguous and the composer tells you to check your works list on
  AO3 before trying again.
- Adding and editing chapters: the chapter manager at
  `/my/works/<id>/chapters` lists AO3's own chapter index for one of your works
  (30-minute cached; `?refresh=1` forces a fetch) with chapter titles, dates and
  Edit links, and the editor at `/write/chapter/new|edit` mirrors AO3's chapter
  form (a new chapter suggests the next number; an edit is prefilled from AO3).
  The editor validates against AO3's chapter limits and shows a confirmation
  page naming the chapter; `/write/chapter` then sends exactly one
  `POST /works/<work_id>/chapters` (`post_without_preview_button` publishes,
  `save_button` saves an unposted AO3 chapter draft) or one
  `PUT /works/<work_id>/chapters/<chapter_id>` (`update_button`). On success the
  reader opens with a notice and the cached work is re-fetched once, best effort,
  so it shows the new chapter; an AO3 refusal re-renders the editor with AO3's
  messages and the composed text preserved. It is never retried — a timeout is
  recorded as ambiguous, so check the chapter on AO3 before trying again.
- Editing a posted work's metadata and tags: My Works rows offer **Edit Work**
  (`/write/work/edit?work_id=<id>`) when writes are enabled. The editor is
  prefilled from AO3's own `/works/<id>/edit` form, and the rating/warning/category/
  language choices come from the cached `/works/new` defaults. A confirmation page
  re-submits every value, then `/write/work/edit` sends exactly one
  `PATCH /works/<id>` carrying only `work[...]` metadata fields — never chapter
  text and never `work[posted]`. A 302 back to `/works/<id>` means updated (audit
  `work-updated`); a 200/422 `#error` re-render is AO3's refusal, shown on the
  editor with the fields preserved (audit `failed`); a timeout is recorded as
  ambiguous and never retried. On success the cached copy is re-fetched once (best
  effort) and the reader opens with a notice; an uncached work is opened through
  `/open/<id>`. Caveat: AO3's ownership filter redirects a non-author to the same
  `/works/<id>` path as a successful save, so the action is only offered on the
  account's own works and must not be used for arbitrary work ids.
- Managing works: `/my/works` and `/my/works/drafts` (linked from the signed-in
  header menu and from the dashboard sidebar's **Writing** section) show AO3's
  own works and drafts lists. **Post Draft** and
  **Delete This Work**/**Delete Draft** go through the confirmation page
  (`/write/preview?kind=work-publish|work-delete&id=<id>`) and send exactly one
  `PUT /works/<id>/post_draft` or `DELETE /works/<id>` carrying the CSRF token.
  Publishing opens the freshly cached work; a refusal (already posted or
  invalid) returns to My Drafts with a notice. Deletion is permanent: AO3
  removes the work, the app drops its local cached copy and the cached account
  lists, then returns to My Works with a notice. Neither action auto-retries; a
  timeout is treated as unknown, so verify the list on AO3 before trying again.
  The **Post a new work** link appears only with writes enabled; My Works and My
  Drafts need only a signed-in AO3 session.
- Client budgets: comments 3/15 min (AO3's own limit), kudos 10/min,
  subscriptions 20/h, bookmarks 10/min, new works 3/h, chapter saves 10/h,
  draft publishing 10/h, work deletes 5/h, work metadata edits 10/h.
  `Retry-After` parks the action.
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

- The FlareSolverr fallback is for reads only, one page per user action; for
  search that action is the search submit. A blocked search is solved once; it
  never crawls pages. It is
  not a crawler and cannot be enabled for writes.

- Chapter-level reading progress only; no scroll/block anchor yet (I7 refinement).
- Bookmarks and comments are created, not edited or deleted from the app.
- New-work posting creates the work plus its first chapter in one request;
  further chapters are added and corrected from the chapter manager. Chapter
  deletion is not implemented, and neither is AO3's dedicated chapter-reordering
  view (the editor's Chapter Number field can still change a chapter's position
  one at a time, but there is no reorder screen). A posted work's metadata and tags
  can be edited from `/write/work/edit` — that editor never touches chapter text,
  which is edited through the chapter manager — but byline/co-creator changes,
  work skins and work preferences (comment permissions, restricted) and scheduled
  or backdated posting stay on AO3. There is no scheduling or backdating UI: AO3
  dates a new chapter today and the publication date is never exposed. After a
  successful chapter save the cached copy of the work is re-fetched once (best
  effort) so the reader shows the change; if that refresh fails, the previous
  copy stays until "Refresh from AO3". **My Works**/**My Drafts** can publish a
  draft and delete a work or draft, and deletion is permanent — AO3 cannot undo
  it. Drafts live on AO3, not locally, and AO3 deletes unposted drafts after
  about 29 days. Management requires a signed-in AO3 session, and its actions
  additionally require `AO3I_WRITE_ENABLED=true`; nothing is scheduled or posted
  in the background.
- Cached comment threads are a read-only copy of AO3's rendered comments: there
  is no reply, edit or delete from the cached thread, and only AO3's first page
  is stored (further pages link out). The copy is keyed to the commentable the
  reader position maps to — `work` for Entire Work view or a one-chapter work,
  `chapter:<ao3_chapter_id>` for a single chapter of a multi-chapter work — so
  the thread can differ between views. Refreshing is explicit: the "Refresh
  comments" button under a cached thread does one `POST /read/<id>/comments`,
  which replaces the cached copy; a failed refresh keeps the old copy with a
  fixed notice, and nothing refreshes in the background.
- No OPDS, no batch import, no collection management.
- The local text is a cache for personal offline reading; do not redistribute it.
