//! Minimal RFC 7578 `multipart/form-data` parser without extra crate deps,
//! used by the cookie-file upload endpoint.

/// Extracts the `boundary` parameter from a `multipart/form-data` header.
pub(super) fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let media = parts.next()?.trim();
    if !media.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    for param in parts {
        let Some((key, val)) = param.trim().split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            let trimmed = val.trim().trim_matches('"');
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extracts `(field_name, utf8_lossy_value)` pairs from a multipart body.
pub(super) fn parse_multipart_fields(boundary: &str, body: &[u8]) -> Vec<(String, String)> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut out = Vec::new();
    let mut cursor = match find_bytes(body, &delimiter) {
        Some(pos) => pos + delimiter.len(),
        None => return out,
    };

    while cursor < body.len() {
        // Epilogue marker `--` ends the multipart stream.
        if body[cursor..].starts_with(b"--") {
            break;
        }
        // Skip CRLF / LF right after the boundary token.
        if body[cursor..].starts_with(b"\r\n") {
            cursor += 2;
        } else if body[cursor..].starts_with(b"\n") {
            cursor += 1;
        }
        let next = match find_bytes(&body[cursor..], &delimiter) {
            Some(rel) => cursor + rel,
            None => break,
        };
        let mut part = &body[cursor..next];
        if let Some(stripped) = part.strip_suffix(b"\r\n") {
            part = stripped;
        } else if let Some(stripped) = part.strip_suffix(b"\n") {
            part = stripped;
        }

        let (header_bytes, value_bytes) = if let Some(sep) = find_bytes(part, b"\r\n\r\n") {
            (&part[..sep], &part[sep + 4..])
        } else if let Some(sep) = find_bytes(part, b"\n\n") {
            (&part[..sep], &part[sep + 2..])
        } else {
            cursor = next + delimiter.len();
            continue;
        };

        let headers_str = String::from_utf8_lossy(header_bytes);
        if let Some(name) = extract_disposition_name(&headers_str) {
            let value = String::from_utf8_lossy(value_bytes).into_owned();
            out.push((name, value));
        }
        cursor = next + delimiter.len();
    }
    out
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn extract_disposition_name(headers: &str) -> Option<String> {
    for line in headers.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        for param in rest.split(';') {
            let Some((k, v)) = param.trim().split_once('=') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case("name") {
                let name = v.trim().trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multipart_cookie_file_upload() {
        let ct = "multipart/form-data; boundary=\"----WebKitFormBoundary123\"";
        let boundary = extract_multipart_boundary(ct).expect("boundary");
        assert_eq!(boundary, "----WebKitFormBoundary123");

        let payload = concat!(
            "------WebKitFormBoundary123\r\n",
            "Content-Disposition: form-data; name=\"account\"\r\n\r\n",
            "someone\r\n",
            "------WebKitFormBoundary123\r\n",
            "Content-Disposition: form-data; name=\"cookie_file\"; filename=\"cookies.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "_otwarchive_session=s1; user_credentials=1\r\n",
            "------WebKitFormBoundary123\r\n",
            "Content-Disposition: form-data; name=\"adult\"\r\n\r\n",
            "1\r\n",
            "------WebKitFormBoundary123--\r\n"
        );
        let fields = parse_multipart_fields(&boundary, payload.as_bytes());
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("account".into(), "someone".into()));
        assert_eq!(
            fields[1],
            (
                "cookie_file".into(),
                "_otwarchive_session=s1; user_credentials=1".into()
            )
        );
        assert_eq!(fields[2], ("adult".into(), "1".into()));
    }
}
