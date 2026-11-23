this project is designed to run as a webapplication that can be acessed thought login;, providing a frontend for AO3 with additional capabilities, such as local caching for fanfics (in server) and saving scroll position of they, and other capabilities, even standing after the AO3 is down, providing a cool and elegant fanfic consumation.

Features:

 - Accounts
 - Caching
 - Local storage of fanfics that user acess
 - Full IMPL of Ao3
 - Search <--> AO3 (server)
 - Full Reader, EPUB (Main) <-- as caching
 - Optional edge-challenge fallback through a host-provided FlareSolverr
   (never a build dependency; the app works without it)
 - Structured HTTP diagnostics at `/diagnostics`
 - In Rust
 - SQLX as DB;


 Arch ref1; https://github.com/otwcode/otwarchive
 Arch ref2; https://github.com/Kareadita/Kavita <-- ePUB reading desing & features;

 This project shall utilizate flake.nix && Kunobi Kache (https://ninja.kunobi.com/kache) for Local builds. The project shall also use strict Clippy rules.

## What you get

- A local cache that keeps reads working when AO3 is down.
- AO3-shaped start page: Find your favorites, News from the cache, Continue
  Reading, and the personal shortcuts row.
- AO3-style pages: header/nav/footer, work blurbs, required-tag badges,
  browse filters, fandoms, dashboard, subscriptions, bookmarks, reading history.
- Work Search modelled on AO3's form: submitting it performs **one literal AO3
  search** via the user's session and renders AO3's result entries (summary,
  Found count, required tags, stats, date, Edit Your Search, numbered pages).
  Results are not cached until you open a work. A blocked direct search may use
  one local-browser solve of that exact result page; `?local=1` keeps an offline
  cache-only search. Signed-out results match AO3's signed-out set; import your
  session on `/session` for account-specific results. See
  [docs/search-parity.md](docs/search-parity.md).
- Live tag pages: every tag chip opens `/tags/<tag>/works`, fetched from AO3
  with a cached fallback, including tags on uncached search results. The tag
  page carries AO3's Sort and Filter sidebar (sort, Include/Exclude tag groups,
  other tags, completion/word-count/language/search-within).
- One-click work opening: a title in live results goes through `/open/<id>`,
  caches on first use and lands in the reader.
- Automatic reading progress: the reader tracks scroll percentage inside the
  current chapter, saves it locally on scroll/page-hide, shows an overall
  percentage, and restores the chapter plus scroll position on return.
- Tag Search over cached tags, AO3-shaped tag work pages with Works/Bookmarks,
  range headings and numbered pagination, and tag include/exclude fields in the
  live search form.
- AO3 header dropdowns (Fandoms, Browse, Search, About) and a header search box
  with AO3's tip line; category links open AO3 media pages because the local
  cache does not store media classifications.
- Live AO3 home modules loaded from the signed-in AO3 home page: favorite tags,
  news, read-later works and unread inbox messages, with a short local cache and
  an explicit refresh link. Read-later titles cache/open works through the app;
  inbox items are read-only.
- Local-only **History** (`/history`): latest reads/caches with a range
  dropdown (`Today` / `Last 7 days` / `Everything`).
- A reader with chapter jump, **Entire Work** mode and automatic progress/resume.
- Appearance settings: **Auto** (system light/dark), **Classic**, **Reversi**,
  **Snow**, **Frost**, **Cloud**, **Sepia** and **Midnight**, persisted locally;
  choose them under `/session#appearance` or from the footer.
- Public AO3 site-skin import: `/themes` fetches AO3's public skin catalog, and
  importing one stores its CSS locally as a custom theme; built-in themes clear
  the custom CSS again.
- Structured HTTP diagnostics at `/diagnostics` (All / Upstream / Inbound).
- Automatic use of a **host-provided** FlareSolverr whenever it is reachable
  (falling back to direct HTTP when absent); the project never builds, vendors
  or starts it.

## Running it

```
nix develop -c cargo run --bin ao3i-server
# then open http://127.0.0.1:8099/
```

Paste an AO3 work id (or a `/works/<id>` URL) on the start page: the work is fetched
once, parsed and cached in SQLite, and served by a local reader. Re-opening a cached work
never touches archiveofourown.org, so it keeps working offline.

Configuration (environment, read once at startup): `AO3I_BIND`, `AO3I_PORT`,
`AO3I_DATA_DIR`, `AO3I_DB_FILE`, `AO3I_BASE_URL` (sandbox), `AO3I_USER_AGENT`,
`AO3I_WRITE_ENABLED`, `AO3I_MIN_SPACING_MS`, `AO3I_LOG_HTTP_ERRORS`,
`AO3I_HTTP_LOG_RETENTION`, and the optional `AO3I_FLARESOLVERR*` knobs. See
`docs/operator.md`.

### NixOS module

```nix
{
  inputs.ao3instance.url = "path:/path/to/AO3Instance";

  outputs = { nixpkgs, ao3instance, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ao3instance.nixosModules.default
        {
          services.ao3instance = {
            enable = true;
            dataDir = "/var/lib/ao3instance";
            port = 8099;
          };
        }
      ];
    };
  };
}
```

The module's settings are `dataDir` (where `ao3i.db` and `key` are written),
`port`, `openFirewall` (default `false`), and `flaresolverrPort` (default `null`,
which lets AO3Instance use its built-in `http://127.0.0.1:8191` default; when
set, passes `AO3I_FLARESOLVERR_URL=http://127.0.0.1:<port>`). With `openFirewall = true` the
configured port is opened and the service binds to all interfaces; otherwise it
is loopback-only. It creates a dedicated `ao3instance` system user. The app
holds an AO3 session, so expose it only on trusted networks or behind a reverse
proxy.

Operator guide: [docs/operator.md](docs/operator.md). Failure behaviour:
[docs/failure-matrix.md](docs/failure-matrix.md).

Current state: M1 vertical slice (open → cache → read) plus AO3 session management
(cookie import or password login, encrypted jar), chapter-level reading progress/resume,
local full-text search over the cache, conditional refresh from AO3, a write path
(subscriptions, kudos, comments, bookmarks and AO3 reading flags) behind an explicit
confirmation step, an intent/audit ledger and a kill switch
(`AO3I_WRITE_ENABLED=true`), redacted HTTP diagnostics at `/diagnostics`, and an
optional challenge fallback that calls a host-provided FlareSolverr on an explicit
click only. Tests: `nix develop -c cargo test`.
Design and decisions: `docs/init-audit.md`, `docs/ao3-write-api.md`, `docs/adr/`.
