# ADR-0001 — AO3 authentication model

- **Status:** Accepted (2026-11-22); password-at-rest superseded by
  [ADR-0006](0006-no-password-custody.md) (login works, password is not persisted)
- **Date:** 2026-11-22
- **Supersedes:** nothing
- **Related:** ADR-0002, ADR-0005, `docs/security-standards.md` §6

> **W1 scope note (2026-11-22).** Cookie import and password *login* are
> implemented and tested. Password *at rest* (AAD `ao3/password`) and the
> three-failure parking rule land with persistence in W3 — there is no local
> store yet to put a ciphertext in. The test list at the bottom is the acceptance
> criterion for that milestone, not for W1.

## Context

The client acts as a real AO3 user: kudos, comments, subscriptions and reading flags are
all attributed to an authenticated account. AO3 exposes no OAuth and no user API tokens
(`docs/ao3-internals.md` §7); Devise session cookies are the only credential.

Two ways in:

1. **Cookie import** — the user copies `_otwarchive_session` and `user_credentials` from
   their browser. No password ever reaches us. Sessions last 2 weeks (3 months
   remembered).
2. **Credential login** — the user gives us their AO3 username and password; we post
   `/users/login`, capture the rotated jar, and store the password encrypted so we can
   re-authenticate when the session expires.

The MUSIKA precedent stores server passwords encrypted (`musika/src/crypto.rs`), so both
are implementable to the same standard.

## Options

| option | pros | cons |
| --- | --- | --- |
| A. Cookie import only | no password custody; smallest blast radius; trivial threat model | re-import every 2 weeks; confusing for non-technical users; user must find an HttpOnly cookie |
| B. Credential login only | best UX; survives expiry; "just works" | we hold a password that can post publicly; phishing value; pwned-password flow; login throttles |
| C. Both, cookie-import default | A's safety as default; B available for users who want it | two code paths and two threat stories; more tests |

## Decision

**C.** Ship both, default to cookie import.

- Import is validated by an authenticated read before being marked active
  (`GET /users/:uid/subscriptions` → 200). A jar that lands on `/lost_cookie` is
  rejected, not "fixed".
- Credential login is an explicit opt-in, stores the password with AES-256-GCM under AAD
  `ao3/password`, and is throttled locally to ≤ 1 attempt / 20 s with a 15-minute cool-off
  after 3 failures.
- The CSRF token is never persisted. The jar is encrypted with AAD `ao3/jar`.
- Logout and jar-invalid events wipe stored material immediately.

## Consequences

- The session module must model both origins (`Imported`, `Password`) and both failure
  shapes (`SessionIncomplete` from `/lost_cookie`, `LoggedOut` from `/users/login`).
- Users who import must be able to re-import in under a minute; the UI must explain where
  the cookie comes from without encouraging copy-paste into untrusted tools.
- Password login inherits AO3's `pwned?` forced-password-change flow: that redirect is a
  distinct outcome, not a success.
- No refresh tokens, no background re-login. A write is never attempted with a stale
  session; the user is asked to re-auth.

## Test

- Import: valid jar → `Active`; missing `user_credentials` → `SessionIncomplete`.
- Password: success stores ciphertext whose plaintext equals the input; three failures
  park the account; the plaintext is absent from logs and API responses.
- Both: logout wipes the jar and the password; the next write returns `LoggedOut` without
  a network call.
