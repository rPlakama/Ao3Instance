# ADR-0002 — Access posture toward AO3

- **Status:** Accepted (2026-11-22)
- **Date:** 2026-11-22
- **Supersedes:** `docs/init-audit.md` §2 D2 (upstream access) and the "crawl" half of D7
- **Related:** ADR-0004, `docs/ao3-internals.md` §6, `docs/write-api.md` §8

## Context

The clone gives us both the technical and the policy picture:

- `public/robots.public.txt` disallows `/downloads/`, `/works?`, `/works/search?`,
  `/autocomplete/`, `/external_works/` and the search endpoints.
- The ToS FAQ says bots and scraping are "generally allowed" when they do not violate
  policy, but AO3 reserves the right to limit them via robots.txt and to ask operators to
  stop (`app/views/home/tos_faq.html.erb:1180`).
- Comments persist Cloudflare bot score, JA3 and JA4
  (`app/controllers/comments_controller.rb:404-406`), so a JavaScript-less client is
  scored and may be challenged.
- Global per-IP throttle is 300 / 300 s; login 5 / 20 s; commenting 3 / 900 s.

The earlier audit assumed broad fetching plus `/downloads/` automation. That is no longer
defensible on AO3's own published terms.

> **W1 implementation note.** The request-spacing floor
> (`AO3I_MIN_SPACING_MS`, default 2 s), honest UA, and the terminal `Killed`
> state are implemented. The governor's per-action budgets and the request audit
> log land with the write pipeline in W3.

## Decision

The client is a **user agent**, not a crawler.

1. **User-initiated only.** Every upstream request traces to a human action in the UI (a
   page for a work the user opened, a button the user pressed). No background crawling, no
   prefetch of arbitrary works, no scheduled sync of someone's bookmarks.
2. **No automated `/downloads/`.** The export endpoint is robots-disallowed; it is never
   fetched by the server. See ADR-0004 for the replacement.
3. **No automated search.** Upstream search is a user-typed, single query, debounced and
   cached; never a paging crawl. Local FTS covers discovery of already-cached works.
4. **One request in flight**, ≥ 2 s spacing, jitter, serialized per account, `Retry-After`
   honoured, immediate stop on `403`/`429`/challenge.
5. **Honest identity.** UA `AO3Instance/<ver> (+<project-url>; contact=<email>)`. No
   spoofed browser strings.
6. **Kill switch** `AO3I_WRITE_ENABLED` (default false until W6) plus a runtime
   "stop all upstream traffic" control (`AccountSession::kill`, implemented in W1), so an
   operator can comply with a request to stop without redeploying. Before public use the
   operator must set `AO3I_USER_AGENT` to the documented format (`+<project-url>;
   contact=<email>`); the default UA does not invent a contact address.
7. **Audit trail** of every upstream request (action, target, outcome, duration) so
   compliance can be demonstrated rather than asserted.

## Consequences

- Features that would need bulk fetching are out of scope by construction (library-wide
  import, author-complete archives, tag sweeps).
- The reader must fetch on demand and cache; a work opened once is instant offline, a work
  never opened is a typed miss.
- Search quality depends on what the user already cached; that is a product decision, not
  a bug.
- An upstream challenge is a first-class outcome (`BlockedByEdge`), not an error to retry.

## Test

- A route walk that asserts no network call happens without a user action.
- A budget test: two concurrent actions serialize; minimum spacing is respected.
- A kill-switch test: `AccountSession::kill()` makes every network method return
  `Error::Killed` with zero requests (implemented and tested in W1). The
  `AO3I_WRITE_ENABLED` construction gate is W6, once write routes exist.
