# ADR-0008 — Work creation (posting fanfics) is in scope

- **Status:** Accepted (2026-11-25)
- **Date:** 2026-11-25
- **Supersedes:** the "works/chapters creation" deferral in ADR-0003, for new works only;
  chapter addition and work editing/deletion remain deferred
- **Related:** ADR-0003, ADR-0005, `docs/ao3-write-api.md` §3–§4.8,
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
   `work[chapter_attributes][title]`; there is no chapter-add flow yet.
5. **Both publish and draft are supported.** A 302 to `/works/<id>` means the
   work is live; a 302 to `/works/<id>/preview` means it was saved as an
   unpublished draft on AO3, which the archive deletes about 29 days after
   creation. The app says so in the composer and on the confirmation page.
6. **No scheduling, no background posting.** A create is always the immediate
   result of an explicit confirmed form submission; there is no queue, timer,
   draft autosave or automatic retry loop.

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
  every attempt traceable. A posted work cannot be withdrawn from the app.

## Scope

**Now:** new work plus first chapter; publish or save as draft; composer options
read from AO3; local validation; local confirmation; audit and governor as above.

**Later** (each needs its own contract, confirmation rules and tests): adding
chapters (`POST /works/:id/chapters`), editing or deleting a work
(`PATCH`/`DELETE /works/:id` with challenge and series edges), a My Works
management page, and any scheduling or background posting. These remain
deferred and are not implied by this ADR.

## Consequences

- The write surface now includes its most consequential action; the confirmation
  page, the audit ledger and the governor matter more than for any earlier
  action.
- A timed-out create leaves a real ambiguity that only the user can resolve on
  AO3; the app reports `ambiguous` instead of guessing, and the composed text
  stays in the composer so nothing is lost.
- Drafts are AO3 objects, not local ones: they are not cached, they expire after
  ~29 days, and publishing them for real happens on AO3 (or through a future
  edit-and-post flow).
- The composer depends on AO3's new-work page shape; if the page loses
  `#work-form` the parser fails closed (`UpstreamChanged`) and the composer
  shows an honest error rather than a form that cannot post.
- Scope is intentionally one chapter. Multi-chapter posting remains a separate
  product decision, not a hidden part of this one.

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
- The action is documented in `docs/ao3-write-api.md` §3–§4.8, and
  `scripts/check-ao3-routes.sh` asserts the works new/create claims against the
  pinned otwarchive clone (47 checks green).
- Evidence from the clone (`452391b`): `works_controller.rb:289-319` (create,
  `post_button` publish switch, 302 to the work vs `/preview`, the 29-day draft
  notice), `works_controller.rb:911-934` (`work_params`; `posted` is not
  permitted), `_standard_form.html.erb:2,208`, `_work_form_tags.html.erb:22-83`,
  `_work_form_associations_language.html.erb:5-10`,
  `_posting_fieldset.html.erb:12-25`, `_byline.html.erb:24-32`.
