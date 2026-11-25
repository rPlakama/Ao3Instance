# ADR-0003 — v1 write scope

- **Status:** Accepted (2026-11-22)
- **Date:** 2026-11-22
- **Supersedes:** `docs/init-audit.md` §2 D3 and D4 (read-only v0)
- **Related:** ADR-0005, `docs/ao3-write-api.md`

> **Amended (2026-11-25).** Authoring is in scope again; see
> [ADR-0008](0008-work-creation.md). New work creation, chapter addition/editing, the
> My Works/Drafts manager, publishing a draft and deleting a work are implemented. The
> "works/chapters creation" item in the deferral table below now covers only the remaining
> authoring slices: chapter deletion/reordering, work metadata/tag editing, co-creators and
> scheduling.

## Context

The owner's objective is a client that posts kudos, comments, subscriptions and more. AO3
offers a large action surface — works and chapters, collections, gifts, moderation, mute
and block, reading history — with wildly different risk and complexity.

Reading a fic needs five verbs; administering an archive needs fifty.

## Decision

**v1 ships:**

| action | reason |
| --- | --- |
| kudos | simplest server-unique write; good first vertical slice |
| comments (top-level + reply) | the core social act; highest care needed |
| subscribe / unsubscribe (Work, User, Series) | clean JSON API; high value for a reader |
| bookmark | natural "save with notes" primitive |
| mark for later / mark as read | pairs with reading progress |
| favorite tags | clean JSON API; cheap |

**Deferred, with reasons:**

| action | defer because |
| --- | --- |
| works/chapters creation | enormous param surface (tags, warnings, collections, skins); a posting client is a different product |
| collections, gifts, challenges | requires understanding archive policy, not just endpoints |
| comment moderation (approve/reject/freeze/hide) | only meaningful to work creators; authorization nuances |
| mute/block | ethically sensitive; needs a product decision about visibility |
| reading history clearing | destructive; needs a confirm flow of its own |
| any admin API | keys are admin-only; not part of a reader |

## Consequences

- The first vertical slice is subscriptions (JSON, easy to verify), then kudos, then
  reading flags, then bookmarks, then comments.
- Upsert/edit flows (comment editing within 0 replies, bookmark notes) are v1.1; v1 creates
  and deletes only where the API is a create/delete pair.
- Every deferred action needs a new ADR that answers the same questions: preconditions,
  confirmation, idempotency, rate budget.

## Test

- The write matrix in `ao3-write-api.md` matches the implemented routes exactly; a route
  without a row fails the docs gate.
- Every shipped action has a fixture for success and each failure mode in §6.
