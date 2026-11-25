# AO3 write API — action contracts

The practical contract for posting to archiveofourown.org as a logged-in user. Derived
from the pinned clone (`452391b`); see [`ao3-internals.md`](ao3-internals.md) for the
evidence map and [`write-client-plan.md`](write-client-plan.md) for the implementation.

**Bottom line:** there is no user JSON API. Two actions have clean JSON responders
(subscriptions, favorite tags); the rest are HTML forms whose success and failure are
distinguished by redirect target, followed page content, and — for comments — a rendered
error block. The client must therefore treat *outcome detection* as a first-class problem,
not an afterthought.

---

## 1. Conventions

All requests:

| aspect | value |
| --- | --- |
| base | `https://archiveofourown.org` |
| encoding | `application/x-www-form-urlencoded; charset=UTF-8` for writes |
| auth | cookie jar: `_otwarchive_session` **and** `user_credentials` (both required) |
| CSRF | `authenticity_token` field, from `GET /token_dispenser.json` |
| referer | the page the user acted from (kudos redirect depends on it) |
| user agent | identifiable, contactable, honest: `AO3Instance/<ver> (+<project-url>; contact=<email>)` |
| accept | `text/html` for form posts; `application/json` only where documented |
| concurrency | target: one upstream request in flight per instance; W1 serializes callers through `&mut self` and enforces the spacing floor — the governor lands W3 |
| minimum interval | ≥ 2 s between upstream requests (global budget 300/300 s per IP) |

Never:

- follow a redirect with a second POST — Rails answers writes with `302`/`303`; continue
  with `GET`.
- retry a write on timeout without proving it did not land (§7).
- reuse a CSRF token across sessions or after a login.
- send `Accept: application/json` to a controller that only renders HTML; Rails will
  raise `UnknownFormat` and redirect to `/404` (`application_controller.rb:16`).

---

## 2. Session lifecycle

### 2.1 Login

```
GET  /users/login                      → 200, capture csrf-token meta
POST /users/login                      → 302 /users/:login        (success)
  user[login]=…&user[password]=…       → 200 re-render + flash    (failure)
  &authenticity_token=…                → 302 /users/login         (failure, stock Devise)
                                       → 302 /auth_error          (CSRF expired)
                                       → 302 /users/:login/change_password (pwned password)
GET  /token_dispenser.json             → {"token": "…"}           (post-rotation token)
GET  /users/:login/subscriptions       → 200 proves the jar
```

Evidence: form fields `app/views/users/sessions/_passwd.html.erb:1-8`; token endpoint
`app/controllers/home_controller.rb:40-44`; throttle 5/20 s
(`config/initializers/rack_attack.rb:70-90`); Devise rotation,
`after_sign_in_path_for` and the pwned-password redirect
(`application_controller.rb:223-236`); CSRF failures redirect to `/auth_error`
(`application_controller.rb:75-88`).

Notes:

- `user[remember_me]` is deliberately **omitted**: a 2-week session is enough for a
  server-side client, and the 3-month remembered cookie is a longer-lived bearer token.
- A 302 is not proof of success. Classify on `Location`: `/users/login` is a rejection,
  `/auth_error` is a CSRF failure, `change_password` is `PasswordChangeRequired`, and
  `/users/<login>` is the success path.
- The success `Location` is the **canonical login**. Use it for the probe: the user may
  have typed an email address.
- Match whole path segments against a same-origin (or root-relative) `Location`. An
  account literally named `login` is indistinguishable from the login page; we fail
  closed (`LoginRejected`) and cookie import still works for that account.
- Success is confirmed by a fresh authenticated read, not by the redirect alone.

### 2.2 Liveness probe

```
GET /users/:login/subscriptions
  200                      → alive
  302 → /users/login…      → logged out
  302 → /lost_cookie       → cookie jar incomplete (user_credentials missing)
  403/503 + HTML challenge → edge block
  anything else            → drift, fail closed (do not guess)
```

