//! Parser for exported browser cookie files (Netscape `cookies.txt`, JSON
//! cookie arrays from Cookie-Editor / EditThisCookie, or raw `Cookie:` headers).
//!
//! Filters to `archiveofourown.org` cookies when domains are present, so a full
//! browser export can be dropped in directly without manual editing.

use crate::ao3::jar::Jar;
use crate::error::{Error, Result};

/// Parsed cookies ready to install into a session [`Jar`].
pub struct ImportedCookies {
    pub jar: Jar,
    pub cf_clearance: Option<String>,
    pub count: usize,
}

/// Parses a cookie file in Netscape `cookies.txt`, JSON, or raw `Cookie:`
/// header format into an AO3 [`Jar`].
pub fn parse_cookie_file(content: &str) -> Result<ImportedCookies> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(Error::Validation("cookie file is empty".into()));
    }

    let pairs = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        parse_json_cookies(trimmed)?
    } else if looks_like_netscape(trimmed) {
        parse_netscape_cookies(trimmed)
    } else {
        parse_header_cookies(trimmed)
    };

    if pairs.is_empty() {
        return Err(Error::Validation(
            "no archiveofourown.org cookies found in file (expected Netscape cookies.txt, JSON cookie export, or Cookie header)".into(),
        ));
    }

    let mut jar = Jar::new();
    let mut cf_clearance = None;
    let mut count = 0usize;
    for (name, value) in pairs {
        if !is_safe_cookie_token(&name) || !is_safe_cookie_value(&value) {
            continue;
        }
        if name == "cf_clearance" {
            cf_clearance = Some(value);
            count += 1;
            continue;
        }
        if name == "__cf_bm" {
            continue;
        }
        if jar.apply(&format!("{name}={value}; Path=/")) {
            count += 1;
        }
    }

    if !jar.is_complete() {
        return Err(Error::Validation(
            "cookie file is missing _otwarchive_session or user_credentials for archiveofourown.org"
                .into(),
        ));
    }

    Ok(ImportedCookies {
        jar,
        cf_clearance,
        count,
    })
}

fn looks_like_netscape(text: &str) -> bool {
    text.lines().any(|line| {
        let l = line.trim();
        l.starts_with("# Netscape")
            || l.starts_with("# HTTP Cookie File")
            || l.split('\t').count() >= 7
    })
}

fn parse_netscape_cookies(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // Browser exporters mark HttpOnly cookies with `#HttpOnly_domain\t...`.
        if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            line = rest;
        } else if line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 7 {
            continue;
        }
        let domain = cols[0].trim();
        if !is_ao3_domain(domain) {
            continue;
        }
        let name = cols[5].trim();
        let value = cols[6].trim();
        if !name.is_empty() && !value.is_empty() {
            out.push((name.to_string(), value.to_string()));
        }
    }
    out
}

fn parse_json_cookies(text: &str) -> Result<Vec<(String, String)>> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|err| Error::Validation(format!("invalid JSON cookie file: {err}")))?;
    let list = if let Some(arr) = value.as_array() {
        arr
    } else if let Some(arr) = value.get("cookies").and_then(serde_json::Value::as_array) {
        arr
    } else {
        return Err(Error::Validation(
            "JSON cookie file must be an array or an object with a \"cookies\" array".into(),
        ));
    };

    let mut out = Vec::new();
    for entry in list {
        let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(val) = entry.get("value").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if let Some(domain) = entry.get("domain").and_then(serde_json::Value::as_str)
            && !is_ao3_domain(domain)
        {
            continue;
        }
        let name = name.trim();
        let val = val.trim();
        if !name.is_empty() && !val.is_empty() {
            out.push((name.to_string(), val.to_string()));
        }
    }
    Ok(out)
}

fn parse_header_cookies(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed
            .strip_prefix("Cookie:")
            .or_else(|| trimmed.strip_prefix("cookie:"))
            .unwrap_or(trimmed);
        for part in body.split(';') {
            let Some((name, value)) = part.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if !name.is_empty() && !value.is_empty() {
                out.push((name.to_string(), value.to_string()));
            }
        }
    }
    out
}

fn is_ao3_domain(domain: &str) -> bool {
    let clean = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    clean == "archiveofourown.org"
        || clean.ends_with(".archiveofourown.org")
        || clean == "localhost"
        || clean == "127.0.0.1"
}

fn is_safe_cookie_token(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || ch == ';' || ch == '=')
}

fn is_safe_cookie_value(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || ch == ';' || ch == ',')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_netscape_cookies_txt_including_httponly_and_filters_domains() {
        let txt = "\
# Netscape HTTP Cookie File
.example.com\tTRUE\t/\tTRUE\t1999999999\t_otwarchive_session\twrong-site
#HttpOnly_.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\t_otwarchive_session\tsess-123
.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\tuser_credentials\t1
.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\tremember_user_token\trem-tok
.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\tview_adult\ttrue
.archiveofourown.org\tTRUE\t/\tTRUE\t1999999999\tcf_clearance\tcf-123
";
        let imported = parse_cookie_file(txt).expect("parse netscape");
        assert_eq!(imported.jar.get("_otwarchive_session"), Some("sess-123"));
        assert_eq!(imported.jar.get("user_credentials"), Some("1"));
        assert_eq!(imported.jar.get("remember_user_token"), Some("rem-tok"));
        assert_eq!(imported.jar.get("view_adult"), Some("true"));
        assert_eq!(imported.cf_clearance.as_deref(), Some("cf-123"));
    }

    #[test]
    fn parses_json_cookie_export_and_header_string() {
        let json = r#"[
            {"name": "_otwarchive_session", "value": "json-sess", "domain": ".archiveofourown.org"},
            {"name": "user_credentials", "value": "1", "domain": "archiveofourown.org"},
            {"name": "other", "value": "x", "domain": "other.example"}
        ]"#;
        let imported = parse_cookie_file(json).expect("parse json");
        assert_eq!(imported.jar.get("_otwarchive_session"), Some("json-sess"));
        assert_eq!(imported.jar.get("user_credentials"), Some("1"));
        assert_eq!(imported.jar.get("other"), None);

        let header = "Cookie: _otwarchive_session=hdr-sess; user_credentials=1; view_adult=1";
        let from_hdr = parse_cookie_file(header).expect("parse header");
        assert_eq!(from_hdr.jar.get("_otwarchive_session"), Some("hdr-sess"));
        assert_eq!(from_hdr.jar.get("view_adult"), Some("1"));
    }

    #[test]
    fn rejects_incomplete_or_empty_cookie_files() {
        assert!(parse_cookie_file("").is_err());
        assert!(parse_cookie_file("_otwarchive_session=only-one").is_err());
    }
}
