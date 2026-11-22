//! Shared page chrome: the stylesheet, the document skeleton and the
//! UTC/staleness helpers used by more than one segment.
const STYLE: &str = r#"
:root { color-scheme: light dark; --fg:#1c1b1a; --bg:#fbfaf7; --muted:#6b6762; --line:#e2ddd5; --accent:#8c2f39; }
@media (prefers-color-scheme: dark) { :root { --fg:#e9e4dc; --bg:#171615; --muted:#a39c93; --line:#33302c; --accent:#e07b86; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:16px/1.6 "Iowan Old Style", Georgia, serif; }
header.site { display:flex; align-items:baseline; gap:1rem; padding:.75rem 1.5rem; border-bottom:1px solid var(--line); }
header.site h1 { font-size:1.05rem; margin:0; letter-spacing:.02em; }
header.site a { color:var(--accent); text-decoration:none; }
main { max-width: 52rem; margin:0 auto; padding:1.5rem; }
form.open { display:flex; gap:.5rem; margin:1rem 0 2rem; }
form.open input { flex:1; padding:.6rem .7rem; font:inherit; color:inherit; background:transparent; border:1px solid var(--line); border-radius:6px; }
form.open button { padding:.6rem 1rem; font:inherit; border:1px solid var(--accent); background:var(--accent); color:#fff; border-radius:6px; cursor:pointer; }
ul.works { list-style:none; padding:0; }
ul.works li { padding:.7rem 0; border-bottom:1px solid var(--line); }
ul.works a { color:var(--fg); text-decoration:none; font-weight:600; }
.meta, .muted { color:var(--muted); font-size:.9rem; }
.error { border:1px solid #b3423c; color:#b3423c; padding:.7rem 1rem; border-radius:6px; margin:1rem 0; }
.tags { display:flex; flex-wrap:wrap; gap:.35rem; margin:.6rem 0; }
.tag { font-size:.78rem; padding:.15rem .5rem; border:1px solid var(--line); border-radius:999px; color:var(--muted); }
.tag.warning { border-color:#b3423c; color:#b3423c; }
h2.title { margin:.2rem 0 0; font-size:1.9rem; line-height:1.25; }
.byline { margin:.35rem 0 .8rem; color:var(--muted); }
.toolbar { display:flex; flex-wrap:wrap; gap:.5rem; align-items:center; margin:1rem 0; padding:.5rem 0; border-top:1px solid var(--line); border-bottom:1px solid var(--line); }
.toolbar a, .toolbar span { font-size:.85rem; color:var(--muted); text-decoration:none; }
.toolbar a:hover { color:var(--accent); }
iframe.chapter { width:100%; border:0; background:transparent; min-height:70vh; }
table.audit { width:100%; border-collapse:collapse; font-size:.9rem; }
pre.preview { white-space:pre-wrap; border:1px solid var(--line); border-radius:6px; padding:.8rem; background:rgba(127,127,127,.06); font:inherit; }
details.comment { margin:2rem 0 1rem; border-top:1px solid var(--line); padding-top:1rem; }
details.comment summary { cursor:pointer; color:var(--accent); }
form.stack textarea { padding:.6rem .7rem; font:inherit; color:inherit; background:transparent; border:1px solid var(--line); border-radius:6px; resize:vertical; }
table.audit th, table.audit td { text-align:left; padding:.45rem .5rem; border-bottom:1px solid var(--line); }
table.audit th { color:var(--muted); font-weight:600; }
footer.site { padding:1rem 1.5rem; border-top:1px solid var(--line); color:var(--muted); font-size:.8rem; }
form.stack { display:grid; gap:.6rem; max-width:32rem; margin:1rem 0 2rem; }
form.stack label { font-size:.85rem; color:var(--muted); }
form.stack input[type=text], form.stack input[type=password] { padding:.55rem .7rem; font:inherit; color:inherit; background:transparent; border:1px solid var(--line); border-radius:6px; }
form.stack button, form.save button { padding:.55rem 1rem; font:inherit; border:1px solid var(--accent); background:var(--accent); color:#fff; border-radius:6px; cursor:pointer; }
form.inline { display:flex; flex-wrap:wrap; gap:.5rem; align-items:center; margin:1rem 0; }
form.inline button { padding:.4rem .8rem; font:inherit; border:1px solid var(--line); background:transparent; color:inherit; border-radius:6px; cursor:pointer; }
.state { display:inline-block; padding:.1rem .5rem; border-radius:999px; font-size:.78rem; border:1px solid var(--line); }
.state.active { border-color:#2e7d32; color:#2e7d32; }
.state.imported { border-color:#b8860b; color:#b8860b; }
.notice { border:1px solid #2e7d32; color:#2e7d32; padding:.7rem 1rem; border-radius:6px; margin:1rem 0; }
.continue { margin:.6rem 0; padding:.6rem .9rem; border:1px dashed var(--line); border-radius:6px; font-size:.92rem; }
"#;

pub(super) fn layout(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><style>{STYLE}</style></head>\n<body>\
         <header class=\"site\"><h1><a href=\"/\">AO3Instance</a></h1>\
         <span class=\"muted\">local cache · reader</span>\
         <span class=\"muted\"><a href=\"/audit\">Audit</a> · <a href=\"/session\">Session</a></span></header>\
         <main>{body}</main>\
         <footer class=\"site\">Fetched from archiveofourown.org for personal offline reading. \
         Content belongs to its authors.</footer></body></html>"
    )
}

/// How long before a cached copy is labelled stale.
const STALE_AFTER_SECS: i64 = 7 * 86_400;

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM UTC` without pulling a date crate.
pub(super) fn format_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3_600,
        (seconds % 3_600) / 60
    )
}

pub(super) fn checked_label(fetched_at: i64) -> String {
    let age = now_secs().saturating_sub(fetched_at);
    if age > STALE_AFTER_SECS {
        format!(
            "checked {} ({} days ago — stale)",
            format_utc(fetched_at),
            age / 86_400
        )
    } else {
        format!("checked {}", format_utc(fetched_at))
    }
}

/// Inverse of the day-count algorithm: days since 1970-01-01 → (year, month, day).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting_matches_known_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_795_348_800), "2026-11-22 12:00 UTC");
        assert_eq!(format_utc(1_795_392_000), "2026-11-23 00:00 UTC");
    }

    #[test]
    fn checked_label_flags_only_old_copies() {
        let fresh = checked_label(now_secs());
        assert!(fresh.contains("checked "), "{fresh}");
        assert!(!fresh.contains("stale"), "{fresh}");

        let stale = checked_label(now_secs() - STALE_AFTER_SECS - 86_400);
        assert!(stale.contains("stale"), "{stale}");
        assert!(stale.contains("8 days ago"), "{stale}");
    }
}