A 200 alone is accepted as proof in W1; W2's subscriptions-page parser adds a content
marker so an edge interstitial cannot masquerade as an authenticated read.

`users_only` redirects to the login page (`application_controller.rb:250-252,267-285`);
the forced-logout path is `application_controller.rb:148-157`.

### 2.3 Logout

```
DELETE /users/logout                    → 302 / (signed out)
  authenticity_token=…                  → 302 /auth_error (token rejected)
```

The token is required: AO3 applies `protect_from_forgery` to every non-GET, so a bare
DELETE is answered with `/auth_error` while still looking like a redirect — the browser
flow posts the token from the confirm-logout form
(`app/views/users/sessions/confirm_logout.html.erb:3`, `_greeting.html.erb:48`).
Send it both as `authenticity_token` and `X-CSRF-Token`.

Devise signs out all scopes and the app clears `user_credentials`
(`config/initializers/devise.rb:247`, `application_controller.rb:159-166`). Wipe the local
jar regardless of the response, but surface `LogoutFailed` when upstream refused — claiming
a sign-out that did not happen is a security lie. If the first DELETE is answered with
`/auth_error`, refresh the token and retry **exactly once** (idempotent action); a second
rejection is `LogoutFailed`.

### 2.4 Cookie import (no password custody)

The user supplies `_otwarchive_session` (HttpOnly, but visible in devtools → Application →
Cookies) and `user_credentials`. Store the whole jar together; the session cookie alone
triggers `/lost_cookie`. Import is validated by the §2.2 probe before being marked active.

---

## 3. Action matrix

| action | method + path | format | params | success signal | failure signal |
| --- | --- | --- | --- | --- | --- |
| kudos | `POST /kudos` | HTML | `kudo[commentable_id]`, `kudo[commentable_type]=Work` | 302 → `Referer#kudos_message`; page shows kudos count/self | 302 → same with error flash; body shows error text |
| comment | `POST /works/:id/comments` | HTML | `comment[pseud_id]`, `comment[comment_content]` | 302 → `/comments/:id` or work page anchored `comment_<id>` | 200 re-render with error block; 302 → `/429`; 302 → `/auth_error` |
| reply | `POST /comments/:comment_id/comments` | HTML | same | 302 → thread anchored | as above |
| subscribe | `POST /users/:uid/subscriptions` | **JSON** | `subscription[subscribable_id]`, `subscription[subscribable_type]` | 201 `{item_id, item_success_message}` | 422 `{errors:[…]}` |
| unsubscribe | `DELETE /users/:uid/subscriptions/:id` | **JSON** | — | 200 `{item_success_message}` | 404/302 login |
| bookmark | `POST /works/:id/bookmarks` | HTML | `bookmark[pseud_id]`, `[bookmarker_notes]`, `[tag_string]`, `[collection_names]`, `[private]`, `[rec]` | 302 → `/bookmarks/:id` | 200 re-render (new form) |
| mark for later | `PATCH /works/:id/mark_for_later` | HTML | — | 302 back; next read shows state | 302 → login |
| mark as read | `PATCH /works/:id/mark_as_read` | HTML | — | 302 back; next read shows state | 302 → login |
| favorite tag | `POST /users/:uid/favorite_tags` | **JSON** | `favorite_tag[tag_id]` | 201 `{item_id, item_success_message}` | 422 `{errors:[…]}` |
| work create | `POST /works` | HTML | `work[title]`, `work[rating_string]`, `work[archive_warning_strings][]`, `work[fandom_string]`, `work[language_id]`, `work[chapter_attributes][content]`, `work[author_attributes][ids][]`, `post_button`/`save_button` | 302 → `/works/:id` (posted) or `/works/:id/preview` (draft) | 200 re-render with `#error` blocks; 429; 302 → `/auth_error` |
| draft publish | `PUT /works/:id/post_draft` | HTML | `authenticity_token` only — the draft already exists | 302 → `/works/:id` (now posted) | 302 → `/users/:login/works/:id/edit` (already posted/invalid); 200/422 with `#error`; 302 → `/auth_error`; 429; 403/503 |
| work delete | `DELETE /works/:id` | HTML | `authenticity_token` (body field **and** `X-CSRF-Token`) | 302 → `/users/:login/works` (posted work) or `/users/:login/works/drafts` (draft) | 302 → `/users/login`, `/lost_cookie`, `/auth_error`; 200/422; 429; 403/503 |

