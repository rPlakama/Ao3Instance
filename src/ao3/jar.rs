//! Cookie jar for a single upstream (AO3).
//!
//! Deliberately simple: cookies are keyed by name and sent to the one host we
//! talk to. Domain and path matching are not modelled because there is exactly
//! one origin; that is documented rather than silently half-implemented.
//!
//! The jar is what makes a session usable: AO3 requires both the session cookie
//! and `user_credentials`, or it force-logs-out to `/lost_cookie`
//! (`docs/ao3-internals.md` §4.2).
//!
//! Redaction: a jar is a bearer token. `Debug` prints names only, cookie values
//! are never `Debug`, the plaintext serialization type is private, and values are
//! wiped on drop/clear.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::consts::{CREDENTIALS_COOKIE, SESSION_COOKIE};
use crate::crypto::{AAD_JAR, SecretKey, wipe_string};
use crate::error::{Error, Result};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Cookie {
    pub(crate) value: String,
    /// Unix seconds, when the upstream supplied a parseable expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expires_unix: Option<i64>,
}

impl Cookie {
    fn is_expired(&self, now_unix: i64) -> bool {
        self.expires_unix.is_some_and(|expiry| expiry <= now_unix)
    }
}

/// Plaintext shape used only inside [`Jar::encrypt`]. Private so a caller cannot
/// accidentally serialize a jar in the clear.
#[derive(Serialize)]
struct JarRef<'a> {
    cookies: &'a BTreeMap<String, Cookie>,
}

#[derive(Deserialize)]
struct JarData {
    cookies: BTreeMap<String, Cookie>,
}

#[derive(Default, Clone)]
pub struct Jar {
    cookies: BTreeMap<String, Cookie>,
}

