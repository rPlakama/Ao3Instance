# AO3Instance — Init Audit (v0)

**Status:** plan-only. No `src/` exists. This document decides *what to build and in what
order* before the first crate does.
**Baseline:** commit `87b1f5f` (`init`), tree = `README.md` only (1 tracked file, 0 code
lines).
**Reference canon:** `/home/rplakama/Projects/musika` — 188 Rust files, 92,319 LOC
(`cat $(find src -name '*.rs') | wc -l`), 10 migration sets, 61 docs/notes markdown
files, 7 gate scripts, 5 process skills.
**Date:** 2026-11-22. **Network:** unavailable this session (search backend returned
HTTP 402), so every claim about AO3's HTML, endpoints or policy is tagged **VERIFY**
with the exact check to run — do not build on one.

> **2026-11-22 update — write-back approved.** The owner reversed the read-only
> recommendation (§0.8, D3/D4). Those decisions, and D7, are **superseded**. The AO3
> internals now live in [`ao3-internals.md`](ao3-internals.md), the action contracts in
> [`ao3-write-api.md`](ao3-write-api.md), the security bar in
> [`security-standards.md`](security-standards.md), and the build plan in
> [`write-client-plan.md`](write-client-plan.md). The otwarchive clone is pinned at
> `452391b` and gated by `scripts/check-ao3-routes.sh`.

Read order if you pick this up cold: §0 verdict → §2 decisions → §6 invariants → §9 M0.
Everything else is support.

---

## 0. Verdict

1. The README describes a **reader-first AO3 mirror**: server-side cache, offline
   survival, EPUB as the local artifact. That is a coherent, buildable product — and it
   is not what "Full IMPL of Ao3" says. One of those two sentences must die (§1.3).
2. The single best architectural decision available is **EPUB-first ingestion**: AO3
   already exports EPUB/MOBI/PDF/HTML per work. Ingest the sanctioned artifact, scrape
   HTML only for discovery and metadata. This shrinks the parser surface, improves
   offline fidelity, and sidesteps most of the fragility the alternative invites. (§5.3)
3. The single biggest project risk is **not technical**: crawling AO3 without a written
   policy decision is a project-ending legal/ethical risk. That decision is D2 and it
   gates all fetcher code. (§2, R1)
4. "SQLX as DB" is a category error — SQLx is a client library. The engine decision
   (SQLite vs Postgres) changes the whole data layer and is D1. (§2)
5. Musika already solved most of the shape: headless authority + disposable client,
   one ledger for background work, one monotonic revision gating every derived rebuild,
   health-probe-driven offline mode, allowlisted wipes, executable gates. Inherit the
   *contracts*, not the code. (§3)
6. Musika also documents its own drift: god modules, four search pipelines, ten migration
   dirs of which two are live, docs whose counts rotted. Each has a named guardrail here.
   (§4)
7. At init, the deliverable is **decisions + invariants + gates**, not features. M0 is
   green when `nix build` + clippy + an empty test suite pass and D1–D4 are answered.
   (§9)
8. v0 should be **read-only against AO3**. Kudos/comments/bookmarks are write actions
   against a live session and multiply the design surface for a small share of the value. (§12)

---

## 1. Baseline

### 1.1 What exists

| dimension | state | evidence |
| --- | --- | --- |
| tracked files | 1 | `git ls-files` → `README.md` (this audit is the second file, untracked) |
| code | 0 LOC | — |
| build system | absent (README *intends* `flake.nix`) | `README.md:18` |
| CI | absent | no `.github/` |
| license | absent | no `LICENSE` |
| tests | absent | — |
| docs | this file | `docs/` |
| dependency policy | "strict Clippy rules" | `README.md:18` |

The repo is a **statement of intent**, not a codebase. That is the right time to audit:
every decision below is currently free.

### 1.2 README forensics

Each claim is a commitment. Verdict = *clear* (buildable as written) / *ambiguous*
(two incompatible readings) / *unbounded* (no acceptance test can ever pass).

| README claim | line | Engineering commitment | Verdict |
| --- | --- | --- | --- |
| "webapplication … accessed thought login" | 1 | server + browser client, authenticated | **ambiguous**: SSR vs SPA undecided (D5) |
| "frontend for AO3" | 1 | a UI over an upstream that is not ours | **ambiguous**: read-only mirror vs write-back (D4) |
| "local caching for fanfics (in server)" | 1 | server-side artifact cache | clear |
| "saving scroll position of they" | 1 | durable reading progress | clear; must be anchor-based (§6 I7) |
| "even standing after the AO3 is down" | 1 | offline degradation | clear; needs a capability matrix (§5.6) |
| "Accounts" | 5 | local identity | **ambiguous**: 1 user vs many (D1) |
| "Local storage of fanfics that user acess" | 7 | per-user library | clear — and it duplicates line 1's cache; one store, not two (§6 I2) |
| "Full IMPL of Ao3" | 8 | ? | **unbounded** — reject as written (§1.3) |
| "Search <--> AO3 (server)" | 9 | remote + local search | **ambiguous**: which is authoritative, and how are they merged (D6) |
| "Full Reader, EPUB (Main) <-- as caching" | 10 | EPUB is the canonical local artifact | **clear, and load-bearing** |
| "In Rust" | 11 | backend language | clear |
| "SQLX as DB" | 12 | persistence | **category error**: SQLx is a client library (D1) |
| arch ref: otwcode/otwarchive | 15 | domain model source | adopt as a **data dictionary**, never as code |
| arch ref: Kareadita/Kavita | 16 | reading UX + library model | adopt as **UX reference**; it is C#/Angular, do not port |
| flake.nix + Kunobi Kache | 18 | reproducible builds | adopt; Kache must stay optional (§8 G1) |
| "strict Clippy rules" | 18 | lint policy | adopt musika's listed denies verbatim (§8 G2) |