Deferred (documented, not implemented): chapter addition, editing a work or its
tags/metadata, mute/block, collections, comment moderation
(approve/reject/freeze/hide), admin version of any action. Draft publishing and work
deletion are implemented — see §4.9 and §4.10.

---

## 4. Per-action contracts

### 4.1 Kudos — `POST /kudos`

- **Preconditions:** target is a `Work` (AdminPost is also valid but out of scope); user is
  not the creator; not blocked by any creator; account not banned/suspended; work visible.
- **Body:** `kudo[commentable_id]=<work id>`, `kudo[commentable_type]=Work`,
  `authenticity_token`.
- **Response:** always `302` on the HTML path. On success the flash is `kudos_notice` and
  the redirect is the referer with `#kudos_message`; on failure the flash is
  `kudos_error` and the redirect target is the same. **Status code is useless for outcome
  detection** (`app/controllers/kudos_controller.rb:44-72`).
- **Server invariant:** unique index on `(commentable_id, commentable_type, user_id)`
  (`db/schema.rb:651-652`); a race is rescued and reported with the duplicate message
  (`kudos_controller.rb:74-89`).
- **Idempotency:** server-unique. Retrying after an ambiguous failure is safe *provided*
  the duplicate message is mapped to `AlreadyDone`, not to an error.
- **Exact strings** (`config/locales/models/en.yml:220-234`):
  - `You have already left kudos here. :)`
  - `You can't leave kudos on your own work.`
  - `Sorry, you have been blocked by one or more of this work's creators.`
  - `You cannot leave kudos while your account is banned.`
  - `You cannot leave kudos while your account is suspended.`

### 4.2 Comments — `POST /works/:id/comments`

- **Preconditions:** work is posted (not draft), not hidden by an admin, not in an
  unrevealed collection; comments not disabled; commenter not blocked; parent not frozen,
  not hidden, not unapproved spam (`comments_controller.rb:1-40`).
- **Body:** `comment[pseud_id]=<pseud id>`, `comment[comment_content]=<text>` (≤ 10,000
  chars, `config/config.yml:105`), `authenticity_token`.
  The pseud must belong to the account (`check_pseud_ownership`,
  `comments_controller.rb:79-87`); default pseud is the safe choice.
- **Nested target:** the commentable comes from the URL (`work_id`, `chapter_id`,
  `comment_id`), never from the body (`load_commentable`,
  `comments_controller.rb:297-315`).
- **Rate limit:** 3 per 900 s per logged-in user, authors exempt; HTML clients are
  redirected to `/429`, JS clients get 429 JSON (`comments_controller.rb:57-89`).
- **Spam:** Akismet runs on create; a flagged comment fails validation with
  `This comment looks like spam to our system, sorry! Please try again.`
  (`app/models/comment.rb:67-74`, `config/locales/models/en.yml:178`).
- **Duplicate:** identical content by the same pseud on the same commentable is rejected:
  `You've already left this comment here. (It may not appear right away for performance
  reasons.)` (`app/models/comment.rb:84-89`, `config/locales/models/en.yml:170`).
- **Moderation:** a work can require comment approval; the comment is then created
  `unreviewed` and the success message differs (`comments_controller.rb:413-420`).
- **Idempotency:** none. Two different comments are both accepted; the *same* comment is
  rejected. The client must not auto-retry.
