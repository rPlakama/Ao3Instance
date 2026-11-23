# Failure matrix

What the user sees when the world is not cooperating. Every row is backed by a
typed outcome and, where marked, an integration test.

| situation | detection | user-visible result | audit | test |
| --- | --- | --- | --- | --- |
| AO3 unreachable (cached work) | no request needed | work reads from SQLite | — | `failure_matrix::cached_work_survives_an_unreachable_upstream` |
| AO3 unreachable (new work) | connect error | 502 "request failed", nothing cached | — | same |
| Live search blocked | 403/503/429 or timeout | one solver solve of the same result page; if unavailable, search-block page | `solver_event` | `ao3_search_live`, `search-parity.md` |
| Search offline | local FTS only (`?local=1`) | cached hits; explicit empty state | — | `search_and_refresh` |
| 429 rate limit | HTTP 429 / `Retry-After` | "rate limiting … retry after Ns" | write: failed | `writes_comments::spam_and_rate_limits…` |
| 403 / 503 edge block | status | "refused the request; may be a bot challenge"; solve card if the solver is reachable | write: failed | `failure_matrix::blocked_upstream_is_explained` |
| Challenge solved | explicit `POST /solve` (works) or blocked search | page cached/reads, or search results rendered; one page per user action | `solver_event`: solved | `flaresolverr::solve_caches_the_page…` |
| Solver absent | connect error on `/v1` | page still explains the block; `/solve` → 503 | `solver_event`: unavailable | `flaresolverr::absent_solver_degrades…` |
| Solver captcha | `status:"error"` message | "hit a captcha it cannot solve" | `solver_event`: captcha | `flaresolverr::captcha_is_typed…` |
| Solve budget / cooldown | local counters | 429 with the exact wait | — | `flaresolverr::solve_caches…` (cooldown) |
| CSRF expired | 302 `/auth_error` | "AO3 rejected the CSRF token; verify the session" | write: failed | `writes_kudos_reading::reading_flag_csrf_failure…` |
| Logged out | 302 `/users/login` | "session is not usable" | write: failed | `session_flow::probe_recognises_logged_out…` |
| Incomplete jar | 302 `/lost_cookie` | "cookie jar incomplete" | — | `session_flow::probe_recognises_incomplete…` |
| Adult gate | gate markers, no title | "enable adult content on the Session page" | — | `session_and_progress::adult_gate…` |
| Restricted work | 302 on fetch | "sign in on the Session page" | — | `failure_matrix::blocked_upstream…` (redirect path) |
| Kudos duplicate | 422 `already left kudos` | "you had already left kudos" | already-done | `writes_kudos_reading::already_given…` |
| Comment duplicate | 200 duplicate text | "AO3 rejected the comment" + audit evidence | failed, one attempt | `writes_comments::duplicate_comment…` |
| Comment spam | 200 spam text | "spam filter rejected the comment" | failed | `writes_comments::spam_and_rate…` |
| Comment >10k chars | local guard | 400 badge, no request | — | `writes_comments::overlong_or_unconfirmed…` |
| Missing confirmation | local guard | 400, no request | — | same |
| Duplicate subscription / bookmark / kudos | local receipt/ledger | "already … according to the local ledger" | — | `writes`, `writes_bookmarks`, `writes_kudos_reading` |
| Writes disabled | `AO3I_WRITE_ENABLED` | 403 "writes are disabled" | — | `writes::writes_require_the_switch…` |
| Upstream HTML drift | parser returns `UpstreamChanged` | 502 "upstream page changed: …" | — | parser tests |
| Any upstream/inbound 4xx/5xx | diagnostics middleware | normal page, redacted row at `/diagnostics` | `http_error_log` | `tests/http_diagnostics.rs` |
| Stale cache | `fetched_at` > 7 days | "checked … (N days ago — stale)" | — | `render::tests` |

Principles: a read never hangs on upstream (cached answer or typed miss); a write is
never retried when the outcome is ambiguous; an ambiguous failure surfaces in the
audit log instead of being guessed.
