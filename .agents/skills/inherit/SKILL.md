---
name: inherit
description: >-
  Reuse existing code, helpers, modules, and Nix expressions instead of duplicating them.
  Use before creating a new function, pipeline, module, parser, renderer, route guard, or Nix
  attribute, and whenever the same logic appears more than once.
---

# Inherit

**Inherit is best.** Before creating anything, check whether it already exists. If it does, reuse it. If something is used more than once, extract it so we never maintain two copies of the same thing.

## 1. Search First

Before writing new code, search for an existing helper, module, constant, or Nix expression that already covers the behavior (or most of it):

- Grep the codebase for the **behavior**, not just the name.
- Check the repository's shared modules catalog in [references/shared-helpers.md](./references/shared-helpers.md).
- Check `flake.nix` bindings and existing crate dependencies before adding new helpers or crates.

If you find something close, extend or parameterize it rather than writing a parallel copy next to it.

## 2. Rule of Two

The first time you need logic, write it where it is used. The **second time** you need it, stop and extract it:

1. Move the logic into a single shared, discoverable home (a helper function, shared module, constant table, or Nix `let` binding).
2. Parameterize what differs instead of hardcoding caller-specific details.
3. Point both callers (and any other existing duplicates) at the shared version.

Copy-pasting and tweaking a line is the signal to parameterize instead.

## 3. Make It Recyclable

When creating a shared helper or module:

- Give it **one clear job** and a name that states what it does.
- Keep inputs explicit; avoid hidden globals or hardcoded paths.
- Keep modules focused and cohesive — split large files along domain boundaries when they accumulate unrelated responsibilities.
- Place shared code somewhere discoverable (e.g., `ao3::html`, `render::components`, `consts`), not buried inside the first caller.

## 4. In Nix

Prefer inheritance over repetition:

- Share common dependency lists, environment variables, and evaluation helpers through `let` bindings and `inherit`.
- Define a package, option, or module evaluation helper once and reference it across packages, checks, and dev shells instead of redefining it per output.

## 5. Avoid

- Two functions or handlers with near-identical bodies (e.g., repeated guard checks, retry loops, HTML banners, or SQL row mappers).
- Per-module copies of parsing utilities (`collapse_text`, number parsing, ID extraction).
- Premature abstraction: do not build a generic framework for a single use — wait for the second use, then extract.

## 6. Check Before Finishing

- Did I search for an existing implementation first?
- Is anything I wrote duplicated elsewhere (or did I leave an older copy behind)?
- If I created a shared helper, is it discoverable and listed in [references/shared-helpers.md](./references/shared-helpers.md) if broadly useful?
- Do formatting (`cargo fmt --check`), lints (`cargo clippy --all-targets -- -D warnings`), and tests (`cargo test`) all pass?

Fewer copies mean fewer bugs and one place to fix them.