impl Jar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses one `Set-Cookie` header into `(name, cookie)`.
    ///
    /// Only attributes that change behaviour are read: `Max-Age` and `Expires`
    /// (deletion/expiry) and a quoted value. `Domain`/`Path`/`Secure`/`HttpOnly`
    /// are ignored on purpose — see the module note.
    pub(crate) fn parse_set_cookie(header: &str) -> Option<(String, Cookie)> {
        let mut parts = header.split(';');
        let first = parts.next()?.trim();
        let (name, value) = first.split_once('=')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let value = unquote(value.trim());
        if value.is_empty() {
            // An empty value with Max-Age=0 is a deletion; handled by `apply`.
            return Some((
                name.to_string(),
                Cookie {
                    value: String::new(),
                    expires_unix: Some(0),
                },
            ));
        }

        let mut expires_unix = None;
        let mut delete = false;
        for attr in parts {
            let attr = attr.trim();
            let Some((attr_name, attr_value)) = attr.split_once('=') else {
                continue;
            };
            match attr_name.trim().to_ascii_lowercase().as_str() {
                "max-age" => {
                    if let Ok(secs) = attr_value.trim().parse::<i64>() {
                        if secs <= 0 {
                            delete = true;
                        }
                        // Saturating: a hostile Max-Age must not overflow.
                        expires_unix = Some(now_unix().saturating_add(secs));
                    }
                }
                "expires" => {
                    if let Some(ts) = parse_http_date(attr_value.trim()) {
                        expires_unix = Some(ts);
                    }
                }
                _ => {}
            }
        }
        if delete {
            expires_unix = Some(0);
        }
        Some((
            name.to_string(),
            Cookie {
                value,
                expires_unix,
            },
        ))
    }

    /// Applies one `Set-Cookie` header. Returns `true` when the jar changed.
    pub fn apply(&mut self, header: &str) -> bool {
        let Some((name, cookie)) = Self::parse_set_cookie(header) else {
            return false;
        };
        if cookie.is_expired(now_unix()) || cookie.value.is_empty() {
            let mut cookie = cookie;
            wipe_string(&mut cookie.value);
            return self.remove(&name);
        }
        let changed = self.cookies.get(&name) != Some(&cookie);
        // The replaced value is a secret too; wipe it instead of dropping it.
        if let Some(mut replaced) = self.cookies.insert(name, cookie) {
            wipe_string(&mut replaced.value);
        }
        changed
    }

    /// Applies every `Set-Cookie` header from a response.
    pub fn apply_all<'a, I>(&mut self, headers: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        for header in headers {
            self.apply(header);
        }
    }

    /// Value of a live cookie, if present and unexpired.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.cookies
            .get(name)
            .filter(|c| !c.is_expired(now_unix()))
            .map(|c| c.value.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// A `Cookie:` header value for the live cookies, or `None` when empty.
    pub fn cookie_header(&self) -> Option<String> {
        let now = now_unix();
        let pairs: Vec<String> = self
            .cookies
            .iter()
            .filter(|(_, cookie)| !cookie.is_expired(now) && !cookie.value.is_empty())
            .map(|(name, cookie)| format!("{name}={}", cookie.value))
            .collect();
        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }

    /// Both cookies AO3 needs for a session to survive its own checks.
    pub fn is_complete(&self) -> bool {
        self.contains(SESSION_COOKIE) && self.contains(CREDENTIALS_COOKIE)
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn clear(&mut self) {
        self.wipe();
    }

    /// Serializes and encrypts the whole jar for storage at rest.
    pub fn encrypt(&self, key: &SecretKey) -> Result<String> {
        let mut json = serde_json::to_string(&JarRef {
            cookies: &self.cookies,
        })
        .map_err(|e| Error::Crypto(format!("serialize jar: {e}")))?;
        let ciphertext = key.encrypt(AAD_JAR, &json);
        wipe_string(&mut json);
        ciphertext
    }

    /// Inverse of [`Jar::encrypt`].
    pub fn decrypt(key: &SecretKey, ciphertext: &str) -> Result<Self> {
        let mut json = key.decrypt(AAD_JAR, ciphertext)?;
        let parsed = serde_json::from_str::<JarData>(&json);
        wipe_string(&mut json);
        parsed
            .map(|data| Self {
                cookies: data.cookies,
            })
            .map_err(|e| Error::Crypto(format!("deserialize jar: {e}")))
    }

    fn remove(&mut self, name: &str) -> bool {
        let Some(mut removed) = self.cookies.remove(name) else {
            return false;
        };
        wipe_string(&mut removed.value);
        true
    }

    fn wipe(&mut self) {
        let cookies = std::mem::take(&mut self.cookies);
        for (mut name, mut cookie) in cookies {
            wipe_string(&mut cookie.value);
            wipe_string(&mut name);
        }
    }
}

impl Drop for Jar {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl fmt::Debug for Jar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Names only: a jar is a bearer token.
        f.debug_struct("Jar")
            .field("cookies", &self.cookies.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parses an RFC 1123 (IMF-fixdate) timestamp: `Sun, 22 Nov 2026 12:00:00 GMT`.
///
/// Unparseable or out-of-range dates yield `None`; the cookie then behaves as a
/// session cookie, which is the safe interpretation. All arithmetic is checked so
/// a hostile header cannot panic or wrap.
pub fn parse_http_date(text: &str) -> Option<i64> {
    let text = text.trim();
    let comma = text.find(',')?;
    let rest = text[comma + 1..].trim();
    let mut fields = rest.split_whitespace();
    let day: i64 = fields.next()?.parse().ok()?;
    let month = month_number(fields.next()?)?;
    let year: i64 = fields.next()?.parse().ok()?;
    if !(1970..=9999).contains(&year) {
        return None;
    }
    let time = fields.next()?;
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    days.checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)
}

fn month_number(name: &str) -> Option<i64> {
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    months
        .iter()
        .position(|m| m.eq_ignore_ascii_case(name))
        .map(|i| i as i64 + 1)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`), checked.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn jar_with(pairs: &[(&str, &str)]) -> Jar {
        let mut jar = Jar::new();
        for (name, value) in pairs {
            jar.apply(&format!("{name}={value}; path=/"));
        }
        jar
    }

    fn private_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ao3i-jar-{}-{tag}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_the_ao3_session_cookie_shape() {
        let (name, cookie) = Jar::parse_set_cookie(
            "_otwarchive_session=abc123--def; path=/; expires=Sun, 22 Nov 2026 12:00:00 GMT; secure; httponly; samesite=lax",
        )
        .unwrap();
        assert_eq!(name, "_otwarchive_session");
        assert_eq!(cookie.value, "abc123--def");
        assert_eq!(cookie.expires_unix, Some(1_795_348_800));
    }

    #[test]
    fn values_may_contain_equals_and_are_unquoted() {
        let (_, cookie) = Jar::parse_set_cookie(r#"token="a=b=c"; Path=/"#).unwrap();
        assert_eq!(cookie.value, "a=b=c");
    }

    #[test]
    fn max_age_zero_deletes() {
        let mut jar = jar_with(&[("user_credentials", "1")]);
        assert!(jar.contains("user_credentials"));
        assert!(jar.apply("user_credentials=; Max-Age=0; Path=/"));
        assert!(!jar.contains("user_credentials"));
    }

    #[test]
    fn huge_max_age_does_not_overflow() {
        let mut jar = Jar::new();
        jar.apply("x=1; Max-Age=9223372036854775807");
        assert!(jar.contains("x"), "cookie must survive a saturating expiry");
    }

    #[test]
    fn huge_expires_year_is_ignored_not_panicking() {
        let (_, cookie) =
            Jar::parse_set_cookie("x=1; Expires=Fri, 31 Dec 999999999 23:59:59 GMT").unwrap();
        assert_eq!(cookie.expires_unix, None);
    }

    #[test]
    fn past_expiry_behaves_as_deletion() {
        let mut jar = jar_with(&[("x", "1")]);
        jar.apply("x=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT");
        assert!(!jar.contains("x"));
    }

    #[test]
    fn unparseable_expiry_is_a_session_cookie() {
        let (_, cookie) = Jar::parse_set_cookie("x=1; Expires=not-a-date").unwrap();
        assert_eq!(cookie.expires_unix, None);
        let mut jar = Jar::new();
        jar.apply("x=1; Expires=not-a-date");
        assert!(jar.contains("x"));
    }

    #[test]
    fn header_is_sorted_and_skips_expired() {
        let mut jar = Jar::new();
        jar.apply("b=2; Path=/");
        jar.apply("a=1; Path=/");
        jar.apply("dead=; Max-Age=0");
        assert_eq!(jar.cookie_header().unwrap(), "a=1; b=2");
    }

    #[test]
    fn completeness_needs_both_cookies() {
        assert!(!jar_with(&[("_otwarchive_session", "s")]).is_complete());
        assert!(!jar_with(&[("user_credentials", "1")]).is_complete());
        assert!(jar_with(&[("_otwarchive_session", "s"), ("user_credentials", "1")]).is_complete());
    }

    #[test]
    fn encrypted_round_trip() {
        let dir = private_dir("encrypt");
        let key = SecretKey::load_or_create(&dir.join("key")).unwrap();
        let jar = jar_with(&[("_otwarchive_session", "s"), ("user_credentials", "1")]);
        let cipher = jar.encrypt(&key).unwrap();
        let back = Jar::decrypt(&key, &cipher).unwrap();
        assert_eq!(back.cookie_header(), jar.cookie_header());
        drop(fs::remove_dir_all(dir));
    }

    #[test]
    fn jar_debug_does_not_leak_values() {
        let jar = jar_with(&[("_otwarchive_session", "super-secret")]);
        let text = format!("{jar:?}");
        assert!(!text.contains("super-secret"));
        assert!(text.contains("_otwarchive_session"));
    }

    #[test]
    fn http_date_parser_matches_known_epochs() {
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_http_date("Sun, 22 Nov 2026 12:00:00 GMT"),
            Some(1_795_348_800)
        );
        assert_eq!(parse_http_date("garbage"), None);
        assert_eq!(parse_http_date("Sun, 32 Nov 2026 12:00:00 GMT"), None);
    }

    #[test]
    fn clear_empties_the_jar() {
        let mut jar = jar_with(&[("a", "1")]);
        jar.clear();
        assert!(jar.is_empty());
        assert_eq!(jar.cookie_header(), None);
    }
}
