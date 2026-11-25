# Write client — implementation plan

How AO3Instance talks to AO3 with an account. Companion to
[`ao3-write-api.md`](ao3-write-api.md) (contracts), [`ao3-internals.md`](ao3-internals.md)
(evidence), [`security-standards.md`](security-standards.md) (secrets) and ADR-0001…0005
(decisions).

This plan replaces the read-only assumption in [`init-audit.md`](init-audit.md) §2 D3/D4.

---

## 1. Shape

```
web UI ──► local BFF ──► write pipeline ──► AO3 gateway ──► archiveofourown.org
             │                │                  │
             │                ├─ intent log      ├─ cookie jar (encrypted)
             │                ├─ audit log       ├─ CSRF token (memory)
             │                └─ rate governor   └─ one request in flight
             ▼
        local DB + blob store
```

The pipeline is the only thing that may write upstream. Read-side parsing and the write
pipeline share the same gateway, the same limiter and the same fixtures.

Module map (one crate, bins later — same shape as MUSIKA). **W1 landed** the
first nine; the rest are marked with the milestone that owns them.

```
src/
  lib.rs           module wiring only
  consts.rs        timeouts, UA, paths, body cap (one place)
  config.rs        typed Config; the only reader of AO3I_* env
  error.rs         typed error taxonomy (docs/ao3-write-api.md §6)
  crypto.rs        AES-256-GCM store, key file, SecretString, wipe helpers
  ao3/
    client.rs      HTTP boundary: never follows redirects, streams with a body
                   cap, request spacing, CSRF-carrying DELETE, redacted Debug
    session.rs     state machine: import, password login, token, probe, logout,
                   kill; login outcomes classified from Location
    jar.rs         cookie jar: parse/apply/expiry, completeness, encryption
                   primitives (the at-rest wiring is W3 persistence)
    parse.rs       CSRF meta, flash slots, percent-(de)encode shared by form+path
    work.rs        W2 — work page parser: title, byline, tags, stats, chapters,
                   notes, comment-form presence (selectors traced to the clone)
    limiter.rs     W3 — per-action budgets + Retry-After parking
    parse/         W2 — checked-in fixtures per surface (work, comment, subscription)
    actions/       W3/W4/W5 — kudos.rs, comments.rs, subscriptions.rs, …
    outcome.rs     W3 — classification precedence + typed outcomes
    audit.rs       W3 — intent + audit rows, redaction
  db.rs            SQLite pool, pragmas, single migrator (`migrations/`)
  store.rs         cached works/chapters, content hash, deterministic listing
  render.rs        server-rendered pages, escaping, ammonia sanitizer
  server.rs        axum routes: /, /open, /read/{id}, /read/{id}/chapter/{idx}, /health
  bin/ao3i-server.rs  thin entry point
```

Configuration knobs (all read once by `config.rs`; unknown `AO3I_*` keys are ignored):

| env | default | meaning |
| --- | --- | --- |
| `AO3I_BASE_URL` | `https://archiveofourown.org` | upstream origin (sandbox override) |
| `AO3I_DATA_DIR` | `$XDG_CONFIG_HOME/ao3instance` | key and (later) database location |
| `AO3I_KEY_FILE` | `<data_dir>/key` | AES key path (must be absolute; `.env` cannot set it) |
| `AO3I_USER_AGENT` | built-in UA | **set this before public use**: ADR-0002 requires a project URL and contact |
| `AO3I_WRITE_ENABLED` | `false` | master switch for upstream writes (W6 gate) |
| `AO3I_MIN_SPACING_MS` | `2000` | client-side floor between upstream requests |
| `AO3I_BIND` | `127.0.0.1` | address the web app binds to |
| `AO3I_PORT` | `8099` | port the web app listens on |
| `AO3I_DB_FILE` | `<data_dir>/ao3i.db` | SQLite cache |

---

## 2. Session state machine

```
import / login ──► Imported ──probe 200 + CSRF token──► Active
                      │                                    │
                      │ validation                          ├─ 302 /users/login  ─► LoggedOut
                      ▼                                     ├─ 302 /lost_cookie ─► SessionIncomplete
                Imported (fail closed)                      ├─ 403 / 503        ─► Blocked
                                                            └─ operator kill    ─► Killed (terminal)
```

