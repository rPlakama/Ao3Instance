//! Server-rendered HTML: pages, escaping and sanitization.
//!
//! Author content is untrusted. It is sanitized with `ammonia` (tag/attribute
//! allowlist, http/https URLs, forced `noopener noreferrer`) and served inside a
//! sandboxed iframe with its own CSP — never injected into the app document
//! (`docs/security-standards.md` §8, T1).

use std::collections::HashSet;

use crate::store::{StoredChapter, StoredWork, WorkSummary};

/// Minimal HTML text escaping for values interpolated into our own templates.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Sanitizes AO3-authored HTML for the reader iframe.
pub fn sanitize(html: &str) -> String {
    let schemes: HashSet<&'static str> = HashSet::from(["http", "https"]);
    ammonia::Builder::default()
        .link_rel(Some("noopener noreferrer"))
        .url_schemes(schemes)
        .clean(html)
        .to_string()
}

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

fn layout(title: &str, body: &str) -> String {
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

pub fn home_page(works: &[WorkSummary], flash: Option<&str>) -> String {
    let mut body = String::from(
        "<form class=\"open\" method=\"get\" action=\"/search\">\
         <input name=\"q\" placeholder=\"Search cached works (title, author, tag)\">\
         <button type=\"submit\">Search</button></form>\
         <form class=\"open\" method=\"post\" action=\"/open\">\
         <input name=\"id\" placeholder=\"AO3 work id or URL (e.g. 12345678)\" autofocus required>\
         <button type=\"submit\">Open work</button></form>",
    );
    if let Some(flash) = flash {
        body.push_str(&format!("<div class=\"notice\">{}</div>", escape(flash)));
    }
    body.push_str("<div class=\"meta\">cached works stay readable offline</div>");
    if works.is_empty() {
        body.push_str(
            "<p class=\"muted\">Nothing cached yet. Paste a work id or URL above; \
             the work page is fetched once and kept for offline reading.</p>",
        );
    } else {
        body.push_str("<h2>Cached works</h2><ul class=\"works\">");
        for work in works {
            let authors = if work.authors.is_empty() {
                "Anonymous".to_string()
            } else {
                work.authors.join(", ")
            };
            let continue_at = match work.progress_chapter {
                Some(chapter) => format!(
                    " · <a href=\"/read/{}?chapter={chapter}\">continue ch. {chapter}</a>",
                    escape(&work.ao3_id)
                ),
                None => String::new(),
            };
            body.push_str(&format!(
                "<li><a href=\"/read/{id}\">{title}</a>\
                 <div class=\"meta\">{authors} · {chapters} chapter(s){continue_at}</div></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                authors = escape(&authors),
                chapters = work.chapter_count,
                continue_at = continue_at,
            ));
        }
        body.push_str("</ul>");
    }
    layout("AO3Instance", &body)
}

/// Search results over the local cache.
pub fn search_page(query: &str, works: &[WorkSummary]) -> String {
    let mut body = format!(
        "<h2>Search</h2><form class=\"open\" method=\"get\" action=\"/search\">\
         <input name=\"q\" value=\"{}\" placeholder=\"title, author or tag\">\
         <button type=\"submit\">Search</button></form>",
        escape(query)
    );
    if query.trim().is_empty() {
        body.push_str("<p class=\"muted\">Search the works cached on this instance.</p>");
    } else if works.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\">Nothing cached matches “{}”. Open the work from the start \
             page first; offline search only covers what has been cached.</p>",
            escape(query)
        ));
    } else {
        body.push_str(&format!(
            "<p class=\"meta\">{} match(es)</p><ul class=\"works\">",
            works.len()
        ));
        for work in works {
            let authors = if work.authors.is_empty() {
                "Anonymous".to_string()
            } else {
                work.authors.join(", ")
            };
            let continue_at = match work.progress_chapter {
                Some(chapter) => format!(
                    " · <a href=\"/read/{}?chapter={chapter}\">continue ch. {chapter}</a>",
                    escape(&work.ao3_id)
                ),
                None => String::new(),
            };
            body.push_str(&format!(
                "<li><a href=\"/read/{id}\">{title}</a>\
                 <div class=\"meta\">{authors} · {chapters} chapter(s){continue_at}</div></li>",
                id = escape(&work.ao3_id),
                title = escape(&work.title),
                authors = escape(&authors),
                chapters = work.chapter_count,
                continue_at = continue_at,
            ));
        }
        body.push_str("</ul>");
    }
    layout("Search — AO3Instance", &body)
}

