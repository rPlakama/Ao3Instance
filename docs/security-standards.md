# Security standards

How AO3Instance handles secrets, sessions and untrusted input. The baseline is MUSIKA's
practice — cited by file:line — plus the additions this project needs because it holds
*other people's* credentials and posts publicly on their behalf.

Status: normative. A PR that violates a rule here needs an ADR that says why.

---

## 1. Principle

**The user's AO3 account is the asset. The client is the threat surface around it.**

Three consequences:

1. Secrets exist in exactly two places: in memory while in use, and encrypted at rest.
   Never in logs, never in error strings, never in JSON returned to a browser.
2. Every irreversibility (a public comment, a kudo) is a deliberate, confirmed act. The
   server never posts because a page loaded or a session refreshed.
3. Anything parsed from AO3 is untrusted. Anything rendered to a browser is untrusted
   twice.

---

## 2. Standards inherited from MUSIKA

| id | rule | MUSIKA evidence |
| --- | --- | --- |
| S1 | Secrets encrypted at rest with AES-256-GCM; nonce 12 B, tag 16 B, stored `hex(nonce‖tag‖ciphertext)` | `src/crypto.rs:26-28,95-141` |
| S2 | Key is 32 random bytes, hex-encoded, file mode `0600`, created with `create_new` (no overwrite race), parent dir ensured | `src/crypto.rs:65-74,184-203` |
| S3 | Key loaded once at startup by the caller; never re-read per request | musika `src/crypto.rs:144-163`; ours exposes `load_or_create` and the caller pins it |
| S4 | Key material zeroized on drop with volatile writes + compiler fence | `src/crypto.rs:40-50` |
| S5 | `.env` is allowlisted by prefix, denylisted for key paths, owner-only, non-symlink, not group/world-writable; CWD `.env` only in debug | `src/entry.rs:3-5,27-46,48-75` |
| S6 | Local IPC is user-private: dir `0700`, socket `0600`, `SO_PEERCRED` uid check | `docs/Daemon.md:89-93` |
| S7 | Pools closed before deleting files, because an open handle keeps unlinked inodes alive | `src/db.rs:432-436` |
| S8 | Wipe is an explicit allowlist, never a computed path, never `rm -rf` | `src/db.rs:444-488` |
| S9 | Strict lint floor: `unwrap_used` deny, `undocumented_unsafe_blocks` deny, `allow_attributes_without_reason` deny, `todo`/`unimplemented`/`dbg_macro` deny | `Cargo.toml:26-59`; `clippy.toml` |
| S10 | Every suppression states why; prefer `#[expect]` so stale allows fail the build | `Cargo.toml:34-35` |

We adopt S1–S10. Two are adapted (S6: our equivalent is the browser session boundary, not
a Unix socket) and one is strengthened (§3.3 AAD).

---

## 3. Secrets at rest

### 3.1 What is secret

| value | lifetime | storage |
| --- | --- | --- |
| AO3 password (only if credential login is enabled) | until changed | AES-256-GCM, AAD `b"ao3/password"` |
| AO3 cookie jar | until logout/expiry | AES-256-GCM, AAD `b"ao3/jar"` |
| CSRF token | session | **memory only**, never persisted |
| Local account password hash | permanent | Argon2id (not encrypted; hashed) |
| Local session token | session | random 256-bit, stored hashed |

### 3.2 The recipe

```rust
const KEY_LEN: usize = 32;      // AES-256
const NONCE_LEN: usize = 12;    // GCM standard
const TAG_LEN: usize = 16;

// hex(nonce || tag || ciphertext)
fn encrypt(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<String> {
    let mut nonce = [0u8; NONCE_LEN];
    rand_bytes(&mut nonce)?;                    // OS CSPRNG, never a counter
    let mut tag = [0u8; TAG_LEN];
    let ct = encrypt_aead(Cipher::aes_256_gcm(), key, Some(&nonce), aad, plaintext, &mut tag)?;
    Ok(hex(nonce ++ tag ++ ct))
}
```

Rules:

- Fresh random nonce per encryption. Never reuse a nonce with the same key.
- Authenticate with AAD that names the purpose. Musika passes empty AAD
  (`src/crypto.rs:107,135`); we bind the purpose so a jar ciphertext cannot be decrypted
  as a password, or vice versa.
- Decryption failure is a typed error. Never fall back to plaintext, never return partial
  data.