Rules (implemented in `src/ao3/session.rs`):

- `Active` requires a successful authenticated read **and** a CSRF token. A verified jar
  without a token is `Imported` — held, but never usable.
- `Imported` is also the fail-closed state after an ambiguous validation (unknown
  redirect, unexpected status, token fetch failure). It never lets a write through.
- `LoggedOut` and `SessionIncomplete` are distinct: the first means re-login, the second
  means the import was incomplete (usually `user_credentials` missing). Both **wipe the
  jar**; a rejected jar is never kept, or AO3 would append `user_credentials` and revive it.
- `Killed` is terminal: every network method returns `Error::Killed` with no request until
  the operator restarts the process.
- `probe()` returns the **read proof** (`Active` when the read authenticated) while
  `state()` stays `Imported` if no token is held; usability always comes from `state()`.
- Logout retries exactly once with a refreshed token when AO3 answers `/auth_error`, then
  reports `LogoutFailed`. Signing out is idempotent, so one retry is safe.
- A cookie rotation observed on any response drops the CSRF token with the jar.
- Login outcomes are classified from `Location` by whole path segments, not from "302
  means happy":
  `/users/login` → `LoginRejected`, `/auth_error` → `CsrfExpired`,
  `change_password` → `PasswordChangeRequired`, `/users/<login>` → success and the
  canonical login for the probe (which also makes email logins work). Protocol-relative
  and off-origin `Location`s are drift, never followed. Known ambiguity: an account named
  literally `login` redirects to `/users/login`, which is identical to the login page — we
  fail closed; cookie import still works for it.
- Liveness caching (60 s), a governor, and the single-in-flight rule are W3: W1 probes per
  validation, applies the request-spacing floor, and serializes through `&mut self` only.
- A user named literally `login` redirects to `/users/login` (identical to the login page):
  we fail closed; cookie import still works.

---

## 3. CSRF flow

```
login / import  ─► probe ─► GET /token_dispenser.json ─► token in memory
                                    │
write ─► token present? ── no ──────┘ (refetch once, then fail CsrfExpired)
   │
   └─► POST with authenticity_token
          ├─ 302 /auth_error ─► refresh token; retry only ServerUnique actions
          └─ ok
```

The token is masked by Rails and therefore not stable; it is stored as an opaque string
with a session id, never compared. It is dropped whenever the jar changes.

---

## 4. Action pipeline

Every action runs these steps in order. A failure at any step yields a typed outcome and
writes an audit row.

1. **Gate** — `AO3I_WRITE_ENABLED`? session `Active`? kill switch? if not, stop.
2. **Preflight read** — fetch the target state (or use a fresh cached read):
   - kudos: already given? own work? blocked?
   - comments: form present? disabled? frozen? moderated?
   - subscribe: already subscribed (index lookup)?
   - bookmark: already bookmarked?
   - reading flags: current state?