fn tag_chips(tags: &[String], class: &str) -> String {
    let mut out = String::new();
    for tag in tags {
        out.push_str(&format!(
            "<span class=\"tag {class}\">{}</span>",
            escape(tag)
        ));
    }
    out
}

/// Everything the reader needs that is not the work itself.
pub struct ReadContext<'a> {
    pub current: i64,
    pub progress: Option<i64>,
    pub notice: Option<&'a str>,
    pub subscribed: bool,
    pub can_write: bool,
    pub kudos_sent: bool,
    pub reading_state: Option<&'a str>,
}

pub fn read_page(work: &StoredWork, chapters: &[StoredChapter], ctx: &ReadContext<'_>) -> String {
    let ReadContext {
        current,
        progress,
        notice,
        subscribed,
        can_write,
        kudos_sent,
        reading_state,
    } = *ctx;
    let authors = if work.authors.is_empty() {
        "Anonymous".to_string()
    } else {
        work.authors.join(", ")
    };
    let mut body = String::new();
    if let Some(notice) = notice {
        body.push_str(&format!("<div class=\"notice\">{}</div>", escape(notice)));
    }
    body.push_str(&format!(
        "<h2 class=\"title\">{}</h2><div class=\"byline\">by {}</div>",
        escape(&work.title),
        escape(&authors)
    ));

    let mut facts: Vec<String> = Vec::new();
    if let Some(rating) = &work.rating {
        facts.push(escape(rating));
    }
    if let Some(language) = &work.language {
        facts.push(escape(language));
    }
    if let Some(words) = work.word_count {
        facts.push(format!("{words} words"));
    }
    if let Some(kudos) = work.kudos {
        facts.push(format!("{kudos} kudos"));
    }
    facts.push(checked_label(work.fetched_at));
    body.push_str(&format!("<div class=\"meta\">{}</div>", facts.join(" · ")));

    let mut tags = String::new();
    tags.push_str(&tag_chips(&work.tags.warnings, "warning"));
    tags.push_str(&tag_chips(&work.tags.fandoms, ""));
    tags.push_str(&tag_chips(&work.tags.relationships, ""));
    tags.push_str(&tag_chips(&work.tags.characters, ""));
    tags.push_str(&tag_chips(&work.tags.additional, ""));
    tags.push_str(&tag_chips(&work.tags.categories, ""));
    if !tags.is_empty() {
        body.push_str(&format!("<div class=\"tags\">{tags}</div>"));
    }

    if let Some(summary) = &work.summary
        && !summary.trim().is_empty()
    {
        body.push_str(&format!(
            "<details><summary class=\"muted\">Summary</summary><div>{}</div></details>",
            sanitize(summary)
        ));
    }

    let chapter = chapters.iter().find(|chapter| chapter.idx == current);
    let total = chapters.len();
    let position = chapters
        .iter()
        .position(|chapter| chapter.idx == current)
        .unwrap_or(0);

    if let Some(continue_at) = progress
        && continue_at != current
    {
        body.push_str(&format!(
            "<div class=\"continue\">You were at chapter {continue_at}. \
             <a href=\"/read/{}?chapter={continue_at}\">Resume</a></div>",
            escape(&work.ao3_id)
        ));
    }

    let mut toolbar = String::from("<div class=\"toolbar\">");
    toolbar.push_str(&format!(
        "<span>Chapter {} of {}</span>",
        if chapter.is_some() { position + 1 } else { 0 },
        total
    ));
    if position > 0 {
        let previous = chapters[position - 1].idx;
        toolbar.push_str(&format!(
            "<a href=\"/read/{}?chapter={previous}\">← previous</a>",
            escape(&work.ao3_id)
        ));
    }
    if position + 1 < total {
        let next = chapters[position + 1].idx;
        toolbar.push_str(&format!(
            "<a href=\"/read/{}?chapter={next}\">next →</a>",
            escape(&work.ao3_id)
        ));
    }
    toolbar.push_str(&format!(
        "<a href=\"https://archiveofourown.org/works/{}\" rel=\"noopener noreferrer\">\
         open on AO3</a>",
        escape(&work.ao3_id)
    ));
    if can_write {
        if subscribed {
            toolbar.push_str(&format!(
                "<span class=\"state active\">subscribed</span>\
                 <form class=\"inline\" method=\"post\" action=\"/write/unsubscribe\">\
                 <input type=\"hidden\" name=\"kind\" value=\"Work\">\
                 <input type=\"hidden\" name=\"target_id\" value=\"{}\">\
                 <button type=\"submit\">Unsubscribe</button></form>",
                escape(&work.ao3_id)
            ));
        } else {
            toolbar.push_str(&format!(
                "<a href=\"/write/preview?kind=work&id={}\">Subscribe to work</a>",
                escape(&work.ao3_id)
            ));
        }
    } else {
        toolbar.push_str("<a href=\"/session\">Sign in to write</a>");
    }
    if can_write {
        if kudos_sent {
            toolbar.push_str("<span class=\"state active\">kudos given</span>");
        } else {
            toolbar.push_str(&format!(
                "<a href=\"/write/preview?kind=kudos&id={}\">Give kudos</a>",
                escape(&work.ao3_id)
            ));
        }
        toolbar.push_str(&format!(
            "<form class=\"inline\" method=\"post\" action=\"/write/reading\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{}\">\
             <input type=\"hidden\" name=\"action\" value=\"later\">\
             <button type=\"submit\">Mark for later</button></form>\
             <form class=\"inline\" method=\"post\" action=\"/write/reading\">\
             <input type=\"hidden\" name=\"target_id\" value=\"{}\">\
             <input type=\"hidden\" name=\"action\" value=\"read\">\
             <button type=\"submit\">Mark as read</button></form>",
            escape(&work.ao3_id),
            escape(&work.ao3_id)
        ));
        if let Some(state) = reading_state {
            let label = if state == "read" {
                "marked read"
            } else {
                "marked for later"
            };
            toolbar.push_str(&format!("<span class=\"state active\">{label}</span>"));
        }
    }
    toolbar.push_str("</div>");
    body.push_str(&toolbar);
    body.push_str(&format!(
        "<form class=\"inline\" method=\"post\" action=\"/open\">\
         <input type=\"hidden\" name=\"id\" value=\"{}\">\
         <input type=\"hidden\" name=\"refresh\" value=\"1\">\
         <button type=\"submit\">Refresh from AO3</button></form>",
        escape(&work.ao3_id)
    ));
    body.push_str(&format!(
        "<form class=\"save\" method=\"post\" action=\"/progress\">\
         <input type=\"hidden\" name=\"work_id\" value=\"{}\">\
         <input type=\"hidden\" name=\"chapter\" value=\"{}\">\
         <button type=\"submit\">Save this chapter as my place</button></form>",
        escape(&work.ao3_id),
        current
    ));

    match chapter {
        Some(chapter) => {
            body.push_str(&format!(
                "<iframe class=\"chapter\" sandbox referrerpolicy=\"no-referrer\" \
                 title=\"Chapter {} of {}\" src=\"/read/{}/chapter/{}?v={}\"></iframe>",
                position + 1,
                total,
                escape(&work.ao3_id),
                chapter.idx,
                work.fetched_at
            ));
        }
        None => body.push_str(
            "<p class=\"muted\">No chapter content was stored for this work. \
             Re-open it to refresh (AO3's chapter markup may have changed).</p>",
        ),
    }

    layout(&format!("{} — AO3Instance", work.title), &body)
}

