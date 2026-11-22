---
name: inherit
description: Reuse existing code instead of duplicating it. Use before writing a new function, pipeline, module, or config, and whenever the same logic appears more than once.
---

# Inherit

**Inherit is best.** Before creating anything, check whether it already exists. If it does, reuse it. If something is used more than once, make it reusable so we never maintain two copies of the same thing.

## Before you write

Search first. Look for an existing function, pipeline, module, helper, or config that already does the job (or most of it):

- Grep the codebase for the behavior, not just the name.
- Check shared modules, utilities, and the Nix expressions we already expose.
- Check whether a dependency we already have covers it.

If you find something close, extend or generalize it. Do not write a parallel version next to it.

## Rule of two

The first time, write it where it is needed. The **second time** you need it, stop and extract it:

1. Move the logic into a single shared place (a function, module, library, or Nix attribute).
2. Make it take parameters for what differs instead of hardcoding.
3. Point both call sites at the shared version.

Copy-pasting is the signal. If you are about to duplicate a block and tweak a line, parameterize it instead.

## Make it recyclable

When you create something likely to be reused:

- Give it **one clear job** and a name that says what it does.
- Keep inputs explicit. Avoid hidden globals and hardcoded paths.
- Keep it small and composable so it fits into other pipelines.
- Put it somewhere discoverable, not buried inside the first caller.

## In Nix

Prefer inheritance over repetition: share common attributes through `let` bindings, `inherit`, overlays, and shared modules. Define a package, option, or derivation once and reference it everywhere, instead of redefining it per host or per project.

## Avoid

- Two functions with near-identical bodies.
- Per-project copies of the same script or pipeline.
- Premature abstraction: do not invent a generic framework for a single use. Wait for the second use, then extract.

## Check before finishing

- Did I search for an existing version first?
- Is anything I wrote duplicated elsewhere?
- If I wrote something reusable, can the next caller find and use it without copying?

Fewer copies mean fewer bugs and one place to fix them.
