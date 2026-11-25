# ADR-0008 — Work creation (posting fanfics) is in scope

- **Status:** Accepted (2026-11-25)
- **Date:** 2026-11-25
- **Supersedes:** the "works/chapters creation" deferral in ADR-0003, for new works,
  chapter management and managing existing ones; chapter deletion/reordering and work
  metadata editing remain deferred
- **Related:** ADR-0003, ADR-0005, `docs/ao3-write-api.md` §3–§4.12,
  `docs/write-client-plan.md`

## Context

The owner's objective is a client that posts kudos, comments, subscriptions and
more; posting a fanfic is the capability that makes it a writer's client, not
only a reader's. ADR-0003 deferred works/chapters creation because of the size
of the parameter surface (tags, warnings, collections, skins) and because "a
posting client is a different product". That deferral was right for v1, whose
first slices were the cheap social actions.

Two things changed. The write spine now exists: every write goes through the
same gate (writes enabled and a usable session), confirmation, intent/audit
ledger, governor and one-attempt rule (ADR-0005). And the one genuinely
non-public part of the create contract — the numeric language ids and the
creator pseud ids, which AO3 never exposes in a public search form — can be read
from AO3's own `GET /works/new` page for the signed-in account, so the composer
does not have to hardcode private database facts.

## Decision

1. **New-work creation is in scope**, in the shape AO3 itself uses. The composer
   at `GET /works/new` mirrors `works/_standard_form.html.erb` and
   `_work_form_tags.html.erb`: rating, archive warnings, categories,
   fandom/relationship/character/additional-tag fields, title, creator pseud,
   summary, notes, language, collections, gifts, chapter title and work text.
   The select/checkbox options are parsed from AO3's live `/works/new` page and
   reused for 10 minutes (`AppState.compose_defaults`); no rating, warning,
   category, language id or pseud is hardcoded.
2. **Composer → local preview → one POST.** `POST /write/work/preview`
   validates and renders a confirmation page without any network write; the
   confirmed `POST /write/work` sends exactly one `POST /works` to AO3 with
   `post_button` (publish) or `save_button` (Save as Draft).
3. **Local validation mirrors AO3's model limits** so an obviously invalid work
   never reaches the archive: title 1–255, summary 1,250, notes 5,000, each tag
   150, at most 75 user-defined tags, work text 10–510,000, at least one archive
   warning, and rating/fandom/language/pseud required (`ao3::compose::validate`).
   Local errors re-render the composer with the text preserved; AO3-side
   validation failures return its `#error` block, whose messages are parsed and
   shown, also with the composed text preserved.
4. **The work and its first chapter are created together.** The request carries
   `work[chapter_attributes][content]` (singular) and optionally
   `work[chapter_attributes][title]`; after creation, further chapters are added
   through the chapter manager (item 8), not by re-posting the work.
5. **Both publish and draft are supported.** A 302 to `/works/<id>` means the
   work is live; a 302 to `/works/<id>/preview` means it was saved as an
   unpublished draft on AO3, which the archive deletes about 29 days after
   creation. The app says so in the composer and on the confirmation page.
6. **No scheduling, no background posting.** A create is always the immediate
   result of an explicit confirmed form submission; there is no queue, timer,
   draft autosave or automatic retry loop.
7. **The app also manages the account's existing works.** `GET /my/works` and
   `GET /my/works/drafts` render AO3's own `/users/<login>/works[/drafts]`
   indexes (30-minute page cache, honest offline notice). Two destructive-ish
   actions are offered: publishing a draft (`PUT /works/:id/post_draft`) and
   deleting a work or draft (`DELETE /works/:id`). Both reuse the write spine —
   a local confirmation page that names the target, a committed intent row, one
   request, redirect classification, audit evidence and the governor — and
   neither is ever retried. Deletion is irreversible and the confirmation page
   says so; the local cached copy and the cached account lists are dropped after
   a confirmed delete.
8. **Chapters can be added and edited after creation.** `GET
   /my/works/<id>/chapters` renders AO3's own chapter index
   (`/works/<id>/navigate`, 30-minute page cache), listing chapters in order
   with dates, Edit links and Add Chapter; My Works rows gained **Chapters** and,
   for works still in progress, **Add Chapter**. The editor
   (`/write/chapter/new|edit`) mirrors AO3's chapter form and pre-fills an edit
   from AO3's own edit page. A local preview/confirmation page runs the same
   spine, then exactly one request: `POST /works/<work_id>/chapters` with
   `post_without_preview_button` (posted) or `save_button` (unposted chapter
   draft), or `PUT /works/<work_id>/chapters/<chapter_id>` with `update_button`.
   A 302 to the chapter's own path means saved (the new id is parsed from the
   redirect); a 302 to `.../preview` means a chapter draft; a 200/422 re-render
   with AO3's `#error` block is a refusal shown on the editor with the composed
   text kept. After a success the cached work is re-fetched once (best effort)
   so the reader shows the new chapter. One confirmed request, no auto-retry; a
   timeout is `ambiguous`. Chapter deletion/reordering stay on AO3.

