# AO3Instance Shared Helpers & Module Map

Check these shared modules before writing new parsing, rendering, server, store, or Nix code.

## Core Constants (`src/consts.rs`)

- Upstream paths (`LOGIN_PATH`, `LOGOUT_PATH`, `TOKEN_PATH`, etc.) and timeouts.
- Display skin allowlist and validators: `BUILTIN_SKINS`, `is_builtin_skin`, `is_stored_skin`.

## AO3 HTML & URL Parsing (`src/ao3/`)

- `src/ao3/html.rs`: Shared `scraper` and text utilities:
  - `collapse_whitespace(&str) -> String`
  - `collapse_text(&ElementRef<'_>) -> String`
  - `parse_number(&str) -> Option<u64>` (comma/space-delimited AO3 counts)
  - `extract_digits_after<'a>(&'a str, marker: &str) -> Option<&'a str>` (numeric IDs after `/works/`, `/series/`, `/subscriptions/`, `/pseuds/`, etc.)
  - `parse_selector(&str) -> Result<Selector>` (typed `Error::UpstreamChanged`)
- `src/ao3/parse.rs`: `percent_encode` (`SpaceMode::Plus` / `SpaceMode::Percent20`), `percent_decode`, `csrf_token_from_meta`, `csrf_token_from_html`, `logged_in_account_from_html`, `flash_message`.
- `src/ao3/search/`: Split into `mod.rs` (types), `query.rs` (`ao3_tag_param`, `LiveSearchQuery`, `encode_query_pairs`), `parse.rs` (`parse_search_results`, `parse_work_element`, `parse_tag_facets`, `parse_total_from_heading`).
- `src/ao3/account_lists/`: Split into `mod.rs` (`parse_max_page`, `trailing_paren_number`), `subscriptions.rs`, `bookmarks.rs`, `readings.rs`, `directory.rs` (tags, media, fandoms).
- `src/ao3/session/`: Split into `mod.rs` (state machine & lifecycle), `login.rs` (login flow & redirect classification), `csrf.rs` (CSRF token acquisition & write-with-retry), `solver.rs` (FlareSolverr transport).

## Server-Side HTML Rendering (`src/render/`)

- `src/render/mod.rs`: `escape`, `sanitize`, `tag_href`, `overall_percent`, embedded static asset constants and content hashes.
- `src/render/components.rs`: Reusable HTML fragments:
  - `cache_banner(cache_meta, refresh_href, live_label)` (30-min cache notice)
  - `flash_notice(Option<&str>)`, `flash_error(Option<&str>)`
  - `author_links_local(&[String])`, `author_links_live(&[String])`
  - `fandom_links(&[String])`, `tag_list_items(&[String], class)`
  - `summary_block(Option<&str>)`, `stat_row(class, label, value)`
  - `select_options(&[(&str, &str)], current)`
- `src/render/blurb.rs`: Unified work cards (`work_blurb`, `live_blurb`, `reading_blurb`).
- `src/render/search/`: `mod.rs` (form controls & shared AO3 language/completion/crossover option tables), `local.rs`, `live.rs`, `tag_page.rs`.
- `src/render/work/`: `mod.rs` (`ReadContext`, `read_page`), `nav.rs`, `meta.rs`, `chapter.rs`.

## HTTP Server Handlers (`src/server/`)

- `src/server/writes/`: `mod.rs` (shared write preconditions, `record_write_failure`, `ensure_pseud`, `redirect_after_write`, `invalidate_user_list`, `notice_text`, `audit`), `preview.rs`, `actions.rs`.
- `src/server/reads/`: `mod.rs` (home, open, work reader), `search.rs` (local & live search), `progress.rs` (manual & beacon progress).
- `src/server/lists/`: `mod.rs` (query helpers, about, history), `browse.rs`, `directory.rs`, `account.rs`.
- `src/server/session/`: `mod.rs` (session routes, persistence, cache invalidation, skin toggle), `multipart.rs` (RFC 7578 upload parser).
- `src/server/cache.rs`: 30-minute SQLite page cache + stale-on-error fallback.

## SQLite Store (`src/store/`)

- `src/store/mod.rs`: Shared types and `authors_from_row`.
- `src/store/works.rs`: Shared `WORKS_SELECT_COLUMNS`, `stored_work_from_row`, `work_summary_from_row`.

## Nix (`flake.nix`)

- Shared `nativeDeps`, `clangDeps`, `libclangPath`, and `evalModuleConfig` helper in `let`, reused across `packages`, `checks`, and `devShells`.