/// Standalone document for the chapter iframe. Served with its own CSP.
pub fn chapter_document(work_title: &str, chapter: &StoredChapter) -> String {
    let mut body = String::new();
    if let Some(title) = &chapter.title
        && !title.trim().is_empty()
    {
        body.push_str(&format!(
            "<h3 class=\"chapter-title\">{}</h3>",
            escape(title)
        ));
    }
    body.push_str(&sanitize(&chapter.body_html));
    if let Some(notes) = &chapter.notes_html
        && !notes.trim().is_empty()
    {
        body.push_str(&format!(
            "<hr class=\"notes-sep\"><div class=\"notes\"><strong>Chapter notes</strong>{}</div>",
            sanitize(notes)
        ));
    }
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{} — chapter</title><style>\
         body{{margin:0;padding:0 1.25rem;color:inherit;background:transparent;\
         font:16px/1.7 Georgia,serif;}} \
         a{{color:#8c2f39;}} img{{max-width:100%;height:auto;}} \
         .chapter-title{{font-size:1.1rem;font-weight:600;margin:.2rem 0 1rem;}} \
         .notes-sep{{border:0;border-top:1px solid #ccc4;margin:1.5rem 0 1rem;}} \
         .notes{{font-size:.92rem;color:#8a857e;}} \
         blockquote{{border-left:3px solid #ccc4;margin:1rem 0;padding-left:1rem;color:#8a857e;}}</style>\
         </head><body>{body}</body></html>",
        escape(work_title)
    )
}

/// Session management: AO3 login, cookie import, verification, adult opt-in.
#[allow(
    clippy::too_many_arguments,
    reason = "server-rendered form with independent state fields"
)]
pub fn session_page(
    account: &str,
    state: &str,
    jar_complete: bool,
    adult_enabled: bool,
    notice: Option<&str>,
    error: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h2>AO3 session</h2><p class=\"meta\">State: <span class=\"state {class}\">{state}</span> \
         {account}</p>",
        class = if state == "active" {
            "active"
        } else {
            "imported"
        },
        account = if account.is_empty() {
            String::new()
        } else {
            format!("· signed in as {}", crate::render::escape(account))
        },
    ));
    if !account.is_empty() {
        body.push_str(&format!(
            "<p class=\"meta\">Cookie jar: {}. Adult content: {}.</p>",
            if jar_complete {
                "complete"
            } else {
                "incomplete (needs user_credentials)"
            },
            if adult_enabled {
                "enabled"
            } else {
                "not enabled"
            },
        ));
        body.push_str(
            "<form class=\"inline\" method=\"post\" action=\"/session/probe\">\
             <button type=\"submit\">Verify session now</button></form>\
             <form class=\"inline\" method=\"post\" action=\"/session/adult\">\
             <button type=\"submit\">Enable adult content</button></form>\
             <form class=\"inline\" method=\"post\" action=\"/session/logout\">\
             <button type=\"submit\">Log out of AO3</button></form>",
        );
    }
    if let Some(notice) = notice {
        body.push_str(&format!("<div class=\"notice\">{}</div>", escape(notice)));
    }
    if let Some(error) = error {
        body.push_str(&format!("<div class=\"error\">{}</div>", escape(error)));
    }

    body.push_str(
        "<h3>Import cookies (recommended: no password custody)</h3>\
         <p class=\"meta\">In your browser, sign in to archiveofourown.org, open DevTools → \
         Application → Cookies, and copy the values of <code>_otwarchive_session</code> and \
         <code>user_credentials</code>. They are stored encrypted (AES-256-GCM) with a key \
         only this server can read.</p>\
         <form class=\"stack\" method=\"post\" action=\"/session/import\">\
         <label>AO3 login <input type=\"text\" name=\"account\" required autocomplete=\"username\"></label>\
         <label>_otwarchive_session <input type=\"text\" name=\"session_cookie\" required></label>\
         <label>user_credentials <input type=\"text\" name=\"credentials_cookie\" required></label>\
         <label><input type=\"checkbox\" name=\"adult\" value=\"1\"> This account may read adult content</label>\
         <button type=\"submit\">Import session</button></form>",
    );
    body.push_str(
        "<h3>Sign in with a password</h3>\
         <p class=\"meta\">Optional. The password is encrypted at rest and used only to \
         renew the AO3 session; import is safer.</p>\
         <form class=\"stack\" method=\"post\" action=\"/session/login\">\
         <label>AO3 login or email <input type=\"text\" name=\"account\" required autocomplete=\"username\"></label>\
         <label>Password <input type=\"password\" name=\"password\" required autocomplete=\"current-password\"></label>\
         <label><input type=\"checkbox\" name=\"adult\" value=\"1\"> This account may read adult content</label>\
         <button type=\"submit\">Sign in</button></form>",
    );
    layout("Session — AO3Instance", &body)
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
fn format_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3_600,
        (seconds % 3_600) / 60
    )
}

fn checked_label(fetched_at: i64) -> String {
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

/// Two-step confirmation page for a write action.
pub fn confirm_page(
    title: &str,
    description: &str,
    action: &str,
    hidden: &[(&str, &str)],
) -> String {
    let mut body = format!(
        "<h2>{}</h2><p>{}</p><form class=\"stack\" method=\"post\" action=\"{}\"\
         <p class=\"meta\">This sends one request to archiveofourown.org and is recorded \
         in the audit log.</p>",
        escape(title),
        escape(description),
        escape(action)
    );
    for (name, value) in hidden {
        body.push_str(&format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            escape(name),
            escape(value)
        ));
    }
    body.push_str("<button type=\"submit\">Confirm</button></form><p><a href=\"/\">Cancel</a></p>");
    layout("Confirm — AO3Instance", &body)
}

/// Recent write actions, newest first. No secrets: the ledger stores targets and
/// outcomes, never credentials or payloads.
pub fn audit_page(entries: &[crate::store::AuditEntry]) -> String {
    let mut body = String::from("<h2>Audit log</h2>");
    if entries.is_empty() {
        body.push_str("<p class=\"muted\">No writes recorded yet.</p>");
    } else {
        body.push_str(
            "<table class=\"audit\"><thead><tr><th>when</th><th>action</th>\
             <th>target</th><th>outcome</th></tr></thead><tbody>",
        );
        for entry in entries {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&format_utc(entry.created_at)),
                escape(&entry.action),
                escape(&entry.target),
                escape(&entry.outcome),
            ));
        }
        body.push_str("</tbody></table>");
    }
    body.push_str("<p><a href=\"/\">Back to the start</a></p>");
    layout("Audit — AO3Instance", &body)
}

