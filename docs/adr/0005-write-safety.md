# ADR-0005 — Write safety: confirm, record, verify, never guess

- **Status:** Accepted (2026-11-22)
- **Date:** 2026-11-22
- **Related:** ADR-0001, ADR-0003, `docs/ao3-write-api.md` §6–§7

## Context

AO3's write outcomes are not always machine-readable:

- kudos success and failure are both `302`, distinguished only by the flash and the
  followed page (`kudos_controller.rb:44-72`);
- comments fail with a `200` re-render, a `302` to `/429`, or a `302` to `/auth_error`;
- subscriptions have no uniqueness constraint, so a retry can create a duplicate
  (`db/schema.rb:1119-1127`);
- a duplicate public comment is worse than a failed one.

A client that treats "no exception" as success will double-post and then lie about it.

> **W1 implementation note.** The transport never retries, and the session
> layer refuses to act unless `Active`. The `write_intent`/`write_audit` tables
> arrive with persistence in W3; until then no write path exists to guard.

## Decision

Four rules, applied per action.

1. **Confirm.** The UI names the target and the effect; nothing posts as a side effect of
   navigation, refresh, or session restore. Public actions (comments, kudos) need an
   explicit click that says what will happen.
2. **Record intent before send.** A `write_intent` row is written and committed *before*
   the HTTP request: account, action, target, a hash of the payload, timestamp, idempotency
   class. The audit row is completed after the outcome.
3. **One attempt, typed outcome.** The request is sent once. The outcome is classified by
   the precedence in `ao3-write-api.md` §5 (JSON envelope → final URL → body marker →
   verification read). Timeouts are `Ambiguous`, not failures.
4. **Verify, don't retry.** On ambiguity, the client performs a read that can prove the
   effect (kudos state, subscription list, comment id). Only `ServerUnique` and
   `ServerConstrained` actions may be retried, and a duplicate is mapped to `AlreadyDone`.
   `ClientGuarded` (comments, bookmarks) and `NoConstraint` (subscriptions) are never
   auto-retried.

## Consequences

- Some writes cost an extra read; that is the price of not lying.
- The UI needs a third state beyond success/failure: **unconfirmed**, with a "check"
  action.
- The intent log makes a crash mid-write diagnosable and prevents a restart from replaying
  blindly.
- The audit log is append-only and redacted at write time (security-standards §5).

## Test

Fault injection for each class:

| injected fault | expected outcome |
| --- | --- |
| response dropped after send (timeout) | `Ambiguous` → verification read → `AlreadyDone` or `Unconfirmed` |
| 500 after a successful save | verification read proves it landed |
| 429 | `RateLimited` with the server's `Retry-After`; no second request |
| CSRF rotated | `CsrfExpired`; kudos may retry after refresh, comments require re-confirmation |
| duplicate comment | `Validation` with the exact AO3 string |
| duplicate subscription | prevented by pre-check; a blind second POST test must show the server would have duplicated it |
| process killed between intent and response | on restart the intent is `Unconfirmed`, never replayed silently |
