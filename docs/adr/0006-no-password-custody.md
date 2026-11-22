# ADR-0006 — No password custody

- **Status:** Accepted (2026-11-22)
- **Supersedes:** the "store the password encrypted" part of ADR-0001 option B
- **Related:** ADR-0001, `docs/security-standards.md` §6

## Context

ADR-0001 offered two ways in: cookie import (default) and credential login with the
password encrypted at rest so sessions could be renewed automatically. The login path
is now implemented, but persisting a password was never built.

## Decision

**Password login works; the password is never persisted.**

- Login sends the credentials once, captures the rotated cookie jar, encrypts the jar
  and discards the password. The session then behaves exactly like an imported one.
- When AO3 expires the session (2 weeks, 3 months remembered), the operator
  re-authenticates. There is no background credential use.
- The `sessions` table stores the encrypted jar and the default pseud; there is no
  password column, and a test asserts a restart restores only the jar.

## Why

1. AO3 has no token API: a stored password is a long-lived credential that can post
   publicly as the user. The blast radius of a leaked jar is bounded by AO3's session
   lifetime; the blast radius of a leaked password is not.
2. Cookie import is frictionless enough (copy two values once) and is the documented
   default.
3. No password means no "silent re-login" logic, no failed-credential lockout risk on
   AO3's side, and no phishing-shaped storage.

## Consequences

- Sessions end when AO3 says they end; the UI says "re-authenticate" instead of
  pretending.
- The credential path is a convenience, not an availability guarantee.
- If automatic renewal is ever wanted, it needs a new ADR that confronts the blast
  radius above.

## Test

- `sessions` has no password column (migration 0006 is the last schema change and adds
  only pseud columns).
- Login stores only the jar: the persistence test decrypts the jar with the key file
  and finds no credential material.