### 1.3 The three unbounded claims

**"Full IMPL of Ao3."** otwarchive is a full Rails monolith: works, chapters, series,
collections, challenges, gifts, bookmarks, kudos, comments, subscriptions, inbox, user
profiles, pseuds, and the tag-wrangling subsystem (canonical tags, synonyms, filters).
"Implementing all of it" is a multi-year content-management project, and none of it is
needed to read a fic. **Replacement scope (D3):** a read-only surface list — *find a work,
open it, read it, keep it, find it again offline*.

**"SQLX as DB."** Choose the engine (D1). The answer changes migrations, transactions,
search, concurrency and deployment.

**"Search <--> AO3."** Decide whether the local index is a cache of upstream results or an
independent index over cached text (D6). The two have different freshness, ranking and
failure semantics.

---

## 2. Decisions that must land before feature code (ADR seeds)

Every one of these is cheaper now than after the first migration. Recommendation is
given; the owner decides.

| id | decision | options | recommendation | why it is blocking |
| --- | --- | --- | --- | --- |
| **D1** | storage engine | (a) SQLite single file; (b) Postgres | **(a)** if ≤ a handful of humans log in; **(b)** the moment signup is public or >5 concurrent readers | changes migrations, search, tx boundaries, deploy, backups |
| **D2** | upstream access policy | (a) official exports only; (b) exports + HTML scrape; (c) bulk crawl | **(b)**, *user-initiated, single-work, rate-limited*, after quoting AO3's ToS/robots clauses into the ADR | gates every fetcher line; a wrong answer is unrecoverable |
| **D3** | product scope | (a) reader mirror; (b) + write-back (kudos/bookmarks/comments); (c) + collection/series management | ~~**(a)** for v0~~ **superseded** by [ADR-0003](adr/0003-v1-scope.md): write-back is the point, scoped to six verbs | decides the DTO and auth surfaces |
| **D4** | write-back | yes/no | ~~**no** in v0~~ **superseded** by [ADR-0003](adr/0003-v1-scope.md): yes, v1 | doubles auth risk and test matrix |
| **D5** | client stack | (a) server-rendered HTML + progressive enhancement; (b) SPA; (c) hybrid | **(a)** | decides the API, the reader sandbox, and the offline story |
| **D6** | search authority | (a) local FTS over cached text; (b) upstream search proxy; (c) merged with provenance | **(c)**, with provenance mandatory and local preferred when offline | determines index size, ranking, and the empty-result semantics |
| **D7** | ingest artifact | (a) EPUB download endpoint; (b) scrape chapter HTML; (c) both | ~~**(a)** primary~~ **superseded** by [ADR-0004](adr/0004-artifact-source.md): `/downloads/` is robots-disallowed, so (b) builds the EPUB locally | determines parser surface and fidelity |
| **D8** | multiprocess shape | (a) one binary; (b) server binary + admin CLI | **(a)** now; add a thin `ao3i-cli` only when it earns its keep — it speaks the HTTP API, not the DB | copies musika's "one library, two 12-line bins" shape |

D1–D4 are P0. D5–D8 are P1. Write each as `docs/adr/NNNN-*.md` with: context, options,
decision, consequences, and the test that proves the decision holds.

---

## 3. The musika canon — 12 patterns worth inheriting

Adapted from a 92k-LOC project that shares this project's constraints (Rust, SQLx,
offline-first, one authoritative process, a remote server it does not control). Each row
names the musika evidence and the AO3Instance translation.

| # | pattern | musika evidence | AO3Instance translation | verdict |
| --- | --- | --- | --- | --- |
| C1 | **Authority in one process; the client is disposable** | `docs/Daemon.md:5-13,165-178`; UI never opens a DB (`src/ui/daemon_client.rs:17-32`) | the server owns DB, blobs, upstream session, job ledger; the browser owns only view state | **adopt** |
| C2 | **Latched snapshot + revisioned deltas; reattach = replay** | `src/daemon.rs:1445-1494`; `QueueDelta` `src/ipc_protocol.rs:693-716`; gap → resync | bootstrap endpoint returns `{state, rev}` under the same gate as the event stream; `?since=rev`; `409 ResyncRequired` on gap | **adopt** |
| C3 | **One monotonic revision gates every derived rebuild** | `src/library/sync.rs:326-355`; readers compare `*_rev` before rebuilding (`src/search.rs:101-110`) | `content_rev` bumped *inside* the ingest transaction; EPUB/sanitizer/FTS rebuild only when it moved | **adopt — highest-value single idea** |
| C4 | **Never destructive on a partial fetch** | `src/library/sync.rs:619-626` staged TEMP ids + set-difference deletes; `ensure_complete` (`src/index.rs:513-523`) | stage the manifest of what was seen; delete only rows absent from a *complete* manifest; partial fetch = no delete, no publish | **adopt** |
| C5 | **Ledger of intent, not in-memory jobs** | `download` state machine `src/download.rs:37-84`; `queued→active→done/error`; crash recovery `src/media.rs:529-541` | `job` rows with lease/deadline/attempts; `active` rows older than the lease become `error: interrupted` at boot | **adopt** |
| C6 | **Invariants enforced inside the SQL statement** | no-downgrade upsert `src/download.rs:66-74`; `rows_affected()==0 → bail` `src/media.rs:128-159` | e.g. never replace a user-kept blob with a cache copy; never lower `content_hash` to `stale` without a new fetch | **adopt** |
| C7 | **Single-flight claims with RAII release** | `src/covers.rs:60-118`; `src/media_actions.rs:26-69` | N users opening the same work = 1 fetch; guard releases on cancel/panic | **adopt** |
| C8 | **Content-addressed blobs + `.part` + minimum age on sweeps** | `src/covers.rs:394-399`; `src/media.rs:168-180,610-654` | blobs keyed `sha256`; write `.part`, rename; never sweep temp files younger than 10 min | **adopt** |
| C9 | **Health probe drives a first-class offline mode** | `src/daemon.rs:67-89` (5 s probe, 4 s timeout), `src/session.rs:29-73` (atomic + watch) | `upstream_reachable()` with timeout; watch channel wakes parked work; every read surface has a cached answer or a typed miss | **adopt** |
| C10 | **Allowlisted wipe, dry-runnable, pools closed first** | `src/db.rs:432-488` (`WIPE_DB_FILES`, dirs, sidecars; closes pools because "deleted inodes stay alive") | `wipe --dry-run` default; explicit target list; refuse to touch a path that is not under `data_root` | **adopt** |
| C11 | **Gates are executable, not prose** | `.github/workflows/ci.yml` (clippy `-D warnings`, default + `--no-default-features` + feature matrix); `notes/gates/summary.txt` | same matrix, plus the AO3-specific gates in §8 | **adopt** |
| C12 | **Evidence-first audit docs with a verification pass** | `docs/mds/rust-arch-reference.md:23-70` (ground rules); `docs/mds/audit/verification.md` (PASS table, `sha256`, "no src edits", unverifiable list) | every plan doc ends with "how to check this"; an independent verifier re-counts before merge | **adopt** |

