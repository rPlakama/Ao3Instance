# ADR-0007 — Host-provided FlareSolverr fallback

- **Status:** Accepted (2026-11-22), amended for search (2026-11-23)
- **Date:** 2026-11-22
- **Amends:** ADR-0002 §5 (identity), and the "challenge is terminal" consequence
- **Related:** `docs/operator.md`, `docs/failure-matrix.md`, PLAN.md §2.5

## Context

AO3 is fronted by Cloudflare. A JavaScript-less client is scored and may be
challenged (`docs/ao3-internals.md` §6). ADR-0002 made that a first-class
`BlockedByEdge` outcome and deliberately refused automated bypassing. In
practice a user can still be unable to cache a page they have every right to
read, and hand-copying cookies does not help when the edge serves an
interstitial.

FlareSolverr is a proxy server that drives a real Chromium to solve one
challenge and returns the page plus cookies. It is packaged in nixpkgs and has a
systemd module (`services.flaresolverr`, port 8191, firewall closed by default).

## Decision

1. **Runtime service, never a build dependency.** No crate, no vendored Python,
   no flake input, no `buildInputs`/wrapper change. The host operator installs
   and supervises the service; the app only calls its loopback `/v1` API. A
   machine without FlareSolverr must build and run every feature (with the
   fallback absent).
2. **Explicit, per-page, opt-in.** A work solve only happens on `POST /solve`,
   from a button on the edge-block error page. Never in the background, never for
   a batch or a crawl.
   **Search exception (2026-11-23):** a live Work Search is itself one explicit
   user action. If that exact search GET is blocked or times out, the handler may
   perform one solve for that one result page, under the same single-flight lock,
   cooldown and hourly budget, and render it. It never follows pagination or
   searches on its own.
3. **Reads only.** The solver fetches `request.get` pages. Writes keep the
   direct client because a browser replay can double-post, bypasses the write
   governor and confuses the CSRF flow.
4. **Scoped identity override.** While a Cloudflare clearance is fresh, direct
   reads may use the browser User-Agent the clearance is bound to; the honest
   `AO3I_USER_AGENT` returns on expiry or on any `403`. Writes always use the
   honest UA. This is the only sanctioned exception to ADR-0002 §5.
5. **Hard limits.** One solve in flight, `5 min` cooldown, `6` successful solves
   per rolling hour, `Retry-After` respected, one AO3 request per solve. Every
   attempt is recorded in `solver_event`; failures also land in
   `http_error_log` with `transport = solver`.
6. **Secrets stay secret.** `cf_clearance`, `__cf_bm` and any session cookie are
   `SecretString`s, encrypted at rest with the existing AES-256-GCM key and
   `ao3/solver` purpose binding; they never appear in logs, the UI, the audit
   log or `Debug` output. AO3 cookies are handed to the browser only when the
   operator set `AO3I_FLARESOLVERR_SEND_COOKIES=true` and the user ticked the
   per-solve box.
7. **Honest risk.** Cloudflare exists to protect AO3 from abuse. This fallback
   is for a reader caching pages they already have access to. It must not become
   scraping; the operator can disable it entirely with
   `AO3I_FLARESOLVERR=off`.

## Consequences

- The app gains an optional dependency on a *system* service, documented in
  `docs/operator.md`; the build closure does not change.
- `403`/`503` on a work still returns a `502` error page, with a solve card when
  the service is reachable. A blocked search may be solved once automatically
  because the user already submitted that search; work solves remain one click.
- A stale clearance is dropped on the next `403`, so the client degrades to the
  honest identity instead of replaying a browser UA forever.
- The clearance window is a heuristic: Cloudflare also binds to TLS
  fingerprints, so some solved pages may still fail on the next direct read.
  That is recorded as a normal diagnostic, not retried.

## Test

- `tests/flaresolverr.rs`: happy path (403 → solve → cached reader), absent
  service (502 page, 503 action, no hang), captcha, `off` mode (zero probes),
  cooldown, encrypted clearance and no plaintext cookie in SQLite.
- `tests/client_http.rs`: clearance changes UA/cookie only inside the window and
  is dropped by a `403`.
- `scripts/check-no-solver-build-dep.sh`: `Cargo.*` and `flake.*` stay free of
  FlareSolverr, and the `nix build` closure contains no such store path.
