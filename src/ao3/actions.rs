//! Write actions against AO3.
//!
//! Each function sends exactly one request (no retries: a duplicate public write
//! is worse than a failed one) and classifies the response into a typed result.
//! The confirmation step, intent ledger and governor live in the server layer.

use scraper::Html;
use serde::Deserialize;

use crate::ao3::compose::NewWorkInput;
use crate::ao3::html::{collapse_text, parse_selector};
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
            let json = crate::ao3::solver::extract_json_body(&response.body);
            let parsed: SuccessEnvelope = serde_json::from_str(&json).map_err(|_| {
                Error::UpstreamChanged("subscription success envelope is not JSON".into())
            })?;
            Ok(SubscribeOutcome {
                item_id: parsed.item_id,
                message: parsed.item_success_message,
            })
        }
        422 => {
            let json = crate::ao3::solver::extract_json_body(&response.body);
            let parsed: ErrorsEnvelope = serde_json::from_str(&json).unwrap_or_default();
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
            let json = crate::ao3::solver::extract_json_body(&response.body);
            let parsed: SuccessEnvelope = serde_json::from_str(&json).unwrap_or_default();
            Ok(parsed.item_success_message)
        }
        404 => Err(Error::Validation(
            "AO3 no longer has that subscription (already removed)".into(),
        )),
        422 => {
            let json = crate::ao3::solver::extract_json_body(&response.body);
            let parsed: ErrorsEnvelope = serde_json::from_str(&json).unwrap_or_default();
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
            let json = crate::ao3::solver::extract_json_body(&response.body);
            let parsed: ErrorMessageEnvelope = serde_json::from_str(&json).unwrap_or_default();
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
    let response = session.upstream_get(&path, None).await?;
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

/// How AO3 should persist a composed work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostMode {
    /// Publish now (`post_button`): the work becomes public.
    Publish,
    /// Save as an unposted draft (`save_button`); AO3 deletes drafts after ~29 days.
    Draft,
}

/// Result of one create request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedWork {
    pub work_id: Option<String>,
    pub draft: bool,
}

/// `POST /works` — creates one work (and its first chapter) through AO3's own
/// posting form.
///
/// Exactly one request: the caller owns the confirmation step, intent/audit rows
/// and the governor. Creation is not idempotent, so it is never retried here; the
/// CSRF refresh inside `post_write` is safe because `/auth_error` means the
/// request was refused before anything was created.
pub async fn post_work(
    session: &mut AccountSession,
    work: &NewWorkInput,
    mode: PostMode,
) -> Result<PostedWork> {
    let mut fields: Vec<(&str, String)> = vec![
        ("utf8", "\u{2713}".to_string()),
        ("work[title]", work.title.trim().to_string()),
        ("work[rating_string]", work.rating.trim().to_string()),
        ("work[fandom_string]", work.fandom.trim().to_string()),
        ("work[language_id]", work.language_id.trim().to_string()),
        ("work[chapter_attributes][content]", work.content.clone()),
    ];
    for warning in work
        .warnings
        .iter()
        .filter(|warning| !warning.trim().is_empty())
    {
        fields.push(("work[archive_warning_strings][]", warning.clone()));
    }
    for category in work
        .categories
        .iter()
        .filter(|category| !category.trim().is_empty())
    {
        fields.push(("work[category_strings][]", category.clone()));
    }
    for (key, value) in [
        ("work[relationship_string]", &work.relationships),
        ("work[character_string]", &work.characters),
        ("work[freeform_string]", &work.additional_tags),
        ("work[summary]", &work.summary),
        ("work[notes]", &work.notes),
        ("work[collection_names]", &work.collections),
        ("work[recipients]", &work.recipients),
        ("work[chapter_attributes][title]", &work.chapter_title),
        ("work[author_attributes][ids][]", &work.pseud_id),
    ] {
        if !value.trim().is_empty() {
            fields.push((key, value.clone()));
        }
    }
    match mode {
        PostMode::Publish => fields.push(("post_button", "Post".to_string())),
        PostMode::Draft => fields.push(("save_button", "Save as Draft".to_string())),
    }

    let encoded: Vec<(&str, &str)> = fields
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let response = session.post_write("/works", &encoded).await?;
    match response.status {
        302 | 303 => {
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/lost_cookie") {
                return Err(Error::SessionIncomplete);
            }
            let location = response.location;
            let Some(work_id) = work_id_from_location(location.as_deref()) else {
                return Err(Error::UpstreamChanged(format!(
                    "work create redirected to {location:?}"
                )));
            };
            let draft = mode == PostMode::Draft
                || location
                    .as_deref()
                    .is_some_and(|target| target.contains("/preview"));
            Ok(PostedWork {
                work_id: Some(work_id),
                draft,
            })
        }
        // The form is re-rendered with `#error` blocks when validation fails.
        200 | 422 => Err(classify_work_failure(&response.body)),
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "work create returned HTTP {status}"
        ))),
    }
}