Two more that are cheap and load-bearing, worth calling out:

- **Test seams are parameters, not traits.** Musika has exactly two traits, neither for
  testability; instead `*_in_dir(dir)` / `*_with(pool, …)` inject the boundary
  (`src/media.rs:255-260`, `src/lyrics.rs:416`). Do the same for the gateway: one
  concrete client + one fixture implementation, no `trait Http`.
- **Docs that state counts must be generated or asserted, or they rot.** Musika's own
  `docs/Daemon.md:98` says protocol "v2" while `VERSION = 6`
  (`src/ipc_protocol.rs:52`); `docs/Daemon.md:125` says "38 `event_json` bridge events"
  while `Event` has 44 variants (`src/bridge.rs:48-244`, counted
  `awk 'NR>=48 && NR<=244 && /^    [A-Z]/'`). The command registry avoids this with a
  docs→registry test (`src/test/command_registry.rs:177-201`). Copy the test, not the doc.

---

## 4. The musika anti-canon — 9 failure modes and our guardrail

Musika audits itself (`docs/mds/rust-arch-reference.md`, `Issues.md`, `docs/mds/audit/*`).
These are the traps a project of this shape falls into; each gets a mechanical guardrail,
not a good intention.

| # | failure mode | musika evidence | guardrail here |
| --- | --- | --- | --- |
| A1 | **God module** | `qml.rs` 3,389 LOC / 239 fns; `index.rs` a "sync hub" with 37 `crate::` refs across 14 modules (`rust-arch-reference.md:73-97`) | per-file LOC/fn budget in CI (`G9`); no module imported by >8 modules |
| A2 | **Parallel pipelines per domain** | 5 asset domains each with fetch/cache/DB/purge/emit; 4 search paths (`:84-92`) | one ingest pipeline with per-type *policy*, not per-type copies; rule of two (inherited skill) |
| A3 | **N payload builders / JSON round-trips** | 36 `*_json`/`*_payload` builders (`:87`) | one DTO boundary: parser → domain → serde. No layer re-serializes another's JSON. |
| A4 | **State projected into 4 sinks** | playback state duplicated to UI, MPRIS, `status.rs`, player (`:93-94`) | one authoritative row/struct; every other view is derived at read time |
| A5 | **Dual paths kept alive** | QML + GPUI for a release window; legacy DB aliases; its own audit counted 9 of 11 migration dirs as test-only (`:43-44,171-172`) and the tree still carries 10 dirs with 2 live (`src/db.rs:106,116`) | no second implementation of a page; no legacy schema shim without a deletion date |
| A6 | **Migration sprawl** | 10 sets, 2 live (`src/db.rs:106,116`); unreferenced dirs; stale comments citing a dir that never existed | one `migrations/` dir, one migrator, filenames `NNNN_<context>_<name>.sql`; `G6` fails on edits to applied files |
| A7 | **Unbounded queues feeding a bounded one** | outbox is bounded at 128 (`src/daemon/clients.rs:30`) but producers are `tokio::sync::mpsc::unbounded_channel` (`src/daemon.rs:189,199,249`; `src/ipc.rs:276`) | every queue in the push path has a bound; classify each message as *coalesce / drop / block* |
| A8 | **Timestamp encoding drift** | three encodings (TEXT datetime, INTEGER epoch, TEXT server time) in one schema; consumers parse strings | one type: `INTEGER` epoch seconds (or `i64` millis) everywhere; convert at the edge only |
| A9 | **Config sprawl** | ~45 `MUSIKA_*` env vars, many single-use | one typed `Config` struct, `AO3I_*` prefix; grep gate (planned W6): `std::env` outside `config.rs` is test-only |

The meta-lesson from musika's own verification doc: **counts are claims**. Its audit was
corrected mid-flight (`Dbs` = "12 fields / 8 aliases / 4 pools / 7 connections",
`rust-arch-reference.md:159`). Three of my probes gave 92,319 / 92,299 / 91,637 LOC for
the same tree. Every number in a plan gets a command and a date, or it is deleted.

---

## 5. Target design v0

### 5.1 Shape

Musika: *"a headless core plus a disposable GPU window."* Ours:

```
ao3i-server            authoritative process: DB, blob store, upstream session,
                       ingest ledger, search index, progress, HTTP/SSE surface
  └─ browser tab       disposable: renders HTML/state, owns only transient view state
```