- Zeroization scope as implemented: key material (`SecretKey`), `SecretString` contents,
  the key-file read buffer, the decoded key bytes, and the jar's JSON/plaintext
  temporaries are wiped. `SecretKey::decrypt` still returns an ordinary `String` for
  `serde_json`; callers must wipe it (the jar does). This is documented rather than
  pretended away.
- The key is never logged, never exported by an endpoint, never accepted from the
  network.
- No key rotation in v1. Document it: rotating the key requires re-encrypting all rows in
  one transaction; a lost key means every stored secret must be re-entered. This is a
  recorded limitation, not a surprise.

### 3.3 Key file

Same as MUSIKA S2/S3/S4:

- default `$XDG_CONFIG_HOME/ao3instance/key`, override `AO3I_KEY_FILE` (which `.env`
  cannot set — S5);
- 64 hex chars, mode `0600`, `create_new(true)`, `sync_all` after write;
- loaded once at startup by the caller (pinned in the app state), zeroized on drop.
  W1 exposes `load_or_create`; the single-load `OnceLock` wrapper arrives with the app
  bootstrap in W3 — until then every caller is responsible for calling it once.

Refuse to start if the key file exists with wrong length or bad hex — do not silently
regenerate (that would strand every stored secret).

---

## 4. Configuration and environment

- One typed `Config`, read once at startup. `std::env` appears only in `config.rs`.
- Prefix `AO3I_`. Anything security-relevant is denylisted from `.env`:
  `AO3I_KEY_FILE`, `AO3I_DATA_DIR`, any `*_SECRET*`, `*_TOKEN*`.
- `.env` handling ports S5 verbatim: allowlist by prefix, owner-only, non-symlink, not
  group/world-writable. A `.env` that fails the check is ignored with a warning, not
  trusted.
- Secrets may also arrive via environment for container deployments; document that the
  environment is readable by same-user processes and prefer the file for long-lived
  secrets.

---

## 5. Logging and redaction

- A newtype `SecretString` whose `Debug`/`Display` prints `[redacted]`; passwords and CSRF
  tokens are only ever held in it. Types that must hold a cookie value (`Jar`, `Cookie`,
  `Response`) never `Debug`/`Serialize` that value and wipe it on drop.
- No request bodies at `info`. A debug log of a write may include the action, target id
  and outcome — never the comment text, password, cookie or token.
- Error responses to the browser carry a stable code and a human message; never the
  upstream body, never headers.
- Panic payloads must not contain secrets: no `unwrap` on a `Secret`-bearing `Result`.
- Redact in HTTP client traces too (`tracing` fields), not only in our own logs.
- Unit test: serialize every error variant and assert no secret field appears.
  (Planned with the docs-parity gate G7; W1 tests redaction on the types that carry secrets.)

---

## 6. Upstream session handling

| rule | reason |
| --- | --- |
| Persist the **whole jar** (`_otwarchive_session` + `user_credentials` + others) | session-only storage triggers `/lost_cookie` (`ao3-internals.md:§4.2`) |
| Encrypt the jar at rest with AAD `ao3/jar` | it is a bearer token |
| Keep the CSRF token in memory only, refetch after login | it is session-bound and rotates |
| One account per jar; never share a jar between accounts | identity confusion |
| Wipe the jar on logout, on `SessionIncomplete`, and on user request | minimize blast radius |
| Never accept a jar from another user's browser session | that is account takeover |
| Never log or return the jar | bearer token |

Credential login (if enabled) must:

- throttle locally to ≤ 1 attempt / 20 s and stop after 3 consecutive failures for 15 min
  (the upstream limit is 5 / 20 s; do not approach it);
- treat `pwned?` forced-password-change redirects as `PasswordChangeRequired`, not success;
- never store the password if the user chooses cookie import.

---

## 7. CSRF — both directions

**Upstream (our client → AO3):** every mutating request carries `authenticity_token`
from `/token_dispenser.json`, fetched after login. No token, no write. A token refresh is
not a retry — the action must be re-confirmed by the user if it is public.

**Downstream (browser → our server):** because our write endpoint spends the user's AO3
identity, our own CSRF protection is not optional:

- session cookie `HttpOnly; Secure; SameSite=Lax`;
- a CSRF token on every mutating route, checked server-side;
- `Origin`/`Sec-Fetch-Site` validation on writes;
- no state-changing `GET` — ever;
- session id regenerated on login; old session invalidated on logout.