- **Success detection:** follow the 302; a success lands on `/comments/:id` (new comment
  page) or on the work page containing `id="comment_<n>"`. A validation failure renders
  `200` with the form and an error block.

### 4.3 Subscriptions — `POST /users/:uid/subscriptions`

- **Preconditions:** `uid` is the logged-in user (`check_ownership`); `subscribable_type`
  ∈ `Work`, `User`, `Series` (`app/models/subscription.rb:2`).
- **Body:** `subscription[subscribable_id]`, `subscription[subscribable_type]`,
  `authenticity_token`; send `Accept: application/json` (or `.json`).
- **Response:** `201 {"item_id":n,"item_success_message":"…"}`; failure
  `422 {"errors":[…]}` (`subscriptions_controller.rb:26-46`).
- **No uniqueness constraint:** no unique index (`db/schema.rb:1119-1127`) and no model
  validation. Posting twice can create duplicate subscriptions. **Always pre-check the
  subscription index and never blind-retry.**
- **Listing:** `GET /users/:uid/subscriptions?type=Work|User|Series` (filters by type,
  `subscriptions_controller.rb:15-27`).
- **Never create duplicates retroactively:** if the index already shows two, leave them;
  do not "clean up" on the user's behalf without an explicit action.

### 4.4 Unsubscribe — `DELETE /users/:uid/subscriptions/:id`

- Requires the subscription id from the index. JSON returns 200 with a message.
- Removing a subscription is safe to retry only after re-reading the index.

### 4.5 Bookmarks — `POST /works/:id/bookmarks`

- **Preconditions:** logged in, not suspended; work visible.
- **Body:** `bookmark[pseud_id]`, `bookmark[bookmarker_notes]`, `bookmark[tag_string]`,
  `bookmark[collection_names]`, `bookmark[private]` (`"1"`/`"0"`), `bookmark[rec]`
  (`bookmarks_controller.rb:335-339`).
- **Response:** 302 → `/bookmarks/:id`; failure re-renders the new bookmark form with 200
  (`bookmarks_controller.rb:194-204`).
- **Idempotency:** none (a user can bookmark the same work twice? — server-side confirm
  needed; treat as client-guarded).

### 4.6 Reading flags — `PATCH /works/:id/mark_for_later` · `/mark_as_read`

- No body parameters beyond CSRF; the action toggles `Reading.toread` for the current user
  (`app/controllers/works_controller.rb:704-722`, `app/models/reading.rb:19-26`).
- Response: 302 back. The flash appears only on transitions, so the reliable signal is a
  follow-up read of the work page's button state.
- These are the natural primitives for "keep this for later" inside our reader.

### 4.7 Favorite tags — `POST /users/:uid/favorite_tags`

- Body `favorite_tag[tag_id]`; JSON 201/422 (`favorite_tags_controller.rb:9-44`).
- Max 20 favorite tags (`config/config.yml:206`); duplicates rejected by the model.

### 4.8 Work create — `POST /works`

- **Preconditions:** signed-in usable session (both cookies, §2); CSRF token; writes
  enabled client-side. Rating, fandom, language, creator pseud and at least one archive
  warning are required. This request creates the work **and its first chapter**; adding
  further chapters is a different contract (`chapters#create`).
- **Body (minimal):** `work[title]`, `work[rating_string]`,
  `work[archive_warning_strings][]` (at least one), `work[fandom_string]`,
  `work[language_id]`, `work[chapter_attributes][content]`,
  `work[author_attributes][ids][]`, `authenticity_token`.
  **Optional:** `work[category_strings][]`, `work[relationship_string]`,
  `work[character_string]`, `work[freeform_string]`, `work[summary]`, `work[notes]`,
  `work[collection_names]`, `work[recipients]`, `work[chapter_attributes][title]`.
  The chapter field is **singular** (`work[chapter_attributes]`, from
  `fields_for "work[chapter_attributes]"`, `_standard_form.html.erb:208`); tags and
  categories use plural array fields.
