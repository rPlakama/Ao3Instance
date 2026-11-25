# AO3 internals — the collage

What this is: a working model of how archiveofourown.org (otwarchive) accepts a request,
derived from the local clone at `otwarchive/` rather than from guesses. Every claim here
carries a `path:line`. Anything unproven is tagged **TODO(verify)**.

Companion docs: [`ao3-write-api.md`](ao3-write-api.md) (the action contracts),
[`write-client-plan.md`](write-client-plan.md) (how we build against it),
[`security-standards.md`](security-standards.md) (how we hold secrets).

---

## 1. Provenance

| fact | value |
| --- | --- |
| upstream | `https://github.com/otwcode/otwarchive` |
| pinned commit | `452391b648f2fb06b55c36133812449747634504` |
| commit date | 2026-11-01 18:55 +0100 |
| tracked files | 2,706 |
| size | ~105 MB, full clone (not shallow) |
| local path | `otwarchive/` (gitignored) |
| license | GPLv2 — reference only, do not copy code into this project |

Rules for using it:

1. **Read-only.** Never edit `otwarchive/`; never vendor its code. The license and the
   maintenance burden both say no.
2. **Pin.** The audit/route gate prints the commit. If it differs from the value above,
   re-run `scripts/check-ao3-routes.sh` and re-read every section this change touches.
3. **Cite.** A claim without `path:line` is not evidence; it is a hypothesis.
4. **Never run the clone against production data.** Use the Docker sandbox (§9).

---

## 2. Repository map — where the answers live

| question | path |
| --- | --- |
| What URLs exist and which HTTP verbs? | `config/routes.rb` |
| What does a controller require, permit, and redirect to? | `app/controllers/**` |
| What form fields and hidden inputs does the real site post? | `app/views/**` |
| What markup renders a comment thread (our read-side parser)? | `app/views/comments/_commentable.html.erb`, `_comment_thread.html.erb`, `_single_comment.html.erb`; selector table in `src/ao3/comments.rs` |
| What validation, uniqueness, spam and moderation rules apply? | `app/models/**` |
| What configuration constants (limits, TTLs, throttles) exist? | `config/config.yml` |
| How is CSRF, session and authentication wired? | `app/controllers/application_controller.rb`, `config/initializers/{session_store,devise,rack_attack}.rb` |
| What is the expected behaviour in edge cases? | `spec/controllers/**` (the oracle) |
| What is the public policy on automation? | `public/robots.public.txt`, `app/views/home/tos_faq.html.erb` |
| Can we run it locally? | `docker-compose.yml`, `script/docker/init.sh` |

The clone is a full Rails application with tests; the specs are more valuable than the
prose because they encode status codes and exact strings.

---

## 3. The request lifecycle

Every write is the same shape. Understanding it once explains every action.

```
browser form
   │  POST /path  (application/x-www-form-urlencoded)
   │  body: authenticity_token=… & <model>[field]=… & commit=…
   ▼
Rack::Attack            global per-IP throttle, login throttle          rack_attack.rb
   ▼
ApplicationController   protect_from_forgery with: :exception          application_controller.rb:4
   │                    before_actions: logged_in? / ownership / visibility
   ▼
Controller#action       params.require(:model).permit(...)             e.g. kudos_controller.rb:100-102
   ▼
Model                   validations, uniqueness, spam checks, callbacks  e.g. kudo.rb:4-50
   ▼
Response                HTML → flash + 302 redirect
                        JSON → 201/422 envelope (only where respond_to declares it)
```

Three consequences drive the whole client design:

1. **CSRF failure is a redirect, not a 4xx**, in HTML: `rescue_from
   ActionController::InvalidAuthenticityToken` → `/auth_error`
   (`app/controllers/application_controller.rb:4-8,75-88`). A naive client will read that
   302 as success unless it inspects the final URL.
2. **Success is usually a redirect plus a flash message.** Flash lives in the signed
   session cookie, so it is opaque to us; outcome detection must use the final URL, the
   rendered page, or a follow-up read. JSON responders are the exception and are always
   preferable where they exist.
3. **Param names are the model names**, not the URL names: `kudo[...]`, `comment[...]`,
   `subscription[...]`, `bookmark[...]`, `favorite_tag[...]`.

---

## 4. Access model

### 4.1 Authentication

- Devise, authentication key `:login` (username or email):
  `config/initializers/devise.rb:39`.
