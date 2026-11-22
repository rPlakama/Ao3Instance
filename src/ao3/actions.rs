//! Write actions against AO3.
//!
//! Each function sends exactly one request (no retries: a duplicate public write
//! is worse than a failed one) and classifies the response into a typed result.
//! The confirmation step, intent ledger and governor live in the server layer.

use serde::Deserialize;

use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::ao3::pseuds::{self, Pseud};
use crate::ao3::session::AccountSession;
use crate::error::{Error, Result};

/// The subscribable types AO3 accepts (`Subscription::VALID_SUBSCRIBABLES`).
pub const WORK: &str = "Work";
pub const USER: &str = "User";
pub const SERIES: &str = "Series";

/// Normalises a kind from a form/query parameter.
pub fn normalize_kind(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "work" | "works" => Some(WORK),
        "user" | "author" | "authors" => Some(USER),
        "series" => Some(SERIES),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeOutcome {
    pub item_id: Option<i64>,
    pub message: Option<String>,
}

#[derive(Deserialize, Default)]
struct SuccessEnvelope {
    #[serde(default)]
    item_id: Option<i64>,
    #[serde(default)]
    item_success_message: Option<String>,
}

#[derive(Deserialize, Default)]
struct ErrorMessageEnvelope {
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KudosOutcome {
    /// AO3 said we had already left kudos: success for our purposes.
    pub already: bool,
    pub message: Option<String>,
}

#[derive(Deserialize, Default)]
struct ErrorsEnvelope {
    #[serde(default)]
    errors: Vec<String>,
}

fn classified_error(response: &crate::ao3::client::Response) -> Error {
    match response.status {
        302 | 303 if response.location_contains("/users/login") => Error::LoggedOut,
        302 | 303 if response.location_contains("/auth_error") => Error::CsrfExpired,
        429 => Error::RateLimited {
            retry_after: response.retry_after,
        },
        403 | 503 => Error::BlockedByEdge {
            status: response.status,
        },
        status => Error::UpstreamChanged(format!("subscription action returned HTTP {status}")),
    }
}

/// `POST /users/:login/subscriptions` as JSON (the only clean write API AO3 has).
pub async fn subscribe(
    session: &mut AccountSession,
    login: &str,
    kind: &str,
    target_id: &str,
) -> Result<SubscribeOutcome> {
    if !matches!(kind, WORK | USER | SERIES) {
        return Err(Error::Validation(format!(
            "cannot subscribe to a {kind}: AO3 supports Work, User and Series"
        )));
    }
    if target_id.trim().is_empty() {
        return Err(Error::Validation("subscription target is empty".into()));
    }
    let path = format!(
        "/users/{}/subscriptions.json",
        percent_encode(login, SpaceMode::Percent20)
    );
    let response = session
        .post_write(
            &path,
            &[
                ("subscription[subscribable_id]", target_id),
                ("subscription[subscribable_type]", kind),
            ],
        )
        .await?;

    match response.status {
        200 | 201 => {
            let parsed: SuccessEnvelope = serde_json::from_str(&response.body).map_err(|_| {
                Error::UpstreamChanged("subscription success envelope is not JSON".into())
            })?;
            Ok(SubscribeOutcome {
                item_id: parsed.item_id,
                message: parsed.item_success_message,
            })
        }
        422 => {
            let parsed: ErrorsEnvelope = serde_json::from_str(&response.body).unwrap_or_default();
            let message = if parsed.errors.is_empty() {
                "AO3 rejected the subscription".to_string()
            } else {
                parsed.errors.join("; ")
            };
            Err(Error::Validation(message))
        }
        _ => Err(classified_error(&response)),
    }
}

/// `DELETE /users/:login/subscriptions/:id` as JSON.
pub async fn unsubscribe(
    session: &mut AccountSession,
    login: &str,
    subscription_id: i64,
) -> Result<Option<String>> {
    let path = format!(
        "/users/{}/subscriptions/{subscription_id}.json",
        percent_encode(login, SpaceMode::Percent20)
    );
    let response = session.delete_write(&path).await?;
    match response.status {
        200 | 204 => {
            let parsed: SuccessEnvelope = serde_json::from_str(&response.body).unwrap_or_default();
            Ok(parsed.item_success_message)
        }
        404 => Err(Error::Validation(
            "AO3 no longer has that subscription (already removed)".into(),
        )),
        422 => {
            let parsed: ErrorsEnvelope = serde_json::from_str(&response.body).unwrap_or_default();
            Err(Error::Validation(if parsed.errors.is_empty() {
                "AO3 rejected the unsubscribe".to_string()
            } else {
                parsed.errors.join("; ")
            }))
        }
        _ => Err(classified_error(&response)),
    }
}

/// `POST /kudos.js` — the JS format is machine-readable: 201 on success,
/// 422 with `error_message` on rejection (`kudos_controller.rb:52-72`).
pub async fn kudos(session: &mut AccountSession, work_id: &str) -> Result<KudosOutcome> {
    let work_id = work_id.trim();
    if work_id.is_empty() {
        return Err(Error::Validation("kudos target is empty".into()));
    }
    let response = session
        .post_write(
            "/kudos.js",
            &[
                ("kudo[commentable_id]", work_id),
                ("kudo[commentable_type]", WORK),
            ],
        )
        .await?;
    match response.status {
        200 | 201 => Ok(KudosOutcome {
            already: false,
            message: None,
        }),
        422 => {
            let parsed: ErrorMessageEnvelope =
                serde_json::from_str(&response.body).unwrap_or_default();
            let message = parsed
                .error_message
                .unwrap_or_else(|| "AO3 rejected the kudos".to_string());
            if message.contains("already left kudos") {
                Ok(KudosOutcome {
                    already: true,
                    message: Some(message),
                })
            } else {
                Err(Error::Validation(message))
            }
        }
        _ => Err(classified_error(&response)),
    }
}

/// `PATCH /works/:id/mark_for_later` or `/mark_as_read` (`works_controller.rb:704-722`).
///
/// Both set a state rather than toggling, so a lost response is safe to
/// re-issue; we still never retry automatically.
pub async fn set_reading_flag(
    session: &mut AccountSession,
    work_id: &str,
    for_later: bool,
) -> Result<()> {
    let work_id = work_id.trim();
    if work_id.is_empty() {
        return Err(Error::Validation("reading-flag target is empty".into()));
    }
    let action = if for_later {
        "mark_for_later"
    } else {
        "mark_as_read"
    };
    let path = format!("/works/{work_id}/{action}");
    let response = session.patch_write(&path, &[]).await?;
    match response.status {
        200 | 302 | 303 => {
            if response.location_contains("/auth_error") {
                Err(Error::CsrfExpired)
            } else if response.location_contains("/users/login") {
                Err(Error::LoggedOut)
            } else {
                Ok(())
            }
        }
        404 => Err(Error::Validation("work not found upstream".into())),
        _ => Err(classified_error(&response)),
    }
}

/// `GET /users/:login/pseuds` → the default pseud (needed for comments).
pub async fn discover_default_pseud(session: &mut AccountSession, login: &str) -> Result<Pseud> {
    let path = format!(
        "/users/{}/pseuds",
        percent_encode(login, SpaceMode::Percent20)
    );
    let response = session.client_mut().get(&path).await?;
    match response.status {
        200 => {
            pseuds::ensure_pseuds_page(&response.body)?;
            pseuds::default_pseud(&response.body)
                .ok_or_else(|| Error::Validation("AO3 did not show a pseud to comment with".into()))
        }
        302 | 303 if response.location_contains("/users/login") => Err(Error::LoggedOut),
        302 | 303 if response.location_contains("/auth_error") => Err(Error::CsrfExpired),
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "pseuds page returned HTTP {status}"
        ))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentOutcome {
    pub comment_id: Option<i64>,
}

/// AO3's comment limit (`app/models/comment.rb:21-23`, `config/config.yml:105`).
pub const COMMENT_MAX_CHARS: usize = 10_000;

/// `POST /works/:work_id/bookmarks` (HTML form; success is a redirect to the
/// new bookmark). The local receipt is the duplicate guard because AO3 allows
/// several bookmarks of the same work.
pub async fn bookmark(
    session: &mut AccountSession,
    work_id: &str,
    pseud_id: i64,
) -> Result<Option<i64>> {
    let work_id = work_id.trim();
    if work_id.is_empty() {
        return Err(Error::Validation("bookmark target is empty".into()));
    }
    let path = format!("/works/{work_id}/bookmarks");
    let pseud = pseud_id.to_string();
    let response = session
        .post_write(
            &path,
            &[
                ("bookmark[pseud_id]", pseud.as_str()),
                ("bookmark[private]", "0"),
            ],
        )
        .await?;
    match response.status {
        302 | 303 => {
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            Ok(bookmark_id_from_location(response.location.as_deref()))
        }
        200 => {
            if response.body.contains("can't be blank") || response.body.contains("error") {
                Err(Error::Validation(
                    "AO3 rejected the bookmark (validation or duplicate)".into(),
                ))
            } else {
                Err(Error::Validation("AO3 rejected the bookmark".into()))
            }
        }
        _ => Err(classified_error(&response)),
    }
}

fn bookmark_id_from_location(location: Option<&str>) -> Option<i64> {
    let location = location?;
    let marker = "/bookmarks/";
    let start = location.find(marker)? + marker.len();
    let digits: String = location[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse::<i64>().ok()
}

/// `POST /works/:work_id/comments`.
///
/// A 302 whose fragment carries `#comment_<id>` is success; a 200 is a
/// re-rendered form and its body names the reason (duplicate, spam, moderation).
pub async fn post_comment(
    session: &mut AccountSession,
    work_id: &str,
    pseud_id: i64,
    content: &str,
) -> Result<CommentOutcome> {
    let content = content.trim();
    if content.is_empty() {
        return Err(Error::Validation("the comment is empty".into()));
    }
    if content.chars().count() > COMMENT_MAX_CHARS {
        return Err(Error::Validation(format!(
            "the comment is longer than {COMMENT_MAX_CHARS} characters"
        )));
    }
    let path = format!("/works/{}/comments", work_id.trim());
    let pseud = pseud_id.to_string();
    let response = session
        .post_write(
            &path,
            &[
                ("comment[pseud_id]", pseud.as_str()),
                ("comment[comment_content]", content),
            ],
        )
        .await?;

    match response.status {
        302 | 303 => {
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            if response.location_contains("/429") {
                return Err(Error::RateLimited { retry_after: None });
            }
            Ok(CommentOutcome {
                comment_id: comment_id_from_location(response.location.as_deref()),
            })
        }
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        200 => Err(classify_comment_failure(&response.body)),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "comment create returned HTTP {status}"
        ))),
    }
}