- **Button name decides publish vs draft** (`_posting_fieldset.html.erb:12-25`):
  `post_button` makes the controller set `@work.posted = true` and redirect to
  `/works/:id`; `save_button` (or no post button at all) saves an unpublished draft and
  redirects to `/works/:id/preview`. The controller only tests parameter presence
  (`params[:post_button]`, `works_controller.rb:303`), so the submitted value
  (`Post` / `Save as Draft`) is ignored. `work[posted]` is **not** a permitted parameter
  (`work_params`, `works_controller.rb:911-934`) — the flag cannot be forced in the body.
- **Limits** (mirrored in the client from AO3's models/config): title 1–255, summary
  ≤ 1,250, notes ≤ 5,000, each tag ≤ 150 chars, fandom + relationship + character +
  additional tags ≤ 75 total, work text 10–510,000 chars. AO3 still validates tags,
  recipients, language and collection membership server-side; local validation is a fast
  pre-check, not the authority.
- **Response classification:**

  | response | meaning |
  | --- | --- |
  | 302 → `/works/:id` | work created and **posted** |
  | 302 → `/works/:id/preview` | work created as an **unposted draft**; AO3 deletes drafts about 29 days after creation (`works_controller.rb:314-316`) |
  | 200 (or 422) re-render with `#error li` | validation failure; the messages are AO3's own user-facing text, nothing was created |
  | 302 → `/auth_error` | CSRF expired; Rails rejected before the action; one refresh-and-retry is safe |
  | 302 → `/users/login` | logged out; re-authenticate |
  | 302 → `/lost_cookie` | jar incomplete (`user_credentials` missing) |
  | 429 | rate limited; honour `Retry-After` |
  | 403/503 | edge block / challenge |
  | transport error | **ambiguous**: the work may exist; verify on AO3 before retrying |

- **Gotchas:**
  - There is no `POST /works/preview`. The member route is `GET /works/:id/preview`
    (`routes.rb:390`), and AO3's own **Preview** button submits the create form; with no
    `post_button`, that persists a draft. Previewing upstream is a write, never a read.
  - `work[language_id]` is a **numeric database id**, not an ISO code (the search form
    only exposes ISO short codes). Parse the `<select>` on `GET /works/new`
    (`_work_form_associations_language.html.erb:5-10`).
  - The byline is `work[author_attributes][ids][]`; for a one-pseud account AO3 renders
    it as a hidden input that still carries the `[]` suffix
    (`_byline.html.erb:24-32`), so send the array form either way.
  - The create response is a redirect even for a draft; follow it with a `GET`, never a
    second `POST` (§1).

### 4.9 Draft publish — `PUT /works/:id/post_draft`

- **Preconditions:** signed-in usable session (both cookies, §2); CSRF token; writes
  enabled client-side; `:id` is an unposted draft owned by the signed-in account. The
  route is `put :post_draft` on the works resource (`routes.rb:392`); the action
  (`works_controller.rb:596-628`) sets `posted = true` and increments the minor version.
- **Request:** `PUT /works/:id/post_draft`. The body may carry **only
  `authenticity_token`** — no work fields are needed, the draft already exists on AO3.
  Send the token as the form field and as `X-CSRF-Token`, as for other writes (§2.3).
- **Response classification** (redirect outcomes are `302`; validation failures can
  re-render with 200/422):

  | response | meaning |
  | --- | --- |
  | 302 → `/works/:id` | draft published; the work is live (a moderated-collection submit also lands here with a different flash) |
  | 302 → `/users/:login/works/:id/edit` | refused: already posted, or invalid (`works_controller.rb:600-611`); nothing changed |
  | 200 (or 422) re-render with `#error` | refusal; the messages are AO3's own user-facing text |
  | 302 → `/auth_error` | CSRF expired; Rails aborts before the action, so one refresh-and-retry is safe |
  | 302 → `/users/login` | logged out; re-authenticate |
  | 302 → `/lost_cookie` | jar incomplete (`user_credentials` missing) |
  | 429 | rate limited; honour `Retry-After` |
  | 403/503 | edge block / challenge |
  | transport error | **unknown**: the publish may have landed; verify the list before retrying |

- **Idempotency:** publishing is a one-way state transition (draft → posted). A repeat
  after success is refused by the "already posted" branch, so it cannot publish twice —
  but the client still sends **one confirmed request and never retries**: the outcome is
  read from the redirect, and a timeout is treated as unknown.
- **Budget:** the client governor allows `publish_draft` 10 per hour. Publishing is rare,
  consequential and briefly public, so one at a time and no automatic retry.
- **Gotchas:**
  - The redirect target is the proof. Follow the 302 to `/works/:id` with a `GET` (the
    app then caches the freshly published work and opens it); never re-send the `PUT`.
  - A 302 to the owner edit page is a refusal, not a network error — surface AO3's own
    explanation rather than retrying. An already-posted draft needs no publish at all.
  - After a successful publish the work is public on AO3 immediately; there is no
    unpublish from the client (only the separate, permanent delete in §4.10).

### 4.10 Work delete — `DELETE /works/:id`

- **Preconditions:** signed-in usable session; CSRF token; writes enabled client-side;
  the work or draft belongs to the signed-in account. `resources :works` gives the
  destroy action (`routes.rb:384`); AO3's own browser flow goes through
  `GET /works/:id/confirm_delete`, but the destructive request is the plain `DELETE`.
- **Request:** `DELETE /works/:id` with `authenticity_token` in the body **and** as
  `X-CSRF-Token` — a bare DELETE is answered with `/auth_error` while looking like a
  redirect (§2.3). The app confirms locally first
  (`GET /write/preview?kind=work-delete&id=<id>`), and that page states the delete is
  permanent and cannot be undone.
- **Response classification:**

  | response | meaning |
  | --- | --- |
  | 302 → `/users/:login/works` | posted work deleted |
  | 302 → `/users/:login/works/drafts` | draft deleted (the controller redirects to the list the work came from, `works_controller.rb:444-448`) |
  | 200 (or 422) re-render with `#error` | refusal; AO3's messages are user-facing |
  | 302 → `/auth_error` | CSRF expired; Rails aborts before the action |
  | 302 → `/users/login` | logged out; re-authenticate |
  | 302 → `/lost_cookie` | jar incomplete |
  | 429 | rate limited; honour `Retry-After` |
  | 403/503 | edge block / challenge |
  | transport error | **unknown**: the delete may have landed; verify the list before retrying |

- **Idempotency:** deleting is **irreversible** — AO3 has no restore for this action. A
  repeat after success finds no work (`Work.find` raises, so the route answers 404); it
  never deletes something else. The client sends one confirmed request and never retries:
  a timeout is unconfirmed, so the user checks My Works/My Drafts (and AO3) before a
  second attempt.
- **Budget:** the client governor allows `delete_work` 5 per hour. Deletion is
  destructive and rare, so it is budgeted and one at a time.
- **Gotchas:**
  - The drafts list vs works list redirect distinguishes a deleted draft from a deleted
    posted work; accept **both** as success (do not require one path).
  - Deletion is permanent and removes the work for every reader, not just this client;
    the confirmation page must say so before the request.
  - After a successful delete, drop the local cached copy of the work and invalidate the
    30-minute cached `/users/<login>/works` pages — otherwise My Works keeps showing a
    work that no longer exists.

---

## 5. Outcome detection

Ordered from strongest to weakest signal. Prefer the highest available for each action.

1. **JSON envelope** — subscriptions, favorite tags. Parse `item_id`/`errors`.
2. **Final URL** — classify the `Location`/status directly (the client never follows
   redirects by itself; a verification read is a separate explicit GET):
   - `/comments/:id` → comment created;
   - `/bookmarks/:id` → bookmark created;
   - `/works/:id` → work created and posted, or (for `PUT /works/:id/post_draft`) a
     draft published;
   - `/works/:id/preview` → work created as an unposted draft;
   - `/users/:login/works` or `/users/:login/works/drafts` → work deleted (the drafts
     list means the deleted work was a draft; both are success for `DELETE /works/:id`);
   - `/users/:login/works/:id/edit` → draft publish refused (already posted or invalid);
   - `/auth_error` → CSRF expired;
   - `/users/:login/change_password` → `PasswordChangeRequired`;
   - `/429` → rate limited (the comment limiter's redirect is W5; W1 maps the HTTP 429 status);
   - `/users/login…` → logged out;
   - `/lost_cookie` → jar incomplete.
   Login has its own rules: a 302 to `/users/login` is a rejection, and the success
   `Location` supplies the canonical login (see §2.1).
3. **Body marker** — for kudos, the followed work page:
   - success: the kudos section shows the user / `#kudos_message` is present;
   - failure: one of the exact strings in §4.1 appears in the page.
   For a work create, the failure marker is a re-rendered form carrying an `#error`
   block whose `li` items are AO3's messages (§4.8).
4. **Re-read verification** — for ambiguous cases (network failure after send, missing
   marker): read the target state (kudos given? subscription present? comment id present?)
   and only then decide. Never infer success from "no error".

Never use HTTP status alone: kudos success and failure are both `302`.

---

## 6. Error taxonomy

| code | meaning | detection | recovery |
| --- | --- | --- | --- |
| `LoggedOut` | session dead | 302 → `/users/login` | re-auth; do not retry |
| `SessionIncomplete` | jar missing `user_credentials` | 302 → `/lost_cookie` | require re-import |
| `CsrfMissing` | page carried no token | CSRF meta/JSON absent | stop; parser drift |
| `CsrfExpired` | token invalid | 302 → `/auth_error`, or 422 JSON `auth_error` | refresh token; retry only idempotent actions |
| `PasswordChangeRequired` | AO3 forces a password change (pwned) | 302 → `…/change_password` | tell the user; not a usable session |
| `LogoutFailed` | upstream did not sign out (e.g. token rejected) | 302 → `/auth_error` on logout, or 5xx | local wipe still happens; tell the user |
| `Killed` | operator kill switch | terminal state | no network until restart |
| `RateLimited` | IP/global throttle | 429 + `Retry-After` | park account, surface wait |
| `CommentRateLimited` | 3/900 s | 302 → `/429` after a comment POST | wait 15 min; explain |
| `AlreadyDone` | kudos duplicate | exact string §4.1 | treat as success |
| `PolicyRefused` | own work, blocked, banned, frozen, comments off | page text | explain; no retry |
| `SpamRejected` | Akismet | exact string §4.2 | no retry; suggest rewording |
| `Validation` | duplicate comment, too long, missing pseud, bad subscription type | re-render / 422 | show message |
| `TargetGone` | work deleted/private/hidden | 302 → `/404` or login | drop local state (**not constructed yet**: read outcome, lands with the W2 parser) |
| `UpstreamChanged` | expected field/marker missing | fixture mismatch | disable action, alert operator |
| `BlockedByEdge` | Cloudflare challenge/403/503 | HTML challenge markers | stop, back off, do not hammer |
| `Network` | timeout/reset | transport error | ambiguous: verify, never blind-retry |

---

## 7. Retry and idempotency rules

| class | actions | rule |
| --- | --- | --- |
| `ServerUnique` | kudos | retry allowed; map duplicate to `AlreadyDone` |
| `ServerConstrained` | favorite tags (model validation) | retry allowed; 422 duplicate → already done |
| `ClientGuarded` | comments, bookmarks | **never auto-retry**; verify by re-read; require a new user confirmation for a second attempt |
| `NoConstraint` | subscriptions | **never retry**; pre-check the index, post-check, show state |
| `NonIdempotentCreate` | work create | **never retry**; read the 302 to classify; a timeout is ambiguous — the user verifies on AO3 before a second attempt |
| `DestructiveStateChange` | draft publish, work delete | **never auto-retry**; one confirmed request; classify from the redirect; a timeout is unconfirmed — verify My Works/My Drafts before a second attempt (deletion is irreversible) |

Ambiguity rule: a transport failure *after* the request was written is `Ambiguous`. The
only acceptable follow-up is a read that proves the effect; if the read cannot prove
either way, report "unconfirmed" and let the user decide. A duplicate public comment is
worse than a failed one.

---

## 8. Limits

| limit | value | source |
| --- | --- | --- |
| global requests per IP | 300 / 300 s | `config.yml:209-210` |
| login attempts | 5 / 20 s per IP and per login | `config.yml:211-212` |
| comments per user | 3 / 900 s | `config.yml:229-230` |
| comment length | 10,000 chars | `config.yml:105` |
| favorite tags | 20 | `config.yml:206` |
| kudos shown | 50 per page | `config.yml:205` |
| work title | 255 chars | `config.yml:101` |
| work summary | 1,250 chars | `config.yml:103` |
| work notes | 5,000 chars | `config.yml:104` |
| tag name | 150 chars | `config.yml:106` |
| work text | 10–510,000 chars | `config.yml:107-108` |
| user-defined tags (fandom, relationship, character, additional) | 75 total | `config.yml:132` |
| session lifetime | 2 weeks (3 months remembered) | `config.yml:14-15` |
| comment thread depth | 5 | `config.yml:198` |

The client's own floor is stricter than the server's: ≥ 2 s between requests, one in
flight, no parallelism, no background writes. Work creation is additionally budgeted at
3 per hour by the write governor: posting is rare, consequential and non-idempotent.
Draft publishing is budgeted at 10 per hour and work deletion at 5 per hour; both are one
confirmed request and never retried.

---

## 9. Anti-patterns (each one is a bug we have seen elsewhere)

1. Posting to `/works/:id/kudos` — the route is `POST /kudos` with the work in the body.
2. Treating `302` as success for kudos/comments.
3. Sending `Accept: application/json` to `/kudos` or `/comments` — `UnknownFormat`, `/404`.
4. Storing only `_otwarchive_session` — next request is `/lost_cookie`.
5. Caching the CSRF token across login or across accounts.
6. Retrying a comment POST after a read timeout.
7. Blindly re-subscribing because the button said "Subscribe" — the list may already
   contain a duplicate the server will happily accept again.
8. Fetching `/downloads/…` in the background — robots-disallowed (ADR-0004).
9. Building a comment body from a template and posting it to many works. That is spam by
   every definition AO3 uses, and Akismet plus the 3/900 s limit will notice.
10. Using a spoofed browser UA to dodge bot scoring. It converts a policy question into an
    adversarial one.

---

## 10. `TODO(verify)` before W3

- [ ] Whether kudos from a chapter URL is accepted for logged-in users
      (`kudos_controller_spec.rb:35-46` covers guests only).
- [ ] Response of `POST /comments` when the work requires moderation — status and
      redirect target for `unreviewed` comments (`comments_controller.rb:413-420`).
- [ ] Whether `POST /works/:id/bookmarks` is idempotent, or a second POST creates a
      second bookmark.
- [ ] Exact subscription duplicate behaviour on the sandbox (no DB constraint).
- [ ] Whether `view_adult=1` is required to kudos/comment on adult works.
- [ ] Work create on the sandbox: publish → 302 `/works/:id` and save draft → 302
      `/works/:id/preview`; and whether the hidden empty `work[archive_warning_strings][]`
      input AO3's own form includes (`include_hidden`) is needed alongside a selected
      warning.
- [ ] Cloudflare challenge shape on a plain HTTP client (status, body markers).