- Login form posts to `/users/login` with `user[login]`, `user[password]`,
  `user[remember_me]`: `app/views/users/sessions/_passwd.html.erb:1,4,6,8`.
- Logout is `DELETE /users/logout`: `config/initializers/devise.rb:247`. It is a
  non-GET, so it needs a CSRF token; the browser flow posts the confirm form
  (`app/views/users/sessions/confirm_logout.html.erb:3`, `_greeting.html.erb:48`).
  Without the token AO3 answers `/auth_error` while looking like a redirect.
- `paranoid` is off (`devise.rb:80` commented) → login errors are distinguishable.
- No user-facing 2FA. Two-factor is admin-only (`devise.rb:5-6`).
- `reconfirmable` is on (`devise.rb:132`) → an unconfirmed email can affect login.
- Devise locks an account after 50 failed attempts (`devise.rb:185`).

### 4.2 Session

- Cookie `_otwarchive_session`, signed-cookie store, expires after
  `DEFAULT_SESSION_LENGTH_IN_WEEKS` = 2 (`config/initializers/session_store.rb:13`,
  `config/config.yml:14`).
- "Remember me" extends to 3 months (`config.yml:15`, `devise.rb:139`).
- **`user_credentials` is load-bearing.** If a request is authenticated but that cookie is
  missing, the app signs the user out and redirects to `/lost_cookie`
  (`application_controller.rb:148-157`); it is set after every logged-in response
  (`:159-166`). Any stored session must persist the **whole cookie jar**, not just the
  session cookie.
- `_otwarchive_session` is HttpOnly and signed; contents are opaque. Never attempt to read
  or forge it.

### 4.3 CSRF

- Global token, not per-form: no `per_form_csrf_tokens` anywhere in `config/`.
- Two clean sources:
  - `GET /token_dispenser.json` → `{"token": "…"}` (`app/controllers/home_controller.rb:40-44`).
  - `<meta name="csrf-token" content="…">` on every page
    (`app/views/layouts/_includes.html.erb:11`).
- The token is bound to the session: fetch it **after** login and after any session
  rotation. It is masked by Rails, so it is not byte-stable across requests — treat it as
  valid-until-expiry, not reusable-by-value.
- `protect_from_forgery with: :exception, prepend: true`
  (`application_controller.rb:4`).
- Flash slots are `error`, `caution`, `notice`, `alert`
  (`app/views/layouts/session.html.erb:42`), rendered as `class="flash <slot>"`; a parser
  that only matches `flash error` misses Devise's login failure, which uses `alert`.

### 4.4 Cookies that change behaviour

| cookie | effect | evidence |
| --- | --- | --- |
| `_otwarchive_session` | identity | `session_store.rb:13` |
| `user_credentials` | nginx cache hint; missing ⇒ forced logout | `application_controller.rb:148-166` |
| `view_adult` | adult-content consent | `application_controller.rb` `see_adult?` |
| `admin_credentials` | marks an admin session | `application_controller.rb:130-138` |

### 4.5 Throttles (hard numbers)

| scope | limit | evidence |
| --- | --- | --- |
| all requests, per IP | 300 / 300 s | `config/config.yml:209-210`, `rack_attack.rb:52-59` |
| login, per IP | 5 / 20 s | `config.yml:211-212`, `rack_attack.rb:70-78` |
| login, per login string | 5 / 20 s | `rack_attack.rb:80-90` |
| comment create, per logged-in user | 3 / 900 s (authors exempt) | `config.yml:229-230`, `comments_controller.rb:57-64` |
| nginx per-session user | 300 / 300 s | `config.yml:215-226` |

429 responses carry `Retry-After` (`rack_attack.rb:112`). The comment limit is a Rails
`rate_limit` and redirects HTML clients to `/429` (`comments_controller.rb:57-89`).

### 4.6 Edge

Comments persist Cloudflare signals — bot score, JA3, JA4
(`app/controllers/comments_controller.rb:404-406`, `app/models/comment.rb:27,149-151`).
This is direct evidence that AO3 sits behind Cloudflare bot management and that a
JavaScript-less client will be scored. A challenge or an HTML interstitial is a normal
failure mode, not an anomaly.

---

## 5. Write surface

Full contracts live in `ao3-write-api.md`; this is the map.