The server may be restarted mid-read; the tab re-bootstraps. The tab may be closed
mid-fetch; the fetch continues. No state lives only in the tab.

### 5.2 Module map (one crate, two bins)

Follow musika's proven shape: a **library** that holds everything, plus 12-line bins, so
all logic is testable outside `main`. Split into workspace crates only when `G9` trips —
not before (premature abstraction is explicitly forbidden by the inherited `inherit`
skill).

```
src/lib.rs
  consts.rs        limits, timeouts, politeness defaults, TTLs (one place)
  config.rs        typed Config; the only reader of AO3I_* env
  db.rs            pools + pragmas + migrator + allowlisted wipe
  model.rs         ids, domain types, DTOs
  upstream/        NOTHING else in the tree may do HTTP
    http.rs        client, timeouts, retry ladder, UA string
    limiter.rs     global token bucket + circuit breaker
    parse/         work page, tag page, search page (one module per surface + fixtures)
    fixtures/      recorded HTML/EPUB, checked in
  ingest/
    mod.rs         worker loop: claim → fetch → parse → persist → derive → publish
    job.rs         ledger state machine
    work.rs        metadata + official EPUB
    epub.rs        validate, split chapters, extract assets
    sanitize.rs    XHTML allowlist → reader document
  store/
    blobs.rs       content-addressed store, refcount, gc
    catalog.rs     works/chapters/series/tags
    progress.rs    anchors
  search/          local FTS + remote merge, provenance
  web/             routes (auth, search, work, reader, library, api), templates, assets
src/bin/ao3i-server.rs
```

### 5.3 The spine: EPUB-first ingestion

> **Superseded by [ADR-0004](adr/0004-artifact-source.md).** `/downloads/` is disallowed by
> robots.txt; the EPUB is now built locally from the work page. The pipeline shape below
> still holds — only the source of the bytes changes.

```
id/url ─► job(queued) ─► limiter ─► GET work page (metadata only)
                                    │
                                    ├─► GET /downloads/:id/epub   [official export]
                                    │        │
                                    │        ├─ validate zip/OPF/spine
                                    │        ├─ split chapters → sanitized XHTML
                                    │        ├─ extract images → content-addressed blobs
                                    │        └─ sha256(epub) = artifact key
                                    ▼
                             one transaction: upsert work + chapters + job=done + bump content_rev
                                    ▼
                             derive: FTS row, cover, reader doc — all gated on content_rev
```

Why this is the design, not a shortcut:

- **It is the sanctioned interface.** AO3 ships EPUB/MOBI/PDF/HTML from every work page;
  using it is closer to "download for offline reading" than scraping chapter HTML is.
  **VERIFY:** confirm the exact path (`/downloads/:id/epub`?) and that it is not
  `robots`-disallowed, in a real browser before writing `upstream/`.
- **It collapses the parser.** One EPUB reader replaces per-chapter HTML extraction, and
  chapter boundaries, titles, notes and order come from the OPF spine.
- **It is the cache the README already wants** ("EPUB (Main) <-- as caching"). The blob
  *is* the offline artifact; everything else is derived.
- **It makes re-fetch cheap to detect.** `sha256(epub)` changes iff the work changed, so
  refresh is conditional on one hash, not on timestamps AO3 may not expose.

Metadata still comes from the work page: rating, warnings, categories, fandoms,
relationships, characters, tags, language, word count, kudos, series. Only that page is
scraped, and only into a typed struct with a fixture test.

**Fallback policy:** if the download endpoint fails or is absent for a work, the job ends
`error: no_export` and *nothing* is written. Do not silently fall back to scraping
chapter HTML in v0 — that is a separate decision with a separate ADR.

### 5.4 Data model (sketch)

```
work(id PK, ao3_id UNIQUE, title, summary, rating, language, word_count,
     series_id, series_index, updated_at, fetched_at, content_hash,
     state TEXT CHECK(state IN ('fresh','stale','error')), content_rev)

chapter(work_id FK, idx, title, xhtml_blob, source_hash, PRIMARY KEY(work_id, idx))
tag(id PK, name UNIQUE, kind, canonical_id NULL)          -- kinds: fandom, rel, char, freeform, warning, category
work_tag(work_id, tag_id, PRIMARY KEY(work_id, tag_id))
series(id PK, name, ao3_id)
blob(sha256 PK, size, mime, created_at, refcount)
work_blob(work_id, sha256, role, PRIMARY KEY(work_id, role))

job(id PK, kind, target, state CHECK(state IN ('queued','active','done','error')),
    attempts, not_before, lease_until, last_error, created_at, updated_at)

account(id PK, name UNIQUE, pass_hash, created_at)
upstream_session(id PK, account_id, cookie_jar_encrypted, csrf_token, updated_at)

progress(account_id, work_id, chapter_idx, block_idx, char_offset, pct,
         client_seq, updated_at, PRIMARY KEY(account_id, work_id))
```

Rules: ids are opaque `TEXT` (AO3 ids as strings, never `i64` arithmetic); every
timestamp `INTEGER` epoch seconds; no soft-delete columns — use nullable lifecycle
timestamps only where the lifecycle is real (`deleted_at` on `account` only); derived
tables never store what can be recomputed.

### 5.5 Storage tiers and keys

| tier | lives in | deletion policy |
| --- | --- | --- |
| canonical artifact (EPUB bytes) | `data/blobs/<aa>/<bb>/<sha256>` | explicit user delete, or GC when `refcount = 0` |
| chapter XHTML (sanitized) | same store, content-addressed | rebuilt from EPUB; deletable without loss |
| metadata | DB | rebuildable by re-fetch |
| derived (FTS, cover) | DB / store | purge on `content_rev` mismatch |
| upstream session | DB, encrypted | wipe on logout |

