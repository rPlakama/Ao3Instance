# AO3Instance

AO3Instance is a self-hosted, offline-capable frontend for
[Archive of Our Own](https://archiveofourown.org). It sits between your browser
and AO3 as a personal caching layer: open a work once and it is stored locally
for offline reading, while your session, progress and library stay on your
machine.

It is a user agent, not a crawler. Every upstream request traces back to an
action you take in the UI; requests are spaced out, use an honest User-Agent,
and nothing is fetched in the background or in batches.

This is an unofficial project, not affiliated with or endorsed by the OTW.

## Features

- **Offline library** — works are fetched on demand, cached in SQLite and
  readable without a network connection; uncached works show a clear miss card
  instead of fetching silently.
- **Reader** — AO3-shaped pages at `/works/<id>` and `/read/<id>`, chapter
  jump, Entire Work mode, and per-chapter reading progress.
- **AO3 look and feel** — the familiar archive layout plus themes (Auto,
  Classic, Reversi, Snow, Frost, Cloud, Sepia, Midnight) and import of public
  AO3 site skins.
- **Discovery** — AO3 Work Search and tag search (30-minute cached, with
  offline fallback), fandom directories, popular tags, autocomplete, and a
  local full-text search over already-cached works.
- **Your AO3 account** — cookie import (recommended) or password login, with
  the session jar encrypted at rest. Restricted works and all writes require a
  session. Logout is local-only; it does not sign you out of AO3.
- **Live AO3 lists** — subscriptions, bookmarks, reading history, Marked for
  Later, favorites and dashboard modules, cached for 30 minutes with offline
  fallback.
- **Writes (opt-in, off by default)** — kudos, comments and replies,
  subscribe/unsubscribe, bookmarks with notes, and mark for later/as read.
  Each action requires an explicit confirmation, is attempted once, and is
  recorded in an append-only audit ledger.
- **Diagnostics** — `/diagnostics` shows upstream HTTP failures, the request
  governor and optional FlareSolverr state, without exposing cookies.
- **Optional FlareSolverr** — if the host already runs FlareSolverr, blocked
  reads and searches can be solved on demand (one page, hard cooldown and
  budget). It is never a build dependency and is never used for writes.

## Requirements

- **Nix with flakes enabled.** The flake is the only supported build and run
  path and pins the Rust toolchain and native dependencies.
- A writable data directory (defaults to `$XDG_CONFIG_HOME/ao3instance`).
- Outbound HTTPS to `archiveofourown.org`.
- *Optional:* a host-provided FlareSolverr service on loopback for Cloudflare
  challenges. The app builds, runs and passes tests without it.

## Build

```sh
nix build
./result/bin/ao3i-server
```

Or build and run in one step:

```sh
nix run .
```

For development, the flake provides the entire toolchain:

```sh
nix develop
cargo run --bin ao3i-server
```

## Install

### Run directly

```sh
nix run .
# AO3Instance is now at http://127.0.0.1:8099/
```

By default it binds to loopback only and writes to
`$XDG_CONFIG_HOME/ao3instance` (`ao3i.db` plus the encryption `key`).

### NixOS module

The flake exposes `nixosModules.default`. Add this repository as a flake input
and import the module, then:

```nix
services.ao3instance = {
  enable = true;
  dataDir = "/var/lib/ao3instance";
  port = 8099;
  # openFirewall = true;      # default: loopback only
  # flaresolverrPort = 8191;  # optional host-provided FlareSolverr
};
```

The module creates the data directory (mode `0700`, owned by the `ao3instance`
system user), runs `ao3i-server`, restarts on failure, and applies systemd
hardening. Back up the whole data directory (database **and** key); without the
key the session cannot be decrypted.

## Configuration

All settings are environment variables with the `AO3I_` prefix:

| variable | default | meaning |
| --- | --- | --- |
| `AO3I_BIND` | `127.0.0.1` | listen address |
| `AO3I_PORT` | `8099` | listen port |
| `AO3I_DATA_DIR` | `$XDG_CONFIG_HOME/ao3instance` | data directory |
| `AO3I_DB_FILE` | `<data_dir>/ao3i.db` | SQLite database |
| `AO3I_KEY_FILE` | `<data_dir>/key` | AES-256-GCM key (mode `0600`) |
| `AO3I_BASE_URL` | `https://archiveofourown.org` | upstream origin (sandbox/testing) |
| `AO3I_USER_AGENT` | built-in | set a project URL and contact before public use |
| `AO3I_WRITE_ENABLED` | `false` | master switch for all writes |
| `AO3I_MIN_SPACING_MS` | `2000` | floor between upstream requests |
| `AO3I_FLARESOLVERR` | `auto` | `auto`, `on` or `off` for the optional solver |
| `AO3I_FLARESOLVERR_URL` | `http://127.0.0.1:8191` | solver origin (loopback-only by default) |

See [docs/operator.md](docs/operator.md) for the complete table, including HTTP
diagnostics and FlareSolverr limits.

## First use

1. Open `http://127.0.0.1:8099/`.
2. Open a public work by id or URL: it is fetched once, cached, and readable
   offline afterwards.
3. For restricted or adult works and for writes, go to **Session**. Cookie
   import is the recommended path (copy `_otwarchive_session` and
   `user_credentials` from your browser). Password login works, but the
   password is never persisted.
4. Writes remain disabled until `AO3I_WRITE_ENABLED=true`. Every write shows a
   confirmation page and appears in **Audit**.

## Security and privacy

- The app has **no user model of its own** beyond the AO3 session. It binds to
  loopback by default; if you expose it, put it behind a TLS reverse proxy and
  add authentication.
- Secrets at rest are encrypted with AES-256-GCM using a key file created with
  mode `0600`. Passwords are never stored.
- Writes are opt-in, one action at a time, recorded before and after sending,
  and never retried blindly. The audit log stores targets and outcomes, never
  payloads, cookies or passwords.
- The local cache is for personal offline reading. Do not redistribute cached
  work text.
- Only reachable loopback FlareSolverr instances are accepted unless
  `AO3I_FLARESOLVERR_ALLOW_REMOTE=true` is set deliberately. Never expose
  FlareSolverr itself to a network.

## Development

```sh
nix develop

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

# Optional: verify AO3 route claims against a local otwarchive clone
sh scripts/check-ao3-routes.sh
```

CI runs formatting, clippy (warnings as errors), the test suite and the AO3
route drift gate. The `otwarchive/` reference clone is gitignored and never
committed.

## Documentation

- [docs/operator.md](docs/operator.md) — running, configuration, pages,
  troubleshooting, limitations
- [docs/adr/](docs/adr/) — architecture decisions (auth, access posture, write
  scope and safety, FlareSolverr)
- [docs/ao3-write-api.md](docs/ao3-write-api.md) — AO3 action contracts and
  failure taxonomy
- [docs/failure-matrix.md](docs/failure-matrix.md) and
  [docs/security-standards.md](docs/security-standards.md)

## Special thanks

- [otwcode/otwarchive](https://github.com/otwcode/otwarchive) — AO3 is open
  source, and this project studies its routes, markup and policies to stay
  faithful. I am deeply grateful for AO3 and everyone who keeps it running;
  this tool is only a way to make my own reading less dependent on constant
  connectivity.