/// `PUT /works/:id/post_draft` — publishes one draft.
///
/// AO3 answers a successful publish with a 302 to `/works/<id>`; a draft that
/// cannot be posted (already posted, invalid, moderated collection problem)
/// redirects to the owner's edit page instead, which is the typed failure here.
pub async fn publish_draft(session: &mut AccountSession, work_id: &str) -> Result<()> {
    let path = format!("/works/{}/post_draft", work_id.trim());
    let response = session.put_write(&path, &[]).await?;
    match response.status {
        302 | 303 => {
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/lost_cookie") {
                return Err(Error::SessionIncomplete);
            }
            publish_draft_result(work_id, response.location.as_deref())
        }
        200 | 422 => Err(classify_work_failure(&response.body)),
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "publish draft returned HTTP {status}"
        ))),
    }
}

fn publish_draft_result(work_id: &str, location: Option<&str>) -> Result<()> {
    let location = location.unwrap_or_default();
    if location.contains("/edit") {
        return Err(Error::Validation(
            "AO3 could not publish this draft — it may already be posted or invalid".to_string(),
        ));
    }
    if work_id_from_location(Some(location)).as_deref() == Some(work_id.trim())
        || location.contains("/works/")
    {
        return Ok(());
    }
    Err(Error::UpstreamChanged(format!(
        "publish draft redirected to {location:?}"
    )))
}

/// `DELETE /works/:id` — deletes one work (posted or draft).
///
/// AO3 redirects to `/users/<login>/works` (posted) or
/// `/users/<login>/works/drafts` (draft) on success; both prove the delete.
pub async fn delete_work(session: &mut AccountSession, work_id: &str) -> Result<()> {
    let path = format!("/works/{}", work_id.trim());
    let response = session.delete_write(&path).await?;
    match response.status {
        302 | 303 => {
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/lost_cookie") {
                return Err(Error::SessionIncomplete);
            }
            delete_work_result(response.location.as_deref())
        }
        200 | 422 => Err(classify_work_failure(&response.body)),
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "work delete returned HTTP {status}"
        ))),
    }
}

fn delete_work_result(location: Option<&str>) -> Result<()> {
    let location = location.unwrap_or_default();
    if location.contains("/users/") && location.contains("/works") {
        return Ok(());
    }
    Err(Error::UpstreamChanged(format!(
        "work delete redirected to {location:?}"
    )))
}

/// How a chapter submission should be saved on AO3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChapterSave {
    /// New chapter, posted immediately (`post_without_preview_button`).
    Post,
    /// New chapter, saved as an unposted chapter draft (`save_button`).
    Draft,
    /// Existing chapter (`update_button`, one `PUT`).
    Update,
}

/// Result of one chapter submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedChapter {
    pub chapter_id: Option<String>,
    pub draft: bool,
}