One store, not two: "server cache" (README:1) and "local storage per user" (README:7) are
the same bytes with different reference counts. Per-user library = a link row, not a copy.

### 5.6 Offline as a capability matrix, not a mode

```
capability            online                  upstream down
search                local + remote          local only, badge "cached results"
work page             fetch if missing        last snapshot + fetched_at
reader                EPUB from cache         EPUB from cache, identical path
progress write        our server              our server (unchanged; DB is authoritative)
new work              fetch                   typed miss: "not cached yet"
```

Contract: **no view may hang on upstream**. Every read route resolves from the DB, or
returns a typed "not cached" state with the reason. That is what makes "standing after
AO3 is down" a property rather than a slogan.

### 5.7 Reader and progress

- Reader renders **sanitized XHTML inside a sandboxed iframe** with a strict CSP:
  `default-src 'none'; img-src 'self'; style-src 'unsafe-inline'` (work skins need
  inline CSS), no scripts, no forms, no external loads. Untrusted author CSS never enters
  the app document. This is non-negotiable: AO3 works can carry custom skins.
- Progress is an **anchor**, not a pixel offset: `(chapter_idx, block_idx, char_offset)`
  plus a `pct` fallback. Pixels break on font size, viewport and sanitizer version.
- Write path: debounced (~2 s) on scroll idle + on `visibilitychange`; `client_seq`
  monotonic; server is authoritative; last-writer-wins per `(account, work)`, with `pct`
  used to reconcile when the block moved (content updated).
- "Resume" is a first-class route: `/read/:work?resume=1` restores and highlights the last
  block; a mismatch falls back to `pct` and says so.

### 5.8 Search

Three layers, one result type with provenance (`local` | `remote` | `both`):

1. **Local FTS** over title, summary, tags, author (body optional, see D6) — FTS5 if
   SQLite, `tsvector` + GIN if Postgres. Never interpolate user input into FTS syntax
   (musika quotes every token: `src/search.rs:248-254`).
2. **Upstream search proxy** — one module, one fixture test, one kill switch. Any parser
   change breaks only this module.
3. **Merge** — dedupe by `ao3_id`, local wins for display of cached works, badge the
   provenance, and never show a remote-only result as readable offline.

Empty-result semantics: "no matches" and "upstream unavailable" are different states and
must look different.

### 5.9 Identity: two planes that never merge

- **Local account** (`account`) — authenticates to *this instance*.
- **AO3 session** (`upstream_session`) — authenticates *the instance to AO3*. Encrypted at
  rest, never logged, never emitted to the client, wiped on logout, and scoped as an
  explicit decision (D2): one shared instance session (v0, single-operator) vs per-user
  (only if D1 = multi-user).

This split is musika's `accounts.rs` vs `session.rs` distinction and it is the difference
between a clean design and a credentials-leak class bug.

### 5.10 Upstream gateway contract

One type, one place:

```
probe()                      -> Reachable | Degraded | Down        (timeout-bounded)
work_metadata(id)            -> WorkMeta
work_epub(id)                -> bytes (sha256 computed by caller)
search(query, filters, page) -> RemotePage
```

Rules: every method goes through the limiter; every method has a timeout; every parser has
a checked-in fixture; no caller builds a URL. If a feature needs a new request shape, it
adds a method here.

---

## 6. Invariants

These are the sentences a reviewer can fail a PR against. Each names its enforcement.

| id | invariant | enforced by |
| --- | --- | --- |
| **I1** | AO3 is the source of truth for remote facts; the DB is a cache with provenance (`source_url`, `fetched_at`, `content_hash`) | schema `NOT NULL` on all three; migration test (`G6`) |
| **I2** | One blob per content; users reference it, never copy it | `blob.sha256` PK + `work_blob` link; GC only at `refcount = 0` |
| **I3** | Every read path answers locally or returns a typed miss; no route awaits upstream | `G8` offline test kills the gateway and walks every route |
| **I4** | Every background write is a ledger row, idempotent and resumable; no in-memory job queue | `job` table + lease; crash test restarts mid-job |
| **I5** | All outbound traffic passes through the gateway and its budget | `G5` greps for `reqwest`/`ureq` outside `upstream/` |
| **I6** | Local identity and upstream session are separate stores; upstream secrets are encrypted at rest and never serialized outward | schema + DTO test; `grep` gate for the cookie field in responses |
| **I7** | Progress is content-anchored, not pixel-based | `progress` columns; reflow test changes font size and asserts resume position |
| **I8** | One DTO boundary; no layer re-serializes another's JSON | review + `G9` (no `serde_json::Value` outside `web/`) |
| **I9** | Wipes are allowlisted, dry-runnable by default, and never follow a computed path | `wipe` implementation + test that a canary sibling file survives |
| **I10** | Untrusted content (author HTML/CSS) is rendered only in a locked-down document | CSP header test + sanitizer fixtures |
| **I11** | No destructive delete on a partial fetch; deletion requires a complete manifest | `G8`-style test: abort mid-manifest, assert zero deletions |
| **I12** | Derived data (FTS, cover, reader doc) is keyed to `content_rev` and rebuilt, never patched | revision gate in `derive`; test asserts a stale rev triggers rebuild |

---

## 7. Risk register

Likelihood/impact: L/M/H. Owner is a role (`owner` = the project owner).