pub fn error_page(status: u16, message: &str) -> String {
    layout(
        "AO3Instance — error",
        &format!(
            "<h2>Something went wrong</h2><div class=\"error\">HTTP {status}: {}</div>\
             <p><a href=\"/\">Back to the start</a></p>",
            escape(message)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_the_dangerous_characters() {
        assert_eq!(
            escape("<script>alert('x') & \"y\"</script>"),
            "&lt;script&gt;alert(&#39;x&#39;) &amp; &quot;y&quot;&lt;/script&gt;"
        );
    }

    #[test]
    fn sanitizer_strips_scripts_handlers_and_javascript_urls() {
        let dirty = r#"<p>hi<script>alert(1)</script></p><img src=x onerror=alert(1)><a href="javascript:alert(1)">x</a>"#;
        let clean = sanitize(dirty);
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("javascript:"));
        assert!(clean.contains("<p>"));
    }

    #[test]
    fn sanitizer_keeps_formatting_and_links() {
        let clean = sanitize(r#"<p><em>ok</em> <a href="https://example.org">link</a></p>"#);
        assert!(clean.contains("<em>"));
        assert!(clean.contains("https://example.org"));
    }

    #[test]
    fn utc_formatting_matches_known_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_795_348_800), "2026-11-22 12:00 UTC");
        assert_eq!(format_utc(1_795_392_000), "2026-11-23 00:00 UTC");
    }

    #[test]
    fn chapter_document_includes_sanitized_body_and_notes() {
        let chapter = StoredChapter {
            idx: 1,
            title: Some("Chapter <One>".into()),
            body_html: "<p>body</p><script>bad()</script>".into(),
            notes_html: Some("<p>note</p>".into()),
        };
        let doc = chapter_document("w", &chapter);
        assert!(doc.contains("&lt;One&gt;"));
        assert!(doc.contains("<p>body</p>"));
        assert!(!doc.contains("bad()"));
        assert!(doc.contains("Chapter notes"));
    }
}