## Non-idempotent-create rules

Creation is not idempotent and AO3 offers no idempotency token for this POST, so
the outcome must be read from the redirect path:

- **One attempt.** The pipeline commits a `write_intent` row (`post_work`,
  `Work new`), sends one request, classifies it and closes the audit row. It
  never re-sends the create.
- **Classify from the 302.** `/works/<id>` = posted (`work-posted`);
  `/works/<id>/preview` = draft (`work-draft`); `/auth_error` = CSRF expired
  before the action ran; `/users/login` = logged out; `/lost_cookie` =
  incomplete jar. A 200 re-render with `#error` blocks is a failure (`failed`)
  carrying AO3's own messages; 429 is rate limited; 403/503 is an edge block.
- **Timeout is ambiguous, never retried.** A transport failure after the send
  may mean the work was created. The intent/audit row is closed `ambiguous`, the
  composer warns that the work may already exist, and the only acceptable
  follow-up is a manual check of the user's works list on AO3 before trying
  again. The app never infers success or failure from "no response".
- **The CSRF refresh retry is safe.** The session layer refreshes the token and
  retries once when AO3 answers `/auth_error`. Rails aborts in
  `protect_from_forgery` before `works#create` runs, so this retry cannot
  double-post; it is the only retry a create may perform.
- **Budget.** The `post_work` governor budget is 3 per hour — rare,
  consequential and one at a time — and the intent row plus audit outcomes make
  every attempt traceable. Withdrawing a posted work is only possible through the
  explicit delete action in the works manager (confirmation, one request, never a
  retry side effect).

## Chapter-management rules

Adding and editing a chapter reuses the same spine but has its own outcome shapes:

- **The chapter index and editor are AO3's own pages.** `/works/<id>/navigate` is
  parsed (`ol.chapter.index.group > li > a` plus `span.datetime`, work title from
  `h2.heading`; `works/navigate.html.erb`; AO3's `<position>. ` title prefix is
  stripped) and fetched through the 30-minute page cache; the editor is fetched
  live from `GET /works/<work_id>/chapters/<chapter_id>/edit` (`#chapter-form`,
  `chapters/_chapter_form.html.erb`), with textarea values decoded to raw author
  HTML so the editor shows markup rather than entities. A page without its
  marker fails closed (`UpstreamChanged`) instead of rendering an editor that
  cannot save.
- **Create and update are different requests.** Create is `POST
  /works/<work_id>/chapters` (`post_without_preview_button` = posted,
  `save_button` = unposted chapter draft); update is `PUT
  /works/<work_id>/chapters/<chapter_id>` (`update_button`). The chapter fieldset
  is singular (`chapter[...]`), and there is no chapter id on create — the
  redirect path supplies it.