---

## 8. Irreversibility and confirmation

Every public action (kudo, comment, bookmark-with-notes, subscription that emails the
author) requires an explicit confirmation in the UI that names the target and the effect.
No auto-post on page load, no "post on save" side effect, no queue that fires later. If a
write is ambiguous, the UI says so and offers a *read* ("check whether it landed"), never a
silent retry.

This mirrors AO3's own behaviour: even mute/block interpose a `confirm_*` GET before the
POST (`otwarchive/app/controllers/muted/users_controller.rb:75-79`).

---

## 9. Rate and abuse

- Global governor: one request in flight, ≥ 2 s spacing, jitter, `Retry-After` honoured.
- Per-action budgets; comment budget ≤ 3 / 15 min enforced locally *before* upstream.
- No batch endpoints. No loops over targets. A feature that would post N items needs an
  ADR and the owner's explicit approval.
- A kill switch `AO3I_WRITE_ENABLED=false` (default false until W6) that makes every
  write route refuse without touching the network; the session-level control is
  `AccountSession::kill()` → `Error::Killed` (implemented in W1).
- If AO3 ever asks us to stop, the switch and the audit log are how we comply and prove
  it.

---

## 10. Data lifecycle

- Delete = allowlisted targets under `data_root/`, pools closed first (S7/S8).
- Blobs are content-addressed and refcounted; deleting a blob requires refcount 0.
- Audit rows are append-only; redaction applies at write time, not read time.
- Backups: document that a DB backup contains encrypted secrets and the key file does
  not, so a backup alone cannot impersonate a user. If both are backed up together, the
  backup is as sensitive as the account.

---

## 11. Dependency and build standards

- Nix-first; the flake dev shell is the single source of truth (as in MUSIKA's CI).
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, test matrix on default
  and `--no-default-features`.
- `cargo deny`/`cargo audit` in CI; new dependencies need a one-line justification
  (the `dependency-logic` rule: use the system tool, else the crate, else write it).
- No `unsafe` without a `SAFETY:` comment and a test (S9).
- Reproducible builds: lockfile committed; no floating git dependencies.

---

## 12. Threat model

| id | threat | vector | control |
| --- | --- | --- | --- |
| T1 | XSS via work content | author HTML/CSS | strict `ammonia` HTML sanitization (tag/attribute allowlist, `http`/`https` schemes only, forced `noopener noreferrer`, stripping scripts/styles/event handlers) rendered inline inside `#chapters` (matching AO3's `sanitize_field` model) so built-in and public site skins style prose cleanly; standalone `/read/{id}/chapter/{idx}` retains `CHAPTER_CSP` |
| T2 | CSRF against us | malicious page posts to our routes | CSRF token + Origin check + SameSite; confirmation for public actions |
| T3 | Session theft | cookie exfiltration | HttpOnly/Secure; short local session; AO3 jar encrypted and never exposed |
| T4 | Secret leakage | logs, errors, JSON, traces | `Secret<T>` redaction; typed errors; serialization test |
| T5 | SSRF | user-supplied URL fetched by the server | only AO3 host + fixed path shapes; no arbitrary URL fetch |
| T6 | Replay / duplicate writes | retries, double clicks, reload | idempotency classes; intent log; server-unique mapping; no auto-retry for comments |
| T7 | Abuse complaint / ban | bulk or repetitive posting | §9 rates, no batch, confirmation, kill switch, audit log |
| T8 | Upstream drift | AO3 changes forms/routes | fixtures + route gate + `UpstreamChanged` outcome; disable rather than guess |
| T9 | Supply chain | malicious crate | `cargo deny`, lockfile, dependency rule |
| T10 | Local file exposure | other users on the host | `0600` key file, owner-only data dir (created `0700`; an existing one must be owner-owned and not group/world writable), owner checks (S5) |

---

## 13. PR checklist

- [ ] No new secret can reach a log, error string, or JSON body.
- [ ] New secret at rest uses §3.2 with a purpose AAD.
- [ ] New write path has a confirmation step and a `Killed`/`WriteDisabled` check.
- [ ] New upstream call goes through the gateway and the governor.
- [ ] New parse site has a fixture and an outcome classifier.
- [ ] No `unwrap` on a request path; no `allow` without a reason.
- [ ] Tests cover the failure modes in `ao3-write-api.md` §6, not just the happy path.
