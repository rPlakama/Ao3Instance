//! Write actions against AO3.
//!
//! Each function sends exactly one request (no retries: a duplicate public write
//! is worse than a failed one) and classifies the response into a typed result.
//! The confirmation step, intent ledger and governor live in the server layer.

use serde::Deserialize;

use crate::ao3::parse::{SpaceMode, percent_encode};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_normalised_and_bounded() {
        assert_eq!(normalize_kind("work"), Some(WORK));
        assert_eq!(normalize_kind(" Series "), Some(SERIES));
        assert_eq!(normalize_kind("author"), Some(USER));
        assert_eq!(normalize_kind("collection"), None);
        assert_eq!(normalize_kind(""), None);
    }
}
