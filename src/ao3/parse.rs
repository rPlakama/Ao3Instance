//! Tiny parsers for the pieces of AO3 HTML we must read, plus the one
//! percent-encoder both call sites share.
//!
//! Deliberately dependency-free and narrow: we only ever need the CSRF meta tag,
//! a flash message, percent-(de)encoding. A general HTML parser would be a
//! dependency that hides drift instead of surfacing it.

/// How a space is encoded. One implementation, two call sites (`inherit` rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceMode {
    /// `application/x-www-form-urlencoded`: space becomes `+`.
    Plus,
    /// RFC 3986 path segment: every byte outside the unreserved set becomes `%XX`.
    Percent20,
}

/// Percent-encodes `text` per [`SpaceMode`].
pub fn percent_encode(text: &str, space: SpaceMode) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            b' ' if space == SpaceMode::Plus => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Decodes `%XX` escapes. `+` is left alone (it is a literal plus in a path).
///
/// Used to recover the canonical login from a `Location` such as
/// `/users/some_one`. Invalid input yields `None` rather than a best guess.
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_nibble(bytes[index + 1])?;
            let lo = hex_nibble(bytes[index + 2])?;
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Extracts `<meta name="csrf-token" content="…">`.
///
/// Rails' `csrf_meta_tag` emits that attribute order. If AO3 ever changes it the
/// function returns `None` and the caller raises `CsrfMissing` — visible, not
/// silently wrong.
pub fn csrf_token_from_meta(html: &str) -> Option<&str> {
    let anchor = html.find("name=\"csrf-token\"")?;
    let rest = &html[anchor..];
    let content = rest.find("content=\"")?;
    let tail = &rest[content + "content=\"".len()..];
    let end = tail.find('"')?;
    Some(&tail[..end])
}

/// Extracts the text of the first `class="flash …"` block, if present.
///
/// AO3's session layout renders flash slots `:error, :caution, :notice, :alert`
/// (`app/views/layouts/session.html.erb`), and Devise login failures use the
/// `alert` slot, so matching only `flash error` would miss every real rejection.
pub fn flash_message(html: &str) -> Option<String> {
    let marker = html.find("class=\"flash")?;
    let after = &html[marker..];
    let open = after.find('>')?;
    let body = &after[open + 1..];
    // Stop at the closing div of the flash block; nested tags are stripped below.
    let close = body.find("</div>")?;
    let text = collapse_whitespace(&strip_tags(&body[..close]));
    if text.is_empty() { None } else { Some(text) }
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN_PAGE: &str = r#"<!DOCTYPE html>
<html><head><meta name="csrf-token" content="abc123+/=" /></head>
<body><div id="loginform"></div></body></html>"#;

    #[test]
    fn extracts_csrf_meta() {
        assert_eq!(csrf_token_from_meta(LOGIN_PAGE), Some("abc123+/="));
    }

    #[test]
    fn missing_csrf_meta_is_none() {
        assert_eq!(csrf_token_from_meta("<html></html>"), None);
        assert_eq!(csrf_token_from_meta("<meta name=\"csrf-token\" />"), None);
    }

    #[test]
    fn extracts_flash_alert_like_the_real_layout() {
        // Shape from app/views/layouts/session.html.erb: class="flash alert".
        let html = r#"<div class="flash alert" role="alert">
        <p>Invalid login or password.</p>
        </div>"#;
        assert_eq!(
            flash_message(html).as_deref(),
            Some("Invalid login or password.")
        );
    }

    #[test]
    fn extracts_flash_error_too() {
        let html = r#"<div class="flash error" role="alert"><p>Bad</p></div>"#;
        assert_eq!(flash_message(html).as_deref(), Some("Bad"));
    }

    #[test]
    fn no_flash_is_none() {
        assert_eq!(flash_message("<html></html>"), None);
        assert_eq!(flash_message(r#"<div class="flash error"></div>"#), None);
    }

    #[test]
    fn one_encoder_two_modes() {
        assert_eq!(percent_encode("a b", SpaceMode::Plus), "a+b");
        assert_eq!(percent_encode("a b", SpaceMode::Percent20), "a%20b");
        assert_eq!(percent_encode("a/b", SpaceMode::Percent20), "a%2Fb");
        assert_eq!(percent_encode("hé", SpaceMode::Percent20), "h%C3%A9");
        assert_eq!(
            percent_encode("user[login]", SpaceMode::Plus),
            "user%5Blogin%5D"
        );
    }

    #[test]
    fn percent_decode_round_trips_and_rejects_garbage() {
        assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
        assert_eq!(percent_decode("some_one").as_deref(), Some("some_one"));
        assert_eq!(percent_decode("a+b").as_deref(), Some("a+b"));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%2"), None);
    }
}