/// Creates or updates one chapter through AO3's own chapter form.
///
/// Exactly one request: the caller owns the confirmation step, intent/audit rows
/// and the governor. Chapter saving is not idempotent for creates, so nothing is
/// retried here (the CSRF refresh inside the session is the only exception; Rails
/// aborts `/auth_error` before the controller runs).
pub async fn save_chapter(
    session: &mut AccountSession,
    work_id: &str,
    chapter_id: Option<&str>,
    fields: &crate::ao3::chapters::ChapterFields,
    mode: ChapterSave,
) -> Result<SavedChapter> {
    let updating = chapter_id.is_some();
    let mut form: Vec<(&str, String)> = vec![
        ("utf8", "\u{2713}".to_string()),
        ("chapter[content]", fields.content.clone()),
    ];
    let mut push = |name: &'static str, value: &str| {
        if !value.trim().is_empty() || (updating && name != "chapter[position]") {
            form.push((name, value.to_string()));
        }
    };
    push("chapter[title]", fields.title.trim());
    push("chapter[position]", fields.position.trim());
    push("chapter[wip_length]", fields.wip_length.trim());
    push("chapter[summary]", fields.summary.trim());
    push("chapter[notes]", fields.notes.trim());
    push("chapter[endnotes]", fields.endnotes.trim());

    let path = match chapter_id {
        Some(chapter_id) => format!("/works/{}/chapters/{chapter_id}", work_id.trim()),
        None => format!("/works/{}/chapters", work_id.trim()),
    };
    form.push((
        match mode {
            ChapterSave::Post => "post_without_preview_button",
            ChapterSave::Draft => "save_button",
            ChapterSave::Update => "update_button",
        },
        match mode {
            ChapterSave::Post => "Post",
            ChapterSave::Draft => "Save as Draft",
            ChapterSave::Update => "Update",
        }
        .to_string(),
    ));
    let encoded: Vec<(&str, &str)> = form
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let response = match mode {
        ChapterSave::Update => session.put_write(&path, &encoded).await?,
        ChapterSave::Post | ChapterSave::Draft => session.post_write(&path, &encoded).await?,
    };
    match response.status {
        302 | 303 => {
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/lost_cookie") {
                return Err(Error::SessionIncomplete);
            }
            let location = response.location;
            classify_chapter_redirect(location.as_deref(), mode, chapter_id)
        }
        200 | 422 => Err(classify_work_failure(&response.body)),
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "chapter save returned HTTP {status}"
        ))),
    }
}

fn classify_chapter_redirect(
    location: Option<&str>,
    mode: ChapterSave,
    expected_chapter_id: Option<&str>,
) -> Result<SavedChapter> {
    let location = location.unwrap_or_default();
    let chapter_id = crate::ao3::chapters::chapter_id_from_path(location);
    if mode == ChapterSave::Update {
        // Only the chapter's own path proves the update; a 302 to `/works/<id>`
        // is AO3's ownership / "chapter not found" redirect, not a save.
        let expected = expected_chapter_id.map(str::trim);
        return match (chapter_id, expected) {
            (Some(found), Some(expected)) if found == expected => Ok(SavedChapter {
                chapter_id: Some(found),
                draft: false,
            }),
            (Some(found), None) => Ok(SavedChapter {
                chapter_id: Some(found),
                draft: false,
            }),
            _ => Err(Error::UpstreamChanged(format!(
                "chapter update redirected to {location:?} (permission or not-found)"
            ))),
        };
    }
    if let Some(chapter_id) = chapter_id {
        Ok(SavedChapter {
            chapter_id: Some(chapter_id),
            draft: location.contains("/preview"),
        })
    } else {
        Err(Error::UpstreamChanged(format!(
            "chapter create redirected to {location:?}"
        )))
    }
}

fn update_work_fields(fields: &crate::ao3::compose::EditWorkFields) -> Vec<(&'static str, String)> {
    let mut form: Vec<(&'static str, String)> = vec![
        ("utf8", "\u{2713}".to_string()),
        ("work[title]", fields.title.trim().to_string()),
        ("work[rating_string]", fields.rating.trim().to_string()),
        ("work[fandom_string]", fields.fandom.trim().to_string()),
        ("work[language_id]", fields.language_id.trim().to_string()),
        ("work[summary]", fields.summary.trim().to_string()),
        ("work[notes]", fields.notes.trim().to_string()),
        ("work[endnotes]", fields.endnotes.trim().to_string()),
        (
            "work[collection_names]",
            fields.collections.trim().to_string(),
        ),
        ("work[recipients]", fields.recipients.trim().to_string()),
    ];
    for warning in &fields.warnings {
        form.push(("work[archive_warning_strings][]", warning.clone()));
    }
    if fields.categories.is_empty() {
        // AO3's own form emits a hidden empty array value so unchecking every
        // box clears the selection; mirror it (an absent key keeps old values).
        form.push(("work[category_strings][]", String::new()));
    } else {
        for category in &fields.categories {
            form.push(("work[category_strings][]", category.clone()));
        }
    }
    // Free-text tag fields are always sent (even empty) so clearing them works.
    for (name, value) in [
        ("work[relationship_string]", fields.relationships.as_str()),
        ("work[character_string]", fields.characters.as_str()),
        ("work[freeform_string]", fields.additional_tags.as_str()),
    ] {
        form.push((name, value.trim().to_string()));
    }
    form
}