3. **Confirm** — the UI has already collected an explicit confirmation with the target
   named; the pipeline receives a confirmation token proving it (no server-side "just do
   it" path).
4. **Intent** — append `write_intent` (committed) including `payload_hash`.
5. **Send once** — form-encoded, `Referer` set to the target page, honest UA, through the
   governor.
6. **Classify** — precedence: JSON envelope → final URL → body marker → verification read
   (`ao3-write-api.md` §5).
7. **Record** — complete the audit row with status, final URL, evidence selector, duration;
   update local state (subscription list, reading flags) only from verified facts.

No step may be skipped for "simple" actions. Kudos is simple until it double-posts.

---

## 5. Idempotency classes

| class | actions | retry | duplicate handling |
| --- | --- | --- | --- |
| `ServerUnique` | kudos | yes, after refresh | map "already left kudos" to `AlreadyDone` |
| `ServerConstrained` | favorite tags | yes | 422 duplicate → `AlreadyDone` |
| `ClientGuarded` | comments, bookmarks | **no** | verify by read; re-confirmation required |
| `NoConstraint` | subscriptions | **no** | pre-check the index; never blind POST |
| `NonIdempotentCreate` | work create | **no** | read the 302 to classify; a timeout is ambiguous — verify on AO3 before retrying |

The class is a property of the action, declared next to its handler and asserted by a
test. A new action without a class does not compile (or fails the gate).

---

## 6. Local schema (sketch)

```sql
ao3_account(
  id TEXT PRIMARY KEY, login TEXT NOT NULL UNIQUE,
  auth_kind TEXT NOT NULL CHECK(auth_kind IN ('imported','password')),
  secret_enc TEXT,              -- password ciphertext, NULL for imported
  state TEXT NOT NULL, created_at INTEGER NOT NULL, last_ok_at INTEGER
);

ao3_session(
  account_id TEXT PRIMARY KEY REFERENCES ao3_account(id),
  jar_enc TEXT NOT NULL,        -- whole cookie jar, AES-GCM, AAD 'ao3/jar'
  jar_updated_at INTEGER NOT NULL,
  expires_hint INTEGER          -- from cookie expiry, advisory only
);

write_intent(
  id TEXT PRIMARY KEY, account_id TEXT NOT NULL,
  action TEXT NOT NULL, target_kind TEXT NOT NULL, target_id TEXT NOT NULL,
  payload_hash TEXT NOT NULL, idem_class TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','done','failed','ambiguous')),
  created_at INTEGER NOT NULL
);

write_audit(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  intent_id TEXT REFERENCES write_intent(id),
  outcome TEXT NOT NULL, http_status INTEGER, final_url TEXT,
  evidence TEXT, duration_ms INTEGER, created_at INTEGER NOT NULL
);

rate_budget(account_id TEXT, action TEXT, window_start INTEGER, count INTEGER,
            PRIMARY KEY(account_id, action, window_start));
```

Notes:

- No CSRF token column. It lives in memory only.
- `payload_hash` is a hash of the canonical form fields, not the text; the comment body is
  not stored for privacy.
- Audit rows are append-only; a delete requires an explicit operator action.

---

## 7. Rate governor

- One semaphore across the whole process: max 1 upstream request in flight.
- Token bucket: 1 request / 2 s sustained, burst 3, jitter ±500 ms.
- Per-action caps: comments ≤ 3 / 15 min (the server's own limit), kudos ≤ 10 / min,
  subscriptions ≤ 20 / min.
- `429` → read `Retry-After`, park the account for that duration, surface a countdown.
- `403`/challenge → `BlockedByEdge`, stop all requests for 10 min, alert the operator.
- Reads and writes share the governor: a page fetch competes with a pending write, and a
  write always wins the next slot.

---

## 8. UI requirements

- A single "act on AO3" affordance per action, with the target named in the confirmation
  ("Leave kudos on *Work Title* by *author*").
- Outcomes: `success`, `failed` (with the exact AO3 message), `already done`,
  `unconfirmed` + "Check" button, `needs re-login`, `blocked`, `rate limited (wait N)`.
- Never optimistic. The local state updates only after verification.
- A session panel: account, state, last verified, "re-import cookie", "log out upstream".
- An audit page: last N writes with outcomes, filterable; no secrets shown.

---

## 9. Testing

| tier | what | where |
| --- | --- | --- |
| 1 | parsers and classifiers against checked-in fixtures | `cargo test`, fast |
| 2 | request construction against a mock upstream (asserts fields, token, Referer, single POST) | `cargo test` |
| 3 | golden paths and failure injection against a local otwarchive sandbox (`docker compose`) | opt-in `AO3I_SANDBOX=1`, never CI-default |
| 4 | production | never automated; manual, by the owner, with the kill switch ready |

Oracle strings and statuses come from the clone's specs:
`spec/controllers/kudos_controller_spec.rb:85-140`, `spec/controllers/comments/`,
`spec/controllers/subscriptions_controller_spec.rb`.

The fault matrix in ADR-0005 is the acceptance test for W6.

---

## 10. Milestones

| id | scope | exit criteria |
| --- | --- | --- |
| **W0** | docs, `.gitignore`, route gate, audit update | `scripts/check-ao3-routes.sh` green; docs cross-link; audit D3/D4/D7 marked superseded |
| **W1** | session core: jar model + crypto, import, password login, token, probe, logout | every state is exercised (Active, Imported, LoggedOut, SessionIncomplete, Blocked, Killed); no secret in any serialized form |
| **W2** | read-state parsers: kudos state, comment form, subscription list, bookmark state, reading flags | preflight decides correctly on fixtures |
| **W3** | subscriptions write + audit + governor | duplicate prevented; 429 parked; audit complete; JSON outcome |
| **W4** | kudos + reading flags + bookmarks | exact AO3 strings mapped; verification reads |
| **W5** | comments (top-level + reply) | rate limit, spam, duplicate, frozen, moderation all typed; no auto-retry |
| **W6** | failure injection + sandbox suite + kill switch + operator docs | ADR-0005 fault matrix passes; `AO3I_WRITE_ENABLED=false` blocks all writes |

### W6 status — partial (2026-11-22): comments

**Landed:**
- **Comments**: `POST /works/:id/comments` with `comment[pseud_id]` +
  `comment[comment_content]`; the default pseud is discovered from
  `/users/:login/pseuds` (parser traced to `pseuds/_pseud_module.html.erb`,
  default marked `span.current`) and persisted with the encrypted session.
- Public action: goes through the same confirmation page, now showing the exact
  comment text; governor budget 3/900 s (AO3's own limit); one request, never retried.
- Failure classification from the re-rendered form: duplicate-content, spam filter,
  closed/hidden/unrevealed comments, generic validation, HTTP 429 -> distinct
  notices, all audited as failed with the AO3 message as evidence.
- Success is read from the redirect fragment `#comment_<id>`; audit outcome
  "commented" records the id.
- Tests: 137 total; new `tests/writes_comments.rs` (pseud discovery + preview +
  post + audit, duplicate, spam, 429, overlong/unconfirmed no-network).
  Evidence: `notes/gates/w6.txt`.
- **Bookmarks**: `POST /works/:id/bookmarks` with the discovered pseud; success is
  a redirect to `/bookmarks/<id>` (id audited), failures typed; the local `bookmarks`
  ledger (migration 0007) prevents double-posting. Timeout against an unreachable or
  edge-blocked upstream is covered by the failure matrix.
- **Operator docs** ([operator.md](operator.md)) and the **failure matrix**
  ([failure-matrix.md](failure-matrix.md)) with tests in `tests/failure_matrix.rs`
  (offline cache reads, unreachable upstream, 403, 429, restricted redirect).
- Decision closed: [ADR-0006](adr/0006-no-password-custody.md) — password login is
  supported but the password is never persisted; re-authenticate when AO3 expires
  the session. This supersedes ADR-0001's encrypted-password storage.
- Reading position stays chapter-anchored for v0 (I7 refinement needs a JS bridge
  that would weaken the sandbox; recorded as a future item, not an open task).

**Remaining W6:** sandbox validation against a local otwarchive (operator step; heavy
Docker stack), then final gates and the release skill.

### W6 addendum — 2026-11-25: direct kudos/subscribe and cached comments

**Direct posts.** Reader kudos and work subscribe/unsubscribe now post directly from the
work page like AO3's own forms: `form.new_kudo` → `POST /write/kudos` in the bottom
`#feedback` commentable actions, and `form.ajax-create-destroy` → `POST /write/subscribe`
or `/write/unsubscribe` from AO3's top `ul.work.navigation.actions` bar next to Comments
(its `_work_header_navigation` placement). The button click is the deliberate action; kudos
and subscribe carry the hidden `confirm=yes` token (unsubscribe needs none — it only removes
a ledger-known subscription), and the write gate, session requirement, intent/audit ledger,
governor, single attempt and same-origin guard are unchanged. Comments and bookmarks keep
the confirmation page. The decision and its trade-off (no preview page for those two
actions) are recorded as the amendment in [ADR-0005](adr/0005-write-safety.md); duplicate
safety rests on the pre-write local kudos receipt and subscription-ledger checks.

**Cached comments.** Migration `0014_work_comments.sql` stores a copy of AO3's rendered
comment thread. The cache is keyed by commentable: `work` for Entire Work view (and for a
one-chapter work, whose `/works/<id>` URL is the work page), or `chapter:<ao3_chapter_id>`
for a single chapter of a multi-chapter work (the id comes from the chapter header link, so
a work cached before this migration falls back to the work scope until re-fetched).
`POST /read/<id>/comments` is the only fetch path: one upstream GET per click, then a
redirect back to `?show_comments=1`, which renders from SQLite and never contacts AO3; a
plain reader GET never fetches. A cached thread carries an explicit "Refresh comments"
button that submits the same POST and replaces the scope's copy; nothing refreshes in the
background, and a failed fetch or parse keeps the previous copy with a fixed notice. Only
AO3's first page is cached (`has_more` links out); deleted or hidden comments are dropped
from the cache and their replies are hoisted to the omitted comment's parent rather than
lost. Bodies are untrusted author HTML, stored raw like chapter text and sanitized at
render. Tests: `tests/comments_cache.rs`.

### Work creation addendum — 2026-11-25

**Decision.** New-work creation is now in scope ([ADR-0008](adr/0008-work-creation.md)),
superseding the "works/chapters creation" deferral in ADR-0003 for new works and chapter
management. The composer at `GET /works/new` mirrors AO3's own new-work form: rating,
archive warnings, categories, fandom/relationship/character/additional tags, title,
creator pseud, summary, notes, language, collections, gifts, chapter title and work text.
The option lists (ratings, warnings, categories, numeric language ids, creator pseuds) are
parsed from AO3's real `/works/new` page and cached for 10 minutes, so no private database
id is hardcoded. AO3's model limits are mirrored locally (title 1–255, summary 1,250,
notes 5,000, each tag 150, at most 75 user-defined tags, work text 10–510,000, at least one
warning, rating/fandom/language/pseud required), so an obviously invalid work never
reaches the archive.

**Pipeline.** `POST /write/work/preview` renders a local confirmation with no network
write; the confirmation page re-submits the same values with `confirm=yes` to
`POST /write/work`, which sends exactly one `POST /works` with
`work[chapter_attributes][content]`, `work[author_attributes][ids][]`,
`work[rating_string]`, `work[archive_warning_strings][]`, `work[fandom_string]`,
`work[language_id]` and the optional fields, plus `post_button` (publish) or `save_button`
(Save as Draft). The outcome is classified from the redirect: a 302 to `/works/:id` means
posted (audit `work-posted`, then the app caches and opens the work through `/open/:id`);
a 302 to `/works/:id/preview` means saved as a draft (audit `work-draft`, then a
`/session` notice — AO3 deletes drafts about 29 days after creation); a 200 re-render with
`#error` blocks means failed (audit `failed`, AO3's messages shown, composed text kept);
a timeout is audited `ambiguous` and never retried. The `post_work` governor budget is
3 per hour.

**Chapter management.** A chapter is added or edited through AO3's own surfaces, with
the same write spine. `GET /my/works/<id>/chapters` fetches `/works/<id>/navigate`
through the 30-minute page cache and parses `ol.chapter.index.group > li > a` plus
`span.datetime` (work title from `h2.heading`; AO3's `<position>. ` display prefix is
stripped), listing chapters in order with dates, Edit links and Add Chapter; My Works
rows gained **Chapters**, and **Add Chapter** for works still in progress when writes
are enabled. The editor at `/write/chapter/new|edit` mirrors
`chapters/_chapter_form.html.erb`: a new chapter suggests the next position from the
parsed index, and an edit is prefilled from AO3's `GET /works/<w>/chapters/<c>/edit`
(`#chapter-form`), with textarea values decoded to raw author HTML rather than
serialized entities. Fields are `chapter[title]`, `chapter[position]`,
`chapter[wip_length]`, `chapter[summary]`, `chapter[notes]`, `chapter[endnotes]`,
`chapter[content]`, validated locally against AO3's chapter limits (title 255,
summary 1,250, notes 5,000, text 10–510,000; `?` is accepted as "total unknown" like
AO3 renders it). `POST /write/chapter/preview` renders a local confirmation with no
network write; `POST /write/chapter` then sends exactly one request — create
`POST /works/<w>/chapters` with `post_without_preview_button` (posted) or
`save_button` (chapter draft), update `PUT /works/<w>/chapters/<c>` with
`update_button`. A 302 to `/works/<w>/chapters/<c>` means saved (audit
`chapter-posted`/`chapter-updated`, the new id parsed from `Location`; on update the
redirect must name the edited chapter, otherwise it is AO3's permission/not-found
refusal); a 302 to `…/preview` means chapter draft (audit `chapter-draft`); a 200/422
`#error` re-render is AO3's refusal (audit `failed`, AO3's messages and the composed
text kept on the editor); a timeout is audited `ambiguous` and never retried. After a
success the cached work is re-fetched once, best effort, so the reader shows the new
chapter; a refresh failure is logged and never turns the write into an error. The
`chapter` governor budget is 10 per hour. Tests: `tests/writes_chapters.rs` (7) plus
`ao3::chapters` parser/validation unit tests (4); full suite 309 green; the route gate
then stood at 57 claims and the wiring crawl covers `/my/works/12345/chapters`. Evidence:
`notes/gates/chapter-management.txt`.

**Work metadata editing.** A posted work's metadata and tags are edited through AO3's
own edit surface with the same write spine. `GET /write/work/edit?work_id=<id>` fetches
`/works/<id>/edit` live and parses `form#work-form` (`works/edit.html.erb` →
`_standard_form.html.erb`) for the current title, rating, archive warnings, categories,
fandom/relationship/character/additional-tag strings, numeric language id, summary,
notes, end notes, collections and gift recipients; the select/checkbox choices come
from the composer's cached `/works/new` defaults, so no private id is hardcoded, and a
page without `#work-form` fails closed (`UpstreamChanged`). The preview route
`POST /write/work/edit/preview` runs `validate_edit_work` (title 255, summary 1,250,
notes/end notes 5,000, each tag 150, at most 75 user-defined tags; title, rating, at
least one warning, fandom and language required) and renders a confirmation page that
re-submits every value as hidden fields. `POST /write/work/edit` then sends exactly one
`PATCH /works/<id>` with the `work[...]` fields: title, rating, fandom, language,
summary, notes, end notes, collections, recipients, repeated
`work[archive_warning_strings][]` / `work[category_strings][]`, and optional
relationship/character/additional-tag strings — no `work[posted]`, no button name and
never chapter text. A 302 back to `/works/<id>` means updated (audit `work-updated`;
the UI only offers this from My Works, because AO3's ownership filter redirects a
non-author to the same work path), a 200/422 `#error` re-render is AO3's refusal shown
on the editor with the fields preserved (audit `failed`), and a timeout is audited
`ambiguous` and never retried. After success a cached work is re-fetched once (best
effort) and opens in the reader with `notice=work_updated`; an uncached work goes
through `/open/<id>`. My Works rows gained **Edit Work** for posted works when writes
are enabled. The `edit_work` governor budget is 10 per hour. Tests:
`tests/writes_work_edit.rs` (5) plus `ao3::compose` parse/validate unit tests (2) and an
`ao3::actions` redirect-classification test; full suite 317 green; the route-drift gate
gained works#edit/update and the warning/category array claims (61 total). Evidence:
`notes/gates/work-edit.txt`.

**Scope.** New work plus first chapter, publish or draft; chapter management (index,
add, edit); work metadata/tag editing; and the works manager with its publish/delete
actions (next addendum). Chapter deletion/reordering, byline/co-creator changes, work
skins and work preferences, and scheduling or backdating stay later work. Tests:
`tests/writes_work.rs` (6) plus `ao3::compose` unit tests.

### Works management addendum — 2026-11-25

**Lists.** `GET /my/works` and `GET /my/works/drafts` render AO3's own
`/users/<login>/works` and `/users/<login>/works/drafts` pages for the signed-in
account through the same 30-minute SQLite page cache as the other live lists
(`?refresh=1` forces a fetch, `?page=N` pages). The parser
(`src/ao3/account_lists/works.rs`) follows `works/_work_blurb.html.erb`,
`works/index.html.erb` and `works/drafts.html.erb`: blurb title/fandoms/summary/stats,
the draft marker (the `post_draft` action or the draft caution notice) and numbered
pagination. The local cache is used only to choose "Open locally" versus the
one-click `/open/<id>` — the list itself is an AO3 fact. A cache miss or parse failure
renders an honest "could not load this list from AO3" notice and leaves the cached
library untouched; drafts carry AO3's ~29-day deletion caution.

**Publish a draft.** The drafts list offers **Post Draft**, which links to the
existing confirmation page (`/write/preview?kind=work-publish&id=<id>`) and then
`POST /write/work/publish`; the handler commits an intent row and sends exactly one
`PUT /works/<id>/post_draft` carrying just the CSRF token. A 302 to `/works/<id>`
means published: the audit row closes `work-published`, the cached account lists are
invalidated, and the app redirects to `/open/<id>`, which caches and opens the freshly
live work. A 302 to the owner edit page is AO3 refusing (already posted or invalid):
the audit row closes `failed` and My Drafts shows a notice. A transport error is not
retried — the outcome is unknown and the user verifies the list. The `publish_draft`
governor budget is 10 per hour.

**Delete a work.** Both lists offer **Delete This Work**/**Delete Draft**, which warn
on the confirmation page (`kind=work-delete`) that deletion is permanent and cannot be
undone; `POST /write/work/delete` then sends exactly one `DELETE /works/<id>` with the
CSRF token (body field and header). A 302 to `/users/<login>/works` (posted) or
`/users/<login>/works/drafts` (draft) means deleted: audit `work-deleted`, the work's
local cache row is removed, the cached `/users/<login>/works*` pages are invalidated
and the browser returns to `/my/works?notice=work_deleted`. Anything else is a typed
failure with a notice and an audit row, and it is never retried. The `delete_work`
governor budget is 5 per hour.

**Navigation and gate.** The signed-in header menu and the dashboard sidebar gained
Writing entries (My Works and My Drafts need a session; Post a new work and the
publish/delete actions also need `AO3I_WRITE_ENABLED=true`). Tests:
`tests/account_works.rs` (5: lists render AO3 data, publish confirms + PUTs once +
opens + audits, AO3 publish refusal surfaced and audited, delete confirms + DELETEs
once + removes the local cache + audits, destructive actions need the write switch)
plus `ao3::account_lists::works` unit tests (3); full suite 297 green. Evidence:
`notes/gates/works-manager.txt`.

### W5 status — partial (2026-11-22): kudos + reading flags

**Landed:**
- **Kudos** via the machine-readable `.js` format (`POST /kudos.js`): 201 success,
  422 `error_message` rejection; AO3's exact strings map to "given", "already
  given" (a receipt is stored) and "refused" (audited as failed, button stays).
  Public action, so it goes through the same confirmation page as subscriptions.
- **Reading flags** (`PATCH /works/:id/mark_for_later|mark_as_read`) with the CSRF
  token; the local `reading_flags` mirror drives the reader state chips.
- Local receipts: `kudos_sent` and `reading_flags` tables (migration 0005) keep the
  UI honest and prevent duplicate public actions without relying on upstream state.
- Tests: 126 total; new `tests/writes_kudos_reading.rs` (happy path, AO3-side
  duplicate, policy refusal, CSRF failure) plus a shared write-test harness.
  Evidence: `notes/gates/w5.txt`.

**Deferred with reason:** bookmarks need a `pseud_id` (the user's pseud list must be
fetched/parsed first); it lands after W6 comments.

### W4 status — complete (2026-11-22): write spine + subscriptions

**Landed:**
- Write safety spine: `write_intent`/`write_audit` ledger (intent committed before
  the request, audit after, no payloads or secrets), confirmation page
  (`/write/preview` → `POST ... confirm=yes`), local subscription ledger as the
  duplicate pre-check (AO3 has no uniqueness constraint), `AO3I_WRITE_ENABLED`
  master switch (default off).
- Governor (`ao3/governor.rs`): one write at a time, spacing floor, per-action
  budgets (comments 3/900 s, kudos 10/min, subscriptions 20/h), Retry-After parking.
- First write action: **subscriptions** (`POST /users/:login/subscriptions.json`,
  `DELETE .../:id.json`) with typed 200/201/422/302/429/403 classification; reader
  shows Subscribe/Unsubscribe; `/audit` lists the trail.
- Tests: 121 total, `tests/writes.rs` (happy path + duplicate + unsubscribe +
  validation + kill switch). Evidence: `notes/gates/w4.txt`.

**Remaining:** kudos, reading flags/bookmarks (W5); comments + failure injection +
operator docs (W6). W3 leftovers still open: password-at-rest/auto re-login,
scroll anchor.

### W3 status — partial (2026-11-22): session + progress

**Landed:**
- AO3 session in the app: `/session` page with cookie import (default), optional
  password login, verify-now, logout, adult-content opt-in; the jar is persisted
  **encrypted** (AES-256-GCM, `sessions` table) and restored on startup without a
  network probe (`AccountSession::restore`); every upstream read now goes through
  that session's client.
- Login outcomes surfaced: rejection, CSRF expiry, pwned-password redirect and
  canonical-login recovery (from W1) are mapped to human messages.
- Adult interstitial and login gates are recognised and explained instead of
  being reported as parser drift.
- Chapter-anchored reading progress: saved from the reader, shown on the start
  page as "continue ch. N", offered as a resume banner when reading elsewhere.
- Tests: 106 total; new `tests/session_and_progress.rs` (persist/use session,
  progress/resume, adult gate). Evidence: `notes/gates/w3.txt`.
- **Local search**: FTS5 index over cached works (title, authors, tags) kept in
  sync by triggers; `/search` page; input is quoted token-wise so it is data,
  never FTS syntax. `tests/search_and_refresh.rs`, `notes/gates/w3b.txt`.
- **Conditional refresh**: "Refresh from AO3" compares the cached `content_hash`;
  unchanged → cache kept and `fetched_at` moves with a "no changes" notice;
  changed → upsert + the FTS triggers follow. Reader shows the check time and
  labels copies older than 7 days as stale.
- Tests: 113 total. Evidence: `notes/gates/w3b.txt`.

**Remaining W3:** none blocking — password custody is closed by
[ADR-0006](adr/0006-no-password-custody.md); scroll/block anchoring is a documented
I7 refinement (chapter anchor is the v0 contract); series follow/scheduled refresh is
out of v0 scope (ADR-0003).

### W2 / M1 status — complete (2026-11-22)

**Landed:** work-page parser (`src/ao3/work.rs`, selectors traced to the pinned clone), SQLite
cache (`db.rs`, `store.rs`, one migrator, WAL), server-rendered app (`render.rs`, `server.rs`,
`bin/ao3i-server.rs`), cache-first open, sanitized chapter iframes with CSP. The application
**opens and reads a work end to end** against a mock upstream + checked-in fixture.
**Evidence:** `notes/gates/m1.txt` — 100 tests, clippy/fmt clean, real binary serves `/`
and `/health`, cached re-open makes no upstream request.

**Known limits of this slice (next milestones):** adult-content interstitial and restricted
works need the session (W2 session wiring); progress anchors (W3); search/offline index (W3);
no write actions yet (W3+). The live site answered 429 from this environment, so parsing is
fixture-driven; a sandbox run is the acceptance step (plan §9 tier 3).

### W1 status — complete (2026-11-22)

**Landed:** typed `Config` (base URL, data dir, key file, UA, write switch, request
spacing), AES-256-GCM secret store with purpose AAD and validated key directory, cookie
jar (parse/expiry/completeness + encryption primitives, values wiped), HTTP boundary (no redirect
following, streaming body cap, spacing floor, CSRF-carrying DELETE, redacted `Debug`),
session state machine (import, password login, token, probe, logout, kill), mock-upstream
integration tests. **Evidence:** `notes/gates/w1.txt` — 78 tests (46 unit + 7 client +
25 session), clippy `-D warnings`,
fmt, route gate 41/41, `nix build` green.

**Deferred, with the milestone that owns it:** password at rest + three-failure parking
(W3, needs persistence); liveness cache and per-action governor (W3); identity marker on
the probe response (W2, needs the subscriptions-page parser); `Referer` on writes (W3);
intent/audit tables (W3).

W0 is complete with this document set.

---

## 11. Open questions

1. Cookie import UX: do we ask for `_otwarchive_session` only and derive
   `user_credentials` (it is set by the app on any logged-in response, so the next request
   would set it), or require both up front? *Recommendation: require the session cookie,
   and let the first probe establish `user_credentials`; if the probe lands on
   `/lost_cookie`, ask for the second cookie.*
2. Should the local account and the AO3 account be 1:1, or can one local account hold
   several AO3 identities? *Recommendation: 1:1 in v1; the data model already keys by
   account.*
3. Do we expose "unsubscribe all"? The API supports it, but it is a destructive bulk
   action. *Recommendation: no; one at a time, with a count.*
4. Is the audit log exportable (CSV) for the operator? *Recommendation: yes, later; it is
   the compliance story.*
