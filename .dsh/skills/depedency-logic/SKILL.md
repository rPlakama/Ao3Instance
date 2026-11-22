---
name: dependency-logic
description: Decide how to add a dependency. Use before adding a crate, library, or external tool to a project.
---

# Dependency logic

**Less is better, except when more is faster.**

## Default: use the system

If a capability already exists as a system tool or package, use it instead of pulling in a crate. We build with Nix, so declare the tool as a **runtime dependency** of the application and call it directly.

Example: need to convert images? Add `imagemagick` to the package's runtime inputs and shell out to it. Don't add a Rust image crate.

## Exception: when more is faster

Add a crate or write our own solution when it is clearly faster, meaning one of:

- Calling the tool is a measurable bottleneck (process spawn per call, hot loop, large batches).
- The crate replaces a pile of glue code, parsing, or error handling.
- A small in-house implementation (a few dozen lines) beats both.

## Decision order

1. Does a system tool already do this? Use it as a Nix runtime dependency.
2. Is it too slow or too awkward that way? Use a crate.
3. Is the crate heavy for what we need? Write it ourselves.

Each new dependency is a cost: build time, supply chain, upkeep. Add one only when it pays for itself in speed.