- **Classify from the redirect.** 302 → `/works/<work_id>/chapters/<id>` = saved
  (audit `chapter-posted` / `chapter-updated`); 302 → `…/<id>/preview` = chapter
  draft (audit `chapter-draft`); 200/422 with `#error` = AO3 refused (audit
  `failed`, AO3's messages and the composed text kept on the editor);
  `/auth_error`, `/users/login` and `/lost_cookie` are typed as usual. On update
  the redirect must name the edited chapter: a 302 to the work path is AO3's
  permission/not-found refusal, not a save. One attempt, never auto-retried; a
  timeout is `ambiguous` and tells the user to check AO3 before retrying.
- **Cache refresh after success.** The cached work is re-fetched once
  (best effort) so the reader shows the new chapter; a refresh failure is logged
  and leaves the previous cached copy readable, with the reader's own refresh
  control as the fallback.
- **Budget and limits.** The `chapter` governor budget is 10 per hour, for
  create and update alike. Local validation mirrors AO3's chapter limits
  (title ≤ 255, summary ≤ 1,250, notes and end notes ≤ 5,000 each, text
  10–510,000; `chapter.rb:14-24`) before anything is sent.
- **Not implemented.** Deleting chapters and AO3's dedicated reordering view
  stay on AO3 (the editor's Chapter Number field still changes a single
  position); there is no scheduling or backdating control, so AO3 dates a new
  chapter today and the publication date is never exposed.

## Scope

**Now:** new work plus first chapter; publish or save as draft; composer options
read from AO3; local validation; local confirmation; audit and governor as above.
Plus the author works manager: My Works/My Drafts lists read from AO3 and cached
for 30 minutes, publishing a saved draft, and deleting a work or draft (each with
a confirmation page, one request and no auto-retry). Draft publishing is budgeted
10/hour and deletion 5/hour. Plus chapter management: AO3's chapter index, adding
a chapter (posted, or saved as an unposted chapter draft) and editing an existing
one through AO3's chapter form, each with a local confirmation, exactly one
request and no auto-retry; chapter saves are budgeted 10/hour.

**Later** (each needs its own contract, confirmation rules and tests): deleting
and reordering chapters, editing work metadata/tags, co-creators, and any
scheduling or background posting. These remain deferred and are not implied by
this ADR.

## Consequences

- The write surface now includes its most consequential action; the confirmation
  page, the audit ledger and the governor matter more than for any earlier
  action.
- A timed-out create leaves a real ambiguity that only the user can resolve on
  AO3; the app reports `ambiguous` instead of guessing, and the composed text
  stays in the composer so nothing is lost.
- Drafts are AO3 objects, not local ones: they are not cached and expire after
  ~29 days; the works manager publishes or deletes them explicitly on AO3. A
  deleted work is gone permanently — the confirmation page is the guard, and the
  audit row is the record.
- The composer depends on AO3's new-work page shape; if the page loses
  `#work-form` the parser fails closed (`UpstreamChanged`) and the composer
  shows an honest error rather than a form that cannot post.
- Every work still starts with exactly one chapter, but multi-chapter works can
  now grow from the app. The chapter set can be corrected but not removed:
  chapter deletion and reordering stay on AO3, and a successful save refreshes
  the cached work once so the reader reflects it without a manual reload.
- A chapter save inherits the create ambiguity: a timed-out create may already
  have posted the chapter, so the app reports it as unknown and asks the user to
  check AO3 instead of retrying; a failed cache refresh after success never
  turns the write into a failure.

## Test and evidence

- `tests/writes_work.rs` (6 tests): the composer renders AO3's parsed options
  and the header **Post** link; preview sends nothing upstream; a confirmed
  publish makes exactly one `POST /works`, caches and opens the work and audits
  `work-posted`; a draft uses `save_button`, audits `work-draft` and reports the
  29-day lifetime; local validation blocks an invalid work before any write;
  AO3's `#error` messages are surfaced, the attempt is audited `failed` and is
  not retried; the composer refuses when `AO3I_WRITE_ENABLED` is off.
  `ao3::compose` unit tests cover option parsing from the checked-in
  `/works/new` fixture, the fail-closed non-form page, and the model limits.
- `tests/account_works.rs` (5 tests): the works/drafts pages render AO3's lists;
  publishing confirms, PUTs once, caches and opens the work and audits
  `work-published`; an AO3 publish refusal is surfaced and audited `failed`; a
  delete confirms, DELETEs once, removes the local cached copy and audits
  `work-deleted`; both destructive actions refuse without the write switch.
  `ao3::account_lists::works` unit tests (3) cover the list parser and its
  fail-closed login page. Full suite 297 green at the time of writing.
- `tests/writes_chapters.rs` (7 tests): the chapter index renders with Edit
  links; posting confirms, sends exactly one `POST /works/<id>/chapters`, audits
  `chapter-posted` and refreshes the cached work exactly once; the draft button
  audits `chapter-draft`; editing prefills from AO3's edit page and sends exactly
  one `PUT`; short content is rejected locally with zero writes; AO3's `#error`
  refusal is surfaced, audited `failed` and not retried; the editor refuses
  without the write switch. `ao3::chapters` unit tests (4) cover the index
  parser, the editor prefill (including decoded author markup), the fail-closed
  foreign page and the local limits. Full suite 309 green at the time of
  writing.
- The actions are documented in `docs/ao3-write-api.md` §3–§4.12, and
  `scripts/check-ao3-routes.sh` asserts the works new/create, drafts list,
  post_draft, destroy, nested chapters, chapters create/update and
  `post_without_preview_button` claims against the pinned otwarchive clone
  (57 claims).
- Evidence from the clone (`452391b`): `works_controller.rb:289-319` (create,
  `post_button` publish switch, 302 to the work vs `/preview`, the 29-day draft
  notice), `works_controller.rb:911-934` (`work_params`; `posted` is not
  permitted), `_standard_form.html.erb:2,208`, `_work_form_tags.html.erb:22-83`,
  `_work_form_associations_language.html.erb:5-10`,
  `_posting_fieldset.html.erb:12-25`, `_byline.html.erb:24-32`;
  `routes.rb:393,403-414` (`navigate`, nested chapters), `chapters_controller.rb:81-87`
  (`new`/`edit`), `:114-136` (create), `:139-166` (update), `:167-181`
  (`update_positions`), `:261-264` (`post_chapter`), `:268-272`
  (`chapter_params`), `chapters/_chapter_form.html.erb` and
  `chapters/_posting_fieldset.html.erb:11-28` (button names),
  `works/navigate.html.erb:1-7` (chapter index markup).
