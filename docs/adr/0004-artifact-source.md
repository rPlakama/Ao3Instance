# ADR-0004 — Where the EPUB comes from

- **Status:** Accepted (2026-11-22)
- **Date:** 2026-11-22
- **Supersedes:** `docs/init-audit.md` §2 D7 and §5.3 (EPUB-first via `/downloads/`)
- **Related:** ADR-0002

## Context

The audit proposed ingesting AO3's official EPUB export at `/downloads/:id/:title.epub`.
The clone shows two problems:

1. `public/robots.public.txt:6` disallows `/downloads/` for all user agents. Automating it
   contradicts the posture in ADR-0002.
2. The export is a user-facing browser action; the server route is not designed as an API
   and has no JSON/contract stability.

Meanwhile the work page itself is not disallowed: `/works/:id` and
`/works/:id?view_full_work=true` are outside the `/works?` rule (that pattern targets the
index query).

## Options

| option | fidelity | policy | effort |
| --- | --- | --- | --- |
| A. Automate `/downloads/` | highest | violates robots | low |
| B. Build the EPUB from work HTML | good | compliant | medium (packager + sanitizer) |
| C. B + browser handoff for AO3's export | both | compliant | medium + one link |
| D. Ask OTW for an exception | highest | requires permission | unknown lead time |

## Decision

**C.**

- The server fetches the work page the user is reading and builds an EPUB 3 locally:
  chapter split from the page (or `view_full_work=true` for the full text), sanitized
  XHTML, cover image, metadata from the work header.
- The container blob is content-addressed (`sha256`) and becomes the offline artifact.
- AO3's own export is offered as a **user handoff**: a link the browser opens. The server
  never requests `/downloads/`.
- If the owner later obtains explicit permission (option D), a follow-up ADR may enable
  the export path; until then it is a one-line grep gate.

## Consequences

- We own EPUB correctness: valid OPF/spine, stable ids, deterministic ordering, no
  external references, images embedded or excluded by policy.
- Chapter/notes/series metadata comes from the page parser; the parser becomes more
  important than it would have been with the export.
- Fidelity differs from AO3's EPUB (work skins, some notes formatting). Documented as a
  known divergence; the reader's source of truth is still the fetched HTML.
- The packager is testable offline: HTML fixture in, EPUB out, `epubcheck` clean.

## Test

- `grep -r "/downloads/" src/` returns nothing (gate in CI).
- A fixture work produces an EPUB whose chapter count and order match the page, validated
  by `epubcheck` (or a Rust EPUB validator if `epubcheck` is unavailable).
- The same work fetched twice produces the same `sha256` (determinism), and an updated
  work produces a different one.
