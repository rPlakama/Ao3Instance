---
name: testing
description: Test changes thoroughly. Use whenever a function is added or changed, when a change touches other modules, and before any release.
---

# Testing

**Testing is critical.** MUSIKA core must be extremely reliable, and MUSIKA-UI is held to the same standard. Untested code is unfinished code.

## When to test

- You added or changed a function: test it.
- Your change touches other modules: test the interactions, not just the function in isolation.
- You fixed a bug: add a test that fails without the fix.

## Layers

Test at every layer that applies, from the inside out:

1. **Unit tests.** Cover each function, including edge cases and error paths, not just the happy path.
2. **Integration tests.** Verify modules work together correctly.
3. **IPC tests.** Exercise the messages and protocol between core and its clients: valid input, malformed input, and ordering.
4. **UI tests.** Drive MUSIKA-UI itself and verify real behavior. Use the `window-screenshot` skill to get a visual reference and confirm what is actually on screen.

## Standards

- Be thoughtful, not perfunctory. Ask what could break, then test that.
- Cover failure cases: bad input, missing files, dropped connections, empty states.
- Tests must be deterministic. No flaky tests, no reliance on timing or ordering luck.
- Run the full suite, not only the new tests, to catch regressions in other modules.
- Fix a failing test by fixing the code, never by weakening the test.

## Before finishing

- Does every new or changed function have a test?
- Did I test the modules it touches?
- Did I checked if I didn't break something?

Releases require tested binaries (see `release`). Reliable core, reliable UI.