/// `PATCH /works/:id` — updates one work's metadata/tags (not chapter text).
///
/// AO3 answers a successful update with a 302 to the work page; validation
/// failures re-render the edit page with HTTP 200 and an `#error` block. A
/// non-author is redirected to the work page too (AO3's ownership filter), so
/// the UI must only offer this on the account's own works.
pub async fn update_work(
    session: &mut AccountSession,
    work_id: &str,
    fields: &crate::ao3::compose::EditWorkFields,
) -> Result<()> {
    let form = update_work_fields(fields);
    let encoded: Vec<(&str, &str)> = form
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let path = format!("/works/{}", work_id.trim());
    let response = session.patch_write(&path, &encoded).await?;
    match response.status {
        302 | 303 => {
            if response.location_contains("/users/login") {
                return Err(Error::LoggedOut);
            }
            if response.location_contains("/auth_error") {
                return Err(Error::CsrfExpired);
            }
            if response.location_contains("/lost_cookie") {
                return Err(Error::SessionIncomplete);
            }
            update_work_result(work_id, response.location.as_deref())
        }
        200 | 422 => Err(classify_work_failure(&response.body)),
        429 => Err(Error::RateLimited {
            retry_after: response.retry_after,
        }),
        403 | 503 => Err(Error::BlockedByEdge {
            status: response.status,
        }),
        status => Err(Error::UpstreamChanged(format!(
            "work update returned HTTP {status}"
        ))),
    }
}

fn update_work_result(work_id: &str, location: Option<&str>) -> Result<()> {
    let location = location.unwrap_or_default();
    let expected = format!("/works/{}", work_id.trim());
    if location.contains(&expected)
        && !location.contains(&format!("{expected}/edit"))
        && !location.contains(&format!("{expected}/preview"))
    {
        return Ok(());
    }
    Err(Error::UpstreamChanged(format!(
        "work update redirected to {location:?} (permission or layout problem)"
    )))
}