| id | risk | P | L | I | mitigation | trigger |
| --- | --- | --- | --- | --- | --- | --- |
| **R1** | AO3 ToS/robots forbids automated access, or the operator objects to the traffic | **P0** | M | H | D2: quote the clauses first; user-initiated fetches only; official export endpoints; conservative rate (§8 G5); kill switch; document the personal-use assumption | before first fetch |
| **R2** | Upstream HTML/EPUB shape changes and ingestion silently breaks | **P0** | H | M | fixture per surface; versioned parser; `error: parse_drift` (never a half-work); `check-fixtures` in CI | first scrape |
| **R3** | Engine choice wrong (SQLite → Postgres migration later) | **P0** | M | H | D1 answered with the user-count test; if unsure, SQLite + repository-shaped SQL confined to `store/` | M0 |
| **R4** | Restricted works require login; session handling leaks or breaks | **P1** | M | H | encrypted session, explicit scope, no cookie logging, "login required" typed state | M2 |
| **R5** | "Full IMPL" scope creep swallows the project | **P1** | H | H | D3 surface list; every new surface needs an ADR | continuous |
| **R6** | Untrusted author CSS/HTML → XSS | **P1** | M | H | sandboxed iframe + CSP + allowlist sanitizer + fixtures (I10) | M1 |
| **R7** | Storage growth (EPUB + images × works) | **P1** | M | M | content addressing, refcount GC, per-user quota, size report in admin | M3 |
| **R8** | Progress conflicts across devices / after work updates | **P1** | M | M | `client_seq`, `pct` fallback, explicit "content changed" notice | M2 |
| **R9** | Politeness collapse under multi-user demand (N users → N fetches) | **P2** | M | M | single-flight claims (C7), global budget, queue fairness by age | M3 |
| **R10** | Legal: cached works served to other users = redistribution | **P0-if-multi-user** | ? | H | D1/D2: single-operator instance keeps "personal copy"; a public instance needs a counsel-level answer first | before signup opens |
| **R11** | Docs drift (counts, routes, DTOs) | **P2** | H | L | generate or assert (C12); `G7` | M1 onward |
| **R12** | Build depends on Kache endpoint | **P2** | L | L | Kache is a substituter only; prove `nix build` with it disabled | M0 |
| **R13** | `/downloads/` and search crawls are robots-disallowed | **P0** | M | M | [ADR-0002](adr/0002-access-posture.md)/[ADR-0004](adr/0004-artifact-source.md): user-initiated only, local EPUB build, grep gate on `/downloads/` | before M1 fetch |
| **R14** | Cloudflare bot scoring / challenge; Akismet comment rejection | **P1** | M | M | `BlockedByEdge` and `SpamRejected` outcomes; honest UA; stop-and-alert, never hammer | first write |
| **R15** | Duplicate subscriptions — no DB uniqueness, no model validation | **P1** | H | M | pre-check the subscription index; never blind-retry ([`ao3-write-api.md`](ao3-write-api.md) §4.3) | W3 |
| **R16** | Incomplete cookie jar triggers forced logout (`/lost_cookie`) | **P1** | M | M | persist the whole jar; probe before marking a session active ([`ao3-internals.md`](ao3-internals.md) §4.2) | W1 |

---

## 8. Gates to install at init

Non-negotiable, executable, in CI from M0. Thresholds are starting points; they only
ratchet down.

| id | gate | command / check | threshold |
| --- | --- | --- | --- |
| **G1** | Build | `nix build .#default` | green; also green with Kache disabled |
| **G2** | Lint | `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` | musika's list: `unwrap_used` deny, `expect_used` warn, `undocumented_unsafe_blocks` deny, `allow_attributes_without_reason` deny, `todo`/`unimplemented`/`dbg_macro` deny, `match_wildcard_for_single_variants` deny |
| **G3** | Tests | `cargo test` on default + `--no-default-features` | count never decreases |
| **G4** | Fixtures | `scripts/check-fixtures.sh` — every parser surface has ≥1 checked-in fixture | a parser change without a fixture fails |
| **G5** | Upstream confinement | `scripts/check-upstream.sh` — `reqwest`, `ureq`, `hyper` only under `src/upstream/`; default rate constants unchanged unless the ADR says so | 0 violations |
| **G6** | Migrations | `sqlx migrate info` clean; checksums unchanged for applied files; exactly one migrator dir | fails on edits, not just on errors |
| **G7** | Docs parity | test asserts every route + every env var appears in docs, and every doc count is generated/asserted | 0 drift |
| **G8** | Offline walk | integration test: gateway returns `Down`, walk every read route | 0 hangs, 0 5xx |
| **G9** | Modularity | adapted `scripts/modularity.sh`: per-file LOC/fns, cross-module refs, repeated-pattern counters | file ≤ 800 LOC, ≤ 60 fns; no module referenced by >8; no `serde_json::Value` outside `web/` |
| **G10** | Security | `cargo deny`/`cargo audit`; CSP header test; sanitizer corpus; secret scan | 0 advisories, CSP present on reader/doc routes |

Musika's gate culture is the model: `notes/gates/summary.txt` records
`DEFAULT_EXIT=0 / NODEFAULT_EXIT=0 / IOURING_EXIT=0` — a machine-readable record that the
gates actually ran, not a claim that they would pass.

---

## 9. Milestones

Each milestone has an exit test, not a feature list. Do not start M(n+1) with M(n)'s test
red.

