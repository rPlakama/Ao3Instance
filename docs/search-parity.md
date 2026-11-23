# Search parity: `re zero`

Verified 2026-11-23 against live AO3.

## Signed-out baseline

AO3 in a fresh incognito window, `?work_search[query]=re zero`:

- `You searched for: re zero sort by: best match descending`
- **1,364 Found**
- 20 entries on page 1
- first five ids: `7737400`, `94570981`, `82147376`, `84878886`, `66256831`

Our app, signed out, after the solver acquired a clearance:

- **1,364 Found**
- 20 entries; the same first five ids and the same entry data
- default best-match tie order beyond the stable prefix can differ between AO3
  requests; AO3 itself reordered later entries between solver requests
- with `sort=date`, both AO3 and our app return these 20 ids in the same order:

```
70057626 62997667 96459531 71767896 65789221 12291771 95989731 90371901
73613981 96409451 85538911 66622255 91645666 86174511 34386355 80166586
95122321 86221801 72564981 85092231
```

This is the parity check used in acceptance: same summary, same total, same
entry fields, and exact order whenever AO3's sort is deterministic.

## Signed-in baseline

AO3 signed in as the operator shows **1,475 Found** for the same query. The app
is signed out, so it shows AO3's signed-out set (1,364) and says so on the result
page. To compare signed-in results, import `_otwarchive_session` and
`user_credentials` on `/session#cookie-import`, then use the solver with
`send_cookies` (or allow `AO3I_FLARESOLVERR_SEND_COOKIES=true`). The search page
will then use the same account context as the browser.

## How the request is made

- The form is local, then exactly one AO3 `/works/search` GET is issued.
- Default best match omits `sort_column`/`sort_direction` and `commit`, matching
  AO3's canonical URL shape. Sending `_score` explicitly changed AO3's tie order.
- If the direct request is blocked or times out, the search handler may solve
  that exact search page once through the local FlareSolverr (same cooldown and
  hourly budget as any other solve) and render the returned page. Subsequent
  requests use the resulting clearance directly.