| action | verb + path | params | source |
| --- | --- | --- | --- |
| kudos | `POST /kudos` | `kudo[commentable_id]`, `kudo[commentable_type]` | `routes.rb:605`; `kudos_controller.rb:100-102` |
| comment | `POST /works/:id/comments` | `comment[pseud_id]`, `comment[comment_content]`, `name`, `email` | `routes.rb:417`; `comments_controller.rb:772-777` |
| reply | `POST /comments/:comment_id/comments` | same + parent from URL | `routes.rb:573,590` |
| subscribe | `POST /users/:uid/subscriptions` | `subscription[subscribable_id]`, `[subscribable_type]` | `routes.rb:343`; `subscriptions_controller.rb:89-93` |
| unsubscribe | `DELETE /users/:uid/subscriptions/:id` | — | `routes.rb:343`; `subscriptions_controller.rb:48-58` |
| bookmark | `POST /works/:id/bookmarks` | `bookmark[pseud_id]`, `[bookmarker_notes]`, `[tag_string]`, `[collection_names]`, `[private]`, `[rec]` | `routes.rb:402`; `bookmarks_controller.rb:335-339` |
| mark for later | `PATCH /works/:id/mark_for_later` | — | `routes.rb:397`; `works_controller.rb:704-712` |
| mark as read | `PATCH /works/:id/mark_as_read` | — | `routes.rb:398`; `works_controller.rb:714-722` |
| favorite tag | `POST /users/:uid/favorite_tags` | `favorite_tag[tag_id]` | `routes.rb:299`; `favorite_tags_controller.rb:46-50` |
| mute | `POST /users/:uid/muted/users` | `muted_id` (byline) | `routes.rb:370`; `muted/users_controller.rb:75-79` |
| block | `POST /users/:uid/blocked/users` | `blocked_id` (byline) | `routes.rb:360`; `blocked/users_controller.rb:75-79` |

Three later write slices are not repeated in this map: work creation/publishing
(`POST /works`, `PUT /works/:id/post_draft`, `DELETE /works/:id`), chapter
create/update, and work metadata/tag editing (`PATCH /works/:id`, the `update`
action guarded by `check_ownership_or_admin` —
`works_controller.rb:16,354-394`, permitted by `work_params` at `:911-934`; its
form is `works/edit.html.erb` → `_standard_form.html.erb`). Chapters are a
**nested resource** under the work
(`routes.rb:403-414`): `POST /works/:work_id/chapters` creates one and
`PUT /works/:work_id/chapters/:id` updates it, with the fields from
`chapters_controller.rb:268-272` and the button names from
`chapters/_posting_fieldset.html.erb:11-28`. The full contracts are in
[`ao3-write-api.md`](ao3-write-api.md) §4.8–§4.13.

---

## 6. Policy

### 6.1 robots.txt

`public/robots.public.txt` (served in production):

```
User-agent: *
Disallow: /works?            # line 4 — the work index with query params
Disallow: /autocomplete/     # line 5
Disallow: /downloads/        # line 6
Disallow: /external_works/   # line 7
Disallow: /bookmarks/search?
Disallow: /people/search?
Disallow: /tags/search?
Disallow: /works/search?     # line 12
```

Notes:

- `/downloads/` (the official EPUB/MOBI/PDF export) is **disallowed for crawlers**. This
  is why the earlier audit's EPUB-first plan is superseded (ADR-0004).
- `/works?` blocks the *index*; a work page is `/works/12345` and is not matched by that
  rule. `?view_full_work=true` on a work page is likewise outside the pattern.
- `robots.public.txt` is for the public site; `robots.private.txt` is all-disallow and is
  used on private deployments. Do not infer policy from the private file.

### 6.2 Terms of Service FAQ

The relevant sentence, verbatim:

> "The use of bots or scraping for purposes that do not violate our policies is generally
> allowed. However, we reserve the right to implement robots.txt or other protocols
> limiting what bots can do, or to notify you and ask you to discontinue if a bot or
> scraping program is causing problems for the site. At our discretion, we may also ban
> specific bots or other programs from accessing AO3."
>
> — `app/views/home/tos_faq.html.erb:1180`

This is permission-shaped, not permission: it is conditional on policy compliance, it
reserves robots as a limiter, and it reserves the right to ask us to stop. The client must
behave as a polite, identifiable, single-purpose agent that can be shut off.

---