| id | theme | contents | exit criteria |
| --- | --- | --- | --- |
| **M0** | Decisions & skeleton | D1–D4 answered as ADRs; `Cargo.toml`, `flake.nix`, `clippy.toml`, CI; empty crate builds; `docs/adr/`, `notes/gap/` seeded; LICENSE chosen | `G1`–`G3` green on an empty crate; 4 ADRs merged; no `src/` logic |
| **M1** | One work, end to end | fetch metadata + official EPUB for a URL/ID; store blob; sanitize chapters; reader route; progress write | read a real work; reload offline; second open does **zero** upstream requests; progress survives reflow; sanitizer fixtures pass |
| **M2** | Accounts & library | local account, per-user library links, login/logout, session encryption (if needed), resume across devices | two accounts see their own libraries; logout wipes the session; no cookie appears in any response body |
| **M3** | Search | local FTS over cached works + tag filters; upstream proxy behind a switch; provenance merge | offline search returns cached hits with a badge; upstream-down returns "unavailable", not "no results" |
| **M4** | Refresh & follow | conditional re-fetch by `content_hash`; series follow; stale badges; re-derive pipeline | a changed work updates in place and keeps progress; an unchanged work costs one conditional request |
| **M5** | Offline hardening | capability matrix complete; politeness under load; GC; backup/restore; admin status | `G8` green; restart mid-ingest loses nothing; GC never removes a referenced blob |
| **M6** | Release | packaging, systemd unit, docs, perf budgets, a11y pass, security review | `nix build` from a clean store; docs parity green; a11y keyboard-only pass; release notes |

M1 is the whole product in miniature. If M1's exit criteria cannot be met, the design is
wrong and no later milestone fixes it.

---

## 10. Init backlog

| id | item | P | effort | risk | depends |
| --- | --- | --- | --- | --- | --- |
| INIT-01 | Write ADR-001 storage engine | P1 | S | low | — |
| INIT-02 | Write ADR-002 upstream access policy (quote ToS/robots) | P1 | M | low | — |
| INIT-03 | Write ADR-003 v0 surface list; kill "Full IMPL" | P1 | S | low | — |
| INIT-04 | Write ADR-004 identity planes + session scope | P1 | S | low | INIT-01 |
| INIT-05 | `flake.nix` + `clippy.toml` + CI matrix | P1 | M | low | — |
| INIT-06 | `scripts/modularity.sh` + `check-upstream.sh` + `check-fixtures.sh` | P1 | M | low | INIT-05 |
| INIT-07 | Single-migrator schema for `work`/`chapter`/`job` | P1 | M | med | INIT-01 |
| INIT-08 | Gateway trait-less client + limiter + fixture harness | P1 | M | med | INIT-02 |
| INIT-09 | EPUB parse/split/sanitize + fixtures | P1 | L | med | INIT-07 |
| INIT-10 | Blob store (content-addressed, refcount, GC) | P1 | M | med | INIT-07 |
| INIT-11 | Reader route + sandbox + CSP | P1 | M | high | INIT-09 |
| INIT-12 | Progress anchors + resume | P1 | M | med | INIT-11 |
| INIT-13 | Offline walk test harness | P1 | S | low | INIT-08 |
| INIT-14 | Docs-parity check | P2 | S | low | INIT-05 |
| INIT-15 | Backup/restore of DB + blobs | P2 | M | med | INIT-10 |
| INIT-16 | Series model + follow | P2 | L | med | INIT-07 |
| INIT-17 | Admin status page (queue depth, cache size, probe state) | P2 | S | low | INIT-08 |

---

## 11. Open questions for the owner

**Blocking (answers change the architecture):**

1. **Who logs in?** One person on a home server, a household, or the public internet?
   (D1, D2, R10 all hinge on this.)
2. **Is the upstream AO3 always the real archiveofourown.org**, or could this point at a
   self-hosted otwarchive instance you control? (The second changes R1/R2 entirely —
   scraping your own instance is a different question.)
3. **Write-back ever?** If kudos/bookmarks are on the roadmap, say so now; it changes the
   auth design even if v0 does not ship them.

**Non-blocking:**

4. Does "EPUB (Main)" mean *EPUB is what we serve to e-readers too* (an OPDS feed), or
   only the internal cache format?
5. Should the reader support work skins (custom CSS) at all, or flatten them for safety?
   (Recommended: support inline CSS in the sandbox, drop external assets.)
6. Multi-language UI? AO3 content is multilingual; the UI can stay English without loss.
7. Is a public read-only OPDS endpoint a goal? It is nearly free once blobs exist, and it
   makes the cache useful to e-readers.

---

## 12. Do-not-do list

1. **Do not write an HTTP request before D2 is answered.** No exceptions, not even a
   "quick test".
2. **Do not model otwarchive.** No challenges, gifts, collections, kudos, comments,
   inboxes in v0.
3. **Do not store four formats.** EPUB + sanitized XHTML only. MOBI/PDF are out of scope.
4. **Do not serve cached works to other humans** until R10 has an answer.
5. **Do not build an SPA and server templates "just in case."** One client, chosen.
6. **Do not add a client-side search index.** The server is authoritative; a second index
   is a second truth.
7. **Do not store upstream cookies in plaintext**, in logs, or in a response body.
8. **Do not parallelize per user request.** One global budget; queue by age.
9. **Do not use `unwrap`/`expect` on request paths.** Clippy denies it; the deny is the
   documentation.
10. **Do not create a second migration directory** because a feature "wants its own DB."
    One file, one migrator (A6).
11. **Do not let `web/routes.rs` become a god module.** Split by surface before it passes
    the `G9` budget.
12. **Do not trust AO3's HTML.** Every parse site has a fixture or it does not merge.
13. **Do not copy musika's env-var sprawl.** One typed `Config`, `AO3I_*`, read once.
14. **Do not write a doc that states a count** without generating or asserting it.

---

## 13. Verification protocol for this audit

This document is a plan; it is only worth what can be checked.

1. **Musika citations.** Every `path:line` above was read from
   `/home/rplakama/Projects/musika` at the baseline commit or spot-checked after the
   probes (`src/db.rs:101`, `src/download.rs:57`, `src/session.rs:29`,
   `src/ipc_protocol.rs:52-55`, `src/daemon/clients.rs:30`,
   `src/library/sync.rs:619`, `src/media.rs:117`, `src/ipc_command.rs:562`,
   `src/daemon.rs:67`, `src/consts.rs:20` all confirmed). Re-check with
   `sed -n 'A,Bp' <path>`; if a line no longer matches, the citation is stale, not
   "roughly right".