/// Maps the re-rendered comment form's error text to a typed failure.
fn classify_comment_failure(body: &str) -> Error {
    if body.contains("already left this comment") {
        Error::Validation(
            "you already left this exact comment here (AO3 rejects duplicates)".into(),
        )
    } else if body.contains("looks like spam") {
        Error::SpamRejected
    } else if body.contains("can't add or edit comments on a hidden work") {
        Error::Validation("comments are closed on this work".into())
    } else if body.contains("can't add or edit comments on an unrevealed work") {
        Error::Validation("the work is in an unrevealed collection".into())
    } else if body.contains("Couldn't save comment") {
        Error::Validation("AO3 rejected the comment (validation or moderation rules)".into())
    } else {
        Error::Validation("AO3 rejected the comment".into())
    }
}

fn comment_id_from_location(location: Option<&str>) -> Option<i64> {
    let location = location?;
    let marker = "#comment_";
    if let Some(start) = location.find(marker) {
        let digits: String = location[start + marker.len()..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        return digits.parse::<i64>().ok();
    }
    let marker = "/comments/";
    if let Some(start) = location.find(marker) {
        let digits: String = location[start + marker.len()..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        return digits.parse::<i64>().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_ids_are_recovered_from_redirects() {
        assert_eq!(bookmark_id_from_location(Some("/bookmarks/900")), Some(900));
        assert_eq!(
            bookmark_id_from_location(Some("/users/x/bookmarks/12")),
            Some(12)
        );
        assert_eq!(bookmark_id_from_location(Some("/works/1")), None);
        assert_eq!(bookmark_id_from_location(None), None);
    }

    #[test]
    fn comment_ids_are_recovered_from_redirects() {
        assert_eq!(
            comment_id_from_location(Some("/works/1/chapters/2#comment_777")),
            Some(777)
        );
        assert_eq!(comment_id_from_location(Some("/comments/42")), Some(42));
        assert_eq!(comment_id_from_location(Some("/works/1")), None);
        assert_eq!(comment_id_from_location(None), None);
    }

    #[test]
    fn comment_failures_are_classified() {
        assert!(matches!(
            classify_comment_failure("<p>You've already left this comment here.</p>"),
            Error::Validation(_)
        ));
        assert!(matches!(
            classify_comment_failure("<p>This comment looks like spam to our system</p>"),
            Error::SpamRejected
        ));
        assert!(matches!(
            classify_comment_failure(
                "<p>Sorry, you can't add or edit comments on a hidden work.</p>"
            ),
            Error::Validation(_)
        ));
        assert!(matches!(
            classify_comment_failure("<p>Couldn't save comment!</p>"),
            Error::Validation(_)
        ));
    }

    #[test]
    fn kinds_are_normalised_and_bounded() {
        assert_eq!(normalize_kind("work"), Some(WORK));
        assert_eq!(normalize_kind(" Series "), Some(SERIES));
        assert_eq!(normalize_kind("author"), Some(USER));
        assert_eq!(normalize_kind("collection"), None);
        assert_eq!(normalize_kind(""), None);
    }
}