/// First `/works/<id>` in a redirect target (`/works/123` or `/works/123/preview`).
fn work_id_from_location(location: Option<&str>) -> Option<String> {
    let rest = location?
        .split('/')
        .skip_while(|segment| *segment != "works")
        .nth(1)?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

/// AO3's `#error` block (`app/helpers/validation_helper.rb:32-51`), else a typed
/// generic refusal. Messages are AO3's own, already user-facing text.
fn classify_work_failure(body: &str) -> Error {
    let messages = error_messages(body);
    if messages.is_empty() {
        Error::Validation("AO3 rejected the work without showing a reason".to_string())
    } else {
        Error::Validation(messages.join(" "))
    }
}

fn error_messages(html: &str) -> Vec<String> {
    let Ok(selector) = parse_selector("#error li") else {
        return Vec::new();
    };
    let document = Html::parse_document(html);
    document
        .select(&selector)
        .map(|item| collapse_text(&item))
        .filter(|message| !message.is_empty())
        .collect()
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
    fn work_ids_and_draft_redirects_are_parsed() {
        assert_eq!(
            work_id_from_location(Some("/works/777")),
            Some("777".to_string())
        );
        assert_eq!(
            work_id_from_location(Some("/works/778/preview")),
            Some("778".to_string())
        );
        assert_eq!(work_id_from_location(Some("/users/someone")), None);
        assert_eq!(work_id_from_location(Some("/users/login")), None);
        assert_eq!(work_id_from_location(None), None);
    }

    #[test]
    fn work_form_errors_are_read_from_aos_error_block() {
        let html = r#"<div id="error" class="error">
            <h4>Sorry! We couldn&#39;t save this work because:</h4>
            <ul><li>Title must be at least 1 characters long without leading spaces.</li>
            <li>Language cannot be blank.</li></ul></div>"#;
        assert_eq!(
            error_messages(html),
            vec![
                "Title must be at least 1 characters long without leading spaces.".to_string(),
                "Language cannot be blank.".to_string()
            ]
        );
        assert!(error_messages("<html><body>no errors</body></html>").is_empty());
    }

    #[test]
    fn draft_publish_and_delete_redirects_are_classified() {
        assert!(publish_draft_result("777", Some("/works/777")).is_ok());
        assert!(publish_draft_result("777", Some("https://archiveofourown.org/works/777")).is_ok());
        let error = publish_draft_result("777", Some("/users/someone/works/777/edit"))
            .expect_err("edit redirect is a refusal");
        assert!(matches!(error, Error::Validation(_)), "{error:?}");
        assert!(publish_draft_result("777", Some("/users/login")).is_err());
        assert!(delete_work_result(Some("/users/someone/works")).is_ok());
        assert!(delete_work_result(Some("/users/someone/works/drafts")).is_ok());
        assert!(delete_work_result(Some("/")).is_err());
    }

    #[test]
    fn chapter_redirects_are_classified_by_path() {
        let posted =
            classify_chapter_redirect(Some("/works/123/chapters/456"), ChapterSave::Post, None)
                .expect("posted");
        assert_eq!(posted.chapter_id.as_deref(), Some("456"));
        assert!(!posted.draft);
        let draft = classify_chapter_redirect(
            Some("/works/123/chapters/457/preview"),
            ChapterSave::Draft,
            None,
        )
        .expect("draft");
        assert_eq!(draft.chapter_id.as_deref(), Some("457"));
        assert!(draft.draft);
        assert!(
            classify_chapter_redirect(
                Some("/works/123/chapters/457"),
                ChapterSave::Update,
                Some("457")
            )
            .is_ok()
        );
        let refused =
            classify_chapter_redirect(Some("/works/123"), ChapterSave::Update, Some("457"))
                .expect_err("a work redirect is a permission/not-found refusal");
        assert!(matches!(refused, Error::UpstreamChanged(_)), "{refused:?}");
        assert!(
            classify_chapter_redirect(Some("/users/someone"), ChapterSave::Post, None).is_err()
        );
    }

    #[test]
    fn clearing_optional_tags_is_sent_explicitly() {
        let fields = crate::ao3::compose::EditWorkFields {
            title: "T".into(),
            rating: "General Audiences".into(),
            warnings: vec!["No Archive Warnings Apply".into()],
            fandom: "F".into(),
            language_id: "1".into(),
            ..crate::ao3::compose::EditWorkFields::default()
        };
        let form = update_work_fields(&fields);
        let has =
            |name: &str, value: &str| form.iter().any(|(key, sent)| *key == name && sent == value);
        assert!(
            has("work[category_strings][]", ""),
            "empty categories clear"
        );
        assert!(
            has("work[relationship_string]", ""),
            "empty relationships clear"
        );
        assert!(has("work[character_string]", ""), "empty characters clear");
        assert!(
            has("work[freeform_string]", ""),
            "empty additional tags clear"
        );
        assert!(has(
            "work[archive_warning_strings][]",
            "No Archive Warnings Apply"
        ));

        let with_categories = crate::ao3::compose::EditWorkFields {
            categories: vec!["F/M".into()],
            ..fields
        };
        let form = update_work_fields(&with_categories);
        assert!(
            form.iter()
                .any(|(key, value)| *key == "work[category_strings][]" && value == "F/M")
        );
        assert!(
            !form
                .iter()
                .any(|(key, value)| *key == "work[category_strings][]" && value.is_empty())
        );
    }

    #[test]
    fn work_update_redirects_are_classified() {
        assert!(update_work_result("7701", Some("/works/7701")).is_ok());
        assert!(update_work_result("7701", Some("https://archiveofourown.org/works/7701")).is_ok());
        assert!(update_work_result("7701", Some("/works/7701/edit")).is_err());
        assert!(update_work_result("7701", Some("/works/7702")).is_err());
        assert!(update_work_result("7701", Some("/users/someone")).is_err());
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