2. **Counts.** `188` files / `92,319` LOC measured 2026-11-22 with
   `find src -name '*.rs' | wc -l` and `cat $(find src -name '*.rs') | wc -l`.
   Two independent probes of the same tree reported 92,299 and 91,637 — counts need a
   command and a date, and this is the demonstration.
3. **AO3 claims (all tagged VERIFY).** Before any code depends on them, confirm in one
   browser session, logged in and logged out:
   - the download path and offered formats on a work page;
   - whether a restricted work changes that path;
   - the "Entire Work" parameter, if the HTML fallback is ever approved;
   - the search URL shape and which filters are expressible;
   - `robots.txt`, and the exact ToS clauses on automated access (quote them, with a
     retrieval date, into ADR-002).
   Record each result in the ADR. Do not cite this audit as evidence for an AO3 fact.
4. **Unverifiable in this session.** No network (search backend HTTP 402), so AO3
   endpoints, ToS text, and Kache availability are unverified. Nothing in §5.3's
   recommendation depends on the *exact* URL, only on the existence of an official
   export — confirm that first.
5. **Independent pass.** Before M0 closes, a second agent re-counts §1.1, re-reads the
   ten spot-checked musika anchors, and confirms every D-item has an ADR. Musika's own
   `docs/mds/audit/verification.md` is the format: verdict table, PASS/FAIL per category,
   explicit "unverifiable" list.

---

## Appendix A — musika evidence index

What to read, and what it teaches. Paths relative to `/home/rplakama/Projects/musika`.

| path | teaches |
| --- | --- |
| `docs/Daemon.md` | process lifecycle, two sockets, ownership, reattach, reliability, systemd |
| `docs/Commands.md` | one command surface for socket + in-app line + tray + MPRIS |
| `docs/mds/rust-arch-reference.md` | the audit *rubric*: evidence-first, macro lens, "less must be real", priorities |
| `docs/mds/audit/verification.md` | what an independent verification looks like (hashes, PASS tables, unverifiable list) |
| `docs/mds/gpuui/00-design.md` | the design ladder: definition of done, seams, state flow, risks/decisions |
| `notes/gap/p0-p1.md` | the evidence format per item: Status / Evidence / Remaining / Regression test |
| `Issues.md` | how to write a finding someone can act on (symptom → file:line → fix) |
| `Cargo.toml:26-59` | the clippy deny list and *why* each lint is there |
| `clippy.toml` | test-vs-production lint split |
| `flake.nix` | allowlisted source, per-binary wrappers, headless closure, asset dir |
| `.github/workflows/ci.yml` | the matrix: clippy, lib tests, no-default-features, feature tests |
| `notes/gates/summary.txt` | machine-readable gate results |
| `scripts/modularity.sh` | how to inventory fat without an architect |
| `src/db.rs:59-149,652-673` | alias bundle, per-role pools, pragmas, migrator wiring |
| `src/library/sync.rs:326-355,619-782` | revision gating + staged set-difference deletes |
| `src/download.rs:37-84` | ledger state machine |
| `src/offline.rs:30-96` | offline catalog switching |
| `src/session.rs:29-73` | online flag as atomic + watch |
| `src/daemon.rs:67-89` | health probe contract |
| `src/daemon/clients.rs:78-202` | bounded outbox, latest slot, farewell |
| `src/ipc_command.rs:562-1109` | one command table driving parse/completion/help |
| `.dsh/skills/{inherit,dependency-logic,testing,release}` | the four house rules, worth adopting almost verbatim |

## Appendix B — glossary

| musika | AO3Instance |
| --- | --- |
| core daemon | `ao3i-server` |
| `musika-ui` child | browser tab |
| `daemon.sock` framed protocol | HTTP JSON + SSE/WebSocket |
| `ipc.sock` text commands | admin CLI → same HTTP API |
| `library.db` / `media.db` | one DB file (D1) + blob store |
| download ledger | ingest `job` table |
| covers/lyrics/colors caches | EPUB/chapter/cover blobs |
| Navidrome/Subsonic upstream | archiveofourown.org |
| health probe | `upstream_reachable()` |
| scroll memory | reading progress anchors |
| `library_rev` | `content_rev` |
| MPRIS / tray | (out of scope; optional admin notifications later) |

## Appendix C — proposed skeleton

```
AO3Instance/
├── Cargo.toml            # [lints.clippy] copied from musika's rationale
├── clippy.toml           # allow-unwrap-in-tests = true
├── flake.nix             # build + devShell + kache-substituter (optional)
├── .github/workflows/ci.yml
├── migrations/           # ONE dir, NNNN_<context>_<name>.sql
├── scripts/
│   ├── modularity.sh
│   ├── check-upstream.sh
│   ├── check-fixtures.sh
│   ├── check-docs.sh
│   └── check-offline.sh
├── src/
│   ├── lib.rs  consts.rs  config.rs  db.rs  model.rs
│   ├── upstream/{mod,http,limiter}.rs  upstream/parse/*.rs  upstream/fixtures/*
│   ├── ingest/{mod,job,work,epub,sanitize}.rs
│   ├── store/{mod,blobs,catalog,progress}.rs
│   ├── search/mod.rs
│   ├── web/{mod,routes/,templates/,assets/}
│   └── bin/ao3i-server.rs
├── docs/
│   ├── init-audit.md          # this file
│   ├── adr/0001-storage-engine.md …
│   └── design/                # design ladder, filled per milestone
├── notes/gap/                 # evidence-style gap analyses
├── Issues.md
├── Tasks.md                   # prefixes: ARC/ING/RDR/SRH/ACC/GATE
└── LICENSE
```

---

*End of v0. The next artifact is ADR-001, not code.*