## 7. What is *not* available

| thing | status | evidence |
| --- | --- | --- |
| public JSON API for users | does not exist | only `Api::V2`, see below |
| `Api::V2` | admin/archivist only: `Authorization: Token` + `ApiKey`, keys minted in admin UI | `app/controllers/api/v2/base_controller.rb:9-23`; `app/controllers/admin/api_controller.rb:21-28` |
| OAuth / personal access tokens | none | `config/routes.rb` has no OAuth; devise only |
| webhooks / push notifications | none | — |
| JSON responses for kudos | no (HTML or JS) | `kudos_controller.rb:44-72` |
| JSON responses for comments | no (HTML or JS) | `comments_controller.rb` `create` only `format.html` |
| JSON responses for subscriptions | **yes** (`.json` 201/422) | `subscriptions_controller.rb:26-46` |
| JSON responses for favorite tags | **yes** | `favorite_tags_controller.rb:9-44` |
| JSON responses for bookmarks | **no** | `bookmarks_controller.rb` responds HTML |
| downloading via `/downloads/` automatically | robots-disallowed | `robots.public.txt:6` |

The practical rule: **subscriptions and favorite tags are the only clean JSON writes.**
Everything else is an HTML form with a redirect, and outcome detection is a parsing
problem.

---

## 8. Where the truth lives (and how it rots)

- `spec/controllers/kudos_controller_spec.rb` — exact error text:
  "You have already left kudos here. :)", "You can't leave kudos on your own work.",
  "Sorry, you have been blocked by one or more of this work's creators."
  (`:88-91`, `:110-114`, `:136-140`).
- `spec/controllers/comments/` — moderation, freeze, hide, blocking, replies.
- `spec/controllers/subscriptions_controller_spec.rb` — JSON envelopes.
- `config/locales/controllers/en.yml` — user-facing strings; if a message moves, the
  classifier must move with it.

Drift risks, ranked:

1. **Form field renames** (e.g. `comment[comment_content]`) — caught by the fixture tests.
2. **New required hidden fields** (Rails `form_with` additions) — caught by the sandbox
   run, not by fixtures alone.
3. **Redirect target changes** — caught by outcome fixtures.
4. **CSRF scheme change** (`per_form_csrf_tokens` switched on) — the token dispenser stops
   being sufficient; the fallback is form parsing. Detection: `/auth_error` responses.
5. **Route changes** — caught by `scripts/check-ao3-routes.sh`.

---

## 9. Local sandbox

`docker-compose.yml` defines MariaDB 10.5, Redis 7, Elasticsearch 9.3 and a web/test
container; `script/docker/init.sh` creates config and databases. This is the only safe way
to exercise real write flows against real Rails validations.

Cost: heavy (ES alone is configured at 1.5 GB heap). Treat it as opt-in
(`AO3I_SANDBOX=1`), never part of default CI, and never point the client at production in
tests.

Fallback if the stack cannot run here: fixture-driven parsers plus a mock upstream that
enforces the same form shape. That covers request construction but not Rails-side
behaviour; the sandbox remains the acceptance environment for W6.

---

## 10. How to regenerate this document

```sh
cd otwarchive
git log -1 --format='%H %ad' --date=iso
grep -n "resources :kudos" config/routes.rb
grep -n "def comment_params" -A 6 app/controllers/comments_controller.rb
grep -n "rate_limit to:" -A 6 app/controllers/comments_controller.rb
grep -n "RATE_LIMIT" config/config.yml
grep -n "Disallow" public/robots.public.txt
grep -n "bots or scraping" app/views/home/tos_faq.html.erb
```

Or run `scripts/check-ao3-routes.sh`, which automates the assertions and prints the pin.

## 11. TODO(verify)

- [ ] Whether a logged-in user can leave kudos from a chapter URL (the spec only covers
      guests from chapters: `kudos_controller_spec.rb:35-46`).
- [ ] Exact behaviour when a work is restricted and the session is stale midway through a
      write (login redirect vs `/auth_error`).
- [ ] Whether `view_adult=1` is required for kudos/comments on adult works or only for
      viewing.
- [ ] Subscription duplicate behaviour at the DB level (no unique index exists; confirm on
      the sandbox whether `save` raises or silently duplicates).
- [ ] Whether the `remember_me` cookie alone can revive a session after
      `_otwarchive_session` expiry.
