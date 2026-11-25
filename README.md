# AO3Instance

AO3Instance is a self-hosted, offline-capable frontend for
[Archive of Our Own](https://archiveofourown.org). It sits between your browser
and AO3 as a personal caching layer: open a work once and it is stored locally
for offline reading, while your session, progress and library stay on your
machine.

It is a user agent, not a crawler. Every upstream request traces back to an
action you take in the UI; requests are spaced out, use an honest User-Agent,
and nothing is fetched in the background or in batches.

This is an unofficial project, not affiliated with or endorsed by the OTW.

## Features

- **Offline library** — works are fetched on demand, cached in SQLite and
  readable without a network connection; uncached works show a clear miss card
  instead of fetching silently.
- **Reader** — AO3-shaped pages at `/works/<id>` and `/read/<id>`, chapter
  jump, Entire Work mode, per-chapter reading progress, and cached AO3 comment
  threads: one click fetches AO3's rendered comments for the work or chapter
  you are reading, and they stay readable offline afterwards.
- **AO3 look and feel** — the familiar archive layout plus themes (Auto,
  Classic, Reversi, Snow, Frost, Cloud, Sepia, Midnight) and import of public
  AO3 site skins.
- **Discovery** — AO3 Work Search and tag search (30-minute cached, with
  offline fallback), fandom directories, popular tags, autocomplete, and a
  local full-text search over already-cached works; the AO3 Sort & Filter
  sidebar on tag pages and in the cached library becomes a collapsible "Sort
  and Filter" panel on narrow screens (open when a filter is active), while
  desktop keeps the sidebar open.
- **Your AO3 account** — cookie import (recommended) or password login, with
  the session jar encrypted at rest. Restricted works and all writes require a
  session. Logout is local-only; it does not sign you out of AO3. An expired or
  incomplete saved session is cleared on startup and every page says so, with a
  link straight to the cookie import, instead of looking signed in and failing.
- **Live AO3 lists** — subscriptions, bookmarks, reading history, Marked for
  Later, favorites and dashboard modules, cached for 30 minutes with offline
  fallback.
- **Writes (opt-in, off by default)** — kudos, comments, subscribe/unsubscribe,
  bookmarks, and mark for later/as read.
  Kudos and work subscriptions post directly from the reader the way AO3's own
  buttons do (the click is the deliberate action); comments and bookmarks go
  through a confirmation page that names the target. Every action is attempted
  once and recorded in an append-only audit ledger.
- **Comment replies, edits and deletes (opt-in)** — cached reader threads carry AO3's
  comment actions when writes are enabled: **Reply** on any comment and
  **Edit**/**Delete** on the signed-in account's own comments, with Edit offered only
  until the comment gets a reply (AO3's rule). The reply editor shows the parent
  comment; every action confirms locally, sends exactly one request to AO3, and
  re-fetches the cached thread once so the reader shows the change. Replies count
  against AO3's 3-per-15-minutes comment limit; edits and deletes are budgeted at
  10/hour each. A delete is permanent (AO3 keeps a deleted-comment marker when the
  thread has replies), and every attempt is recorded in the audit ledger.
- **Bookmark editing and deletion (opt-in)** — the live AO3 bookmarks page gains
  **Edit Bookmark** and **Delete Bookmark** on each row when writes are enabled.
  The editor is prefilled from AO3's own bookmark edit page and covers notes, your
  tags, collections and the private/rec flags; it validates locally and confirms
  before exactly one `PATCH /bookmarks/<id>` (booleans are always sent as 1/0 so
  unchecking clears them; the pseud is left untouched). Delete warns that AO3
  removes the bookmark permanently, then sends exactly one `DELETE /bookmarks/<id>`.
  Both invalidate the cached bookmarks page and return to `/bookmarks` with a
  notice; each action is budgeted at 10/hour and never retried.
- **Posting new works (opt-in)** — a composer at `/works/new` mirrors AO3's
  new-work form (rating, archive warnings, categories, tags, title, creator
  pseud, summary, notes, language, collections, gifts, chapter title, work
  text). Its options are read from AO3's own page, the work is checked against
  AO3's limits and previewed locally, then exactly one confirmed request
  publishes it or saves it as a draft on AO3.
- **Author works manager (opt-in)** — **My Works** and **My Drafts** list what
  AO3 reports for your own account (the lists are AO3's pages, cached for 30
  minutes with an honest offline notice). A draft can be published, and a posted
  work or draft deleted, from the app: each action goes through a confirmation
  page, sends exactly one request and is never retried.
- **Chapter management (opt-in)** — add chapters to, and correct chapters of,
  works you authored. `/my/works/<id>/chapters` renders AO3's own chapter index
  (30-minute cached) in order with chapter dates and Edit links; the editor
  suggests the next chapter number and prefills an edit from AO3's chapter form
  (raw author HTML, not entities). A save is previewed locally and confirmed,
  then sends exactly one request — Post or Save as Draft for a new chapter,
  Update for an existing one — and the cached work is re-fetched once afterwards
  so the reader shows the change. AO3 refusals appear on the editor with the
  composed text preserved; nothing is retried.
- **Work metadata editing (opt-in)** — edit a posted work's title, rating, archive
  warnings, categories, fandom/relationship/character/additional tags, summary,
  notes, end notes, language, collections and gifts from `/write/work/edit` (the
  **Edit Work** link on My Works rows). The editor is prefilled from AO3's own edit
  form, checked against AO3's limits and previewed locally, then sends exactly one
  `PATCH /works/<id>` — never retried. A cached work is re-fetched once afterwards
  so the reader shows the change; chapter text is not part of this editor.
- **Diagnostics** — `/diagnostics` shows upstream HTTP failures, the request
  governor and optional FlareSolverr state, without exposing cookies.
- **Optional FlareSolverr** — if the host already runs FlareSolverr, blocked
  reads and searches can be solved on demand (one page, hard cooldown and
  budget). It is never a build dependency and is never used for writes.

## Requirements

- **Nix with flakes enabled.** The flake is the only supported build and run
  path and pins the Rust toolchain and native dependencies.
- A writable data directory (defaults to `$XDG_CONFIG_HOME/ao3instance`).
- Outbound HTTPS to `archiveofourown.org`.
- *Optional:* a host-provided FlareSolverr service on loopback for Cloudflare
  challenges. The app builds, runs and passes tests without it.

## Build

```sh
nix build
./result/bin/ao3i-server
```

Or build and run in one step:

```sh
nix run .
```

For development, the flake provides the entire toolchain:

```sh
nix develop
cargo run --bin ao3i-server
```

## Install

### Run directly

```sh
nix run .
# AO3Instance is now at http://127.0.0.1:8099/
```

By default it binds to loopback only and writes to
`$XDG_CONFIG_HOME/ao3instance` (`ao3i.db` plus the encryption `key`).

### NixOS module

The flake exposes `nixosModules.default`. Add this repository as a flake input
and import the module, then:

```nix
services.ao3instance = {
  enable = true;
  dataDir = "/var/lib/ao3instance";
  port = 8099;
  # openFirewall = true;      # default: loopback only
  # flaresolverrPort = 8191;  # optional host-provided FlareSolverr
  # writesEnabled = true;     # opt in to kudos/comments/posting; import AO3 cookies at /session
};
```

The module creates the data directory (mode `0700`, owned by the `ao3instance`
system user), runs `ao3i-server`, restarts on failure, and applies systemd
hardening. `writesEnabled = true` sets `AO3I_WRITE_ENABLED=true`; writes also
need a usable AO3 session, imported on the **Session** page (both the
`_otwarchive_session` and `user_credentials` cookies), otherwise the UI explains
that the session is unusable instead of pretending the reader is signed out. Back up the whole data directory (database **and** key); without the
key the session cannot be decrypted.

## Configuration

All settings are environment variables with the `AO3I_` prefix:

| variable | default | meaning |
| --- | --- | --- |
| `AO3I_BIND` | `127.0.0.1` | listen address |
| `AO3I_PORT` | `8099` | listen port |
| `AO3I_DATA_DIR` | `$XDG_CONFIG_HOME/ao3instance` | data directory |
| `AO3I_DB_FILE` | `<data_dir>/ao3i.db` | SQLite database |
| `AO3I_KEY_FILE` | `<data_dir>/key` | AES-256-GCM key (mode `0600`) |
| `AO3I_BASE_URL` | `https://archiveofourown.org` | upstream origin (sandbox/testing) |
| `AO3I_USER_AGENT` | built-in | set a project URL and contact before public use |
| `AO3I_WRITE_ENABLED` | `false` | master switch for all writes |
| `AO3I_MIN_SPACING_MS` | `2000` | floor between upstream requests |
| `AO3I_FLARESOLVERR` | `auto` | `auto`, `on` or `off` for the optional solver |
| `AO3I_FLARESOLVERR_URL` | `http://127.0.0.1:8191` | solver origin (loopback-only by default) |

See [docs/operator.md](docs/operator.md) for the complete table, including HTTP
diagnostics and FlareSolverr limits.

## First use

1. Open `http://127.0.0.1:8099/`.
2. Open a public work by id or URL: it is fetched once, cached, and readable
   offline afterwards.
3. For restricted or adult works and for writes, go to **Session**. Cookie
   import is the recommended path (copy `_otwarchive_session` and
   `user_credentials` from your browser). Password login works, but the
   password is never persisted.
4. Writes remain disabled until `AO3I_WRITE_ENABLED=true`. Kudos and work
   subscriptions post straight from the reader; comments and bookmarks confirm
   first. Cached comment threads also gain **Reply** — and **Edit**/**Delete** on
   your own comments — once writes are enabled, each through its own confirmation.
   With a session, the header's **Post** link opens the new-work
   composer, and **My Works** rows link to each work's chapter manager
   (**Chapters**; the **Add Chapter** shortcut appears for works still in
   progress once writes are enabled) and, once writes are enabled, to a posted
   work's metadata editor (**Edit Work**). Every write appears in **Audit**.

## Limitations

- Cached comment threads mirror AO3's rendered comments. When writes are enabled, the
  reader offers **Reply** on every comment and **Edit**/**Delete** on your own comments;
  Edit disappears once a comment has a reply (AO3's rule), and a delete is always the
  commenter's own — work authors cannot delete other people's comments from here. Work-author
  moderation (approving/rejecting, freezing/hiding) and inbox replies are not offered, and
  only AO3's first page is cached — further pages link out.
  AO3 answers some edit/delete refusals with the same redirect as success (the reason is
  only a flash), so the app scopes both to your own comments and the audit records the
  verified request, not a guaranteed effect. A deleted comment with replies becomes AO3's
  deleted-comment marker upstream; the cached copy drops deleted comments on the next
  refresh. A "Refresh comments" button under the thread replaces the copy on demand — one
  explicit request; nothing refreshes in the background.
- Cached comments follow AO3's commentable for the view you are reading: the
  work itself in Entire Work mode (and for a one-chapter work), or one chapter
  in single-chapter view of a multi-chapter work — so switching views can show
  a different thread.
- Bookmark editing and deletion cover bookmarks on works. Creation stays the
  reader's **Bookmark** confirmation (the default pseud, no notes/tags/collections
  form — those are added by editing afterwards), the editor cannot change which
  pseud owns the bookmark, and collection curation stays on AO3. External-work
  bookmarks (AO3's bookmarklet/ExternalWork path) are not created by the app and
  have no local bookmark receipt, so the bookmark flow is work bookmarks only.
  AO3 answers a non-owner's edit or delete with the same redirect shape as a
  success (only the flash differs), so the links are offered only on your own
  live bookmarks page and the audit records the verified request; a delete is
  permanent, and a failed or timed-out request is never retried — check the
  bookmark on AO3 before trying again.
- New-work posting creates the work and its first chapter in one request;
  further chapters are added from the chapter manager, and an existing chapter
  can be corrected there. A posted work's metadata and tags can be edited from
  `/write/work/edit` (title, rating, archive warnings, categories, tags, summary,
  notes, end notes, language, collections and gifts), but chapter text is edited
  only from the chapter manager and there is no chapter delete or reorder screen —
  those stay on AO3 (the editor can still change a chapter's Chapter Number).
  Byline/co-creator changes, work skins and work preferences (comment permissions,
  restricted) and scheduled or backdated posting are not exposed here: they stay on
  AO3. There is no scheduling or backdating UI: AO3 dates a new chapter today and
  the publication date is never exposed. A successful save re-fetches
  the cached work once (best effort) so the reader shows it immediately; if that
  refresh fails, the previous cached copy stays until "Refresh from AO3". The
  author works manager can publish a draft and delete a work or draft, but
  deletion is permanent — AO3 cannot undo it — and drafts are AO3 objects that
  the archive deletes after about 29 days. Nothing is scheduled or posted in the
  background.
- Author works management lives at `/my/works` and `/my/works/drafts`. The lists
  need a signed-in AO3 session (they are AO3's own pages, cached for 30 minutes),
  and publishing a draft or deleting a work additionally needs
  `AO3I_WRITE_ENABLED=true`. Each action is one confirmed request with no
  auto-retry; a timed-out action is treated as unknown, so check My Works/My
  Drafts on AO3 before trying again.
- Posting a new work needs `AO3I_WRITE_ENABLED=true` and a signed-in AO3 session
  too. A timed-out create is reported as ambiguous — the work may already be live
  — so check your works list on AO3 before retrying; the app never retries a
  create itself.
- Editing work metadata uses the same gate (write switch and signed-in session).
  Each save confirms locally and sends exactly one `PATCH /works/<id>`; the editor
  never touches chapter text. A timed-out edit is reported as unknown — the change
  may have landed — so check the work on AO3 before retrying.

## Security and privacy

- The app has **no user model of its own** beyond the AO3 session. It binds to
  loopback by default; if you expose it, put it behind a TLS reverse proxy and
  add authentication.
- Secrets at rest are encrypted with AES-256-GCM using a key file created with
  mode `0600`. Passwords are never stored.
- Writes are opt-in, one action at a time, recorded before and after sending,
  and never retried blindly. The audit log stores targets and outcomes, never
  payloads, cookies or passwords.
- The local cache is for personal offline reading. Do not redistribute cached
  work text.
- Only reachable loopback FlareSolverr instances are accepted unless
  `AO3I_FLARESOLVERR_ALLOW_REMOTE=true` is set deliberately. Never expose
  FlareSolverr itself to a network.

## Development

```sh
nix develop

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

# Optional: verify AO3 route claims against a local otwarchive clone
sh scripts/check-ao3-routes.sh
```

CI runs formatting, clippy (warnings as errors), the test suite and the AO3
route drift gate. The `otwarchive/` reference clone is gitignored and never
committed.

## Documentation

- [docs/operator.md](docs/operator.md) — running, configuration, pages,
  troubleshooting, limitations
- [docs/adr/](docs/adr/) — architecture decisions (auth, access posture, write
  scope and safety, work creation, FlareSolverr)
- [docs/ao3-write-api.md](docs/ao3-write-api.md) — AO3 action contracts and
  failure taxonomy
- [docs/failure-matrix.md](docs/failure-matrix.md) and
  [docs/security-standards.md](docs/security-standards.md)

## Special thanks

- [otwcode/otwarchive](https://github.com/otwcode/otwarchive) — AO3 is open
  source, and this project studies its routes, markup and policies to stay
  faithful. I am deeply grateful for AO3 and everyone who keeps it running;
  this tool is only a way to make my own reading less dependent on constant
  connectivity.
