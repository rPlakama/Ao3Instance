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

## Amendment (2026-11-25) — AO3-parity direct posts for kudos and work subscriptions

**What changed.** The work reader now posts kudos and work subscriptions directly from the
work page, the way AO3's own page does: a `form.new_kudo` posts to `/write/kudos`, and the
`form.ajax-create-destroy` subscription form posts to `/write/subscribe` or
`/write/unsubscribe`. The reader renders those forms instead of links to `/write/preview`;
kudos and subscribe carry the existing hidden `confirm=yes` token, while unsubscribe needs
no token because it only removes a subscription with an active local-ledger row. Comments
and bookmarks still go through the confirmation page, and the reader now renders AO3's
`kudos/_kudos.html.erb`-shaped `#kudos` module from the cached count plus a "you left
kudos" receipt.

**Why.** The owner asked for AO3 design parity. AO3's work page leaves kudos and
subscribe/unsubscribe to a direct button click, so routing them through a preview page made
the two cheapest actions feel unlike the archive. A kudo is create-once (AO3 itself rejects
a duplicate), and unsubscribing is the undo of an action the ledger already tracks, so the
preview step added ceremony without adding safety.

**What did not change.** Rule 1 is still enforced per action, with the button click now the
confirmation: the server still refuses a kudos or subscribe request that lacks the hidden
`confirm=yes` token. The write gate (`AO3I_WRITE_ENABLED`), the usable-session requirement,
the `write_intent`/`write_audit` ledger written before and after the send, the governor (one
write at a time, spacing floor, per-action budgets), one attempt with a typed outcome, and
the same-origin guard on mutating routes all still apply. Comments and bookmarks keep both
the confirmation page and their stricter duplicate handling.

**Scope.** Reader kudos and work subscribe/unsubscribe only. No other action changes.

**Trade-off.** These two actions lose the preview page that named the target before the
request, so a mistaken click reaches AO3 directly; a mistaken kudo cannot be withdrawn
(AO3's own button behaves the same way), while a mistaken unsubscribe can be undone with
one subscribe click. Duplicate and abuse risk stays contained server-side: the local kudos
receipt and subscription ledger are checked before the write, a duplicate kudos maps to
`AlreadyDone`, unsubscribe refuses to act on an unknown subscription id, and every outcome
is audited and surfaced as a reader notice.
