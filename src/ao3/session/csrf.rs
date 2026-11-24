//! CSRF token acquisition, synchronization, and write-with-single-retry helpers.

use serde::Deserialize;

use crate::ao3::client::Response;
use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::consts;
use crate::crypto::SecretString;
use crate::error::{Error, Result};

use super::{AccountSession, SessionState, validate_account};

#[derive(Deserialize)]
struct TokenDispenser {
    token: String,
}

#[derive(Clone, Copy)]
enum WriteMethod {
    Post,
    Patch,
    Delete,
}

impl AccountSession {
    /// The masked CSRF token for mutating requests, if the session has one.
    pub fn csrf_token(&self) -> Result<&str> {
        self.csrf
            .as_ref()
            .map(SecretString::expose)
            .ok_or(Error::CsrfMissing)
    }

    /// Keeps the in-memory CSRF token in sync after a direct `Client` read:
    /// clears any stale token if cookies rotated, and captures a fresh token
    /// from an authenticated HTML response when present.
    pub fn sync_csrf_from_response(&mut self, response: &Response) {
        if response.jar_changed {
            self.csrf = None;
        }
        if response.status == 200
            && !self.account.is_empty()
            && self.jar().is_complete()
            && let Some(token) = crate::ao3::parse::csrf_token_from_html(&response.body)
        {
            self.csrf = Some(SecretString::new(token));
        }
    }

    /// CSRF-protected form POST for a write action.
    ///
    /// Lazily refreshes a missing token and retries once if AO3 rejects a stale
    /// token with a redirect to `/auth_error` (Rails aborts before running the
    /// controller action, so one retry cannot double-post).
    pub async fn post_write(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        self.send_write_with_csrf_retry(WriteMethod::Post, path, fields)
            .await
    }

    /// CSRF-protected PATCH for a write action (reading flags).
    pub async fn patch_write(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        self.send_write_with_csrf_retry(WriteMethod::Patch, path, fields)
            .await
    }

    /// CSRF-protected DELETE for a write action (e.g. unsubscribe).
    pub async fn delete_write(&mut self, path: &str) -> Result<Response> {
        self.send_write_with_csrf_retry(WriteMethod::Delete, path, &[])
            .await
    }

    async fn send_write_with_csrf_retry(
        &mut self,
        method: WriteMethod,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<Response> {
        let token = self.ensure_csrf_token().await?;
        let response = self.send_write_once(method, path, fields, &token).await?;
        if is_auth_error_redirect(&response) {
            self.csrf = None;
            if self.fetch_csrf().await.is_ok()
                && let Ok(fresh) = self.csrf_token().map(str::to_string)
            {
                return self.send_write_once(method, path, fields, &fresh).await;
            }
        }
        Ok(response)
    }

    async fn send_write_once(
        &mut self,
        method: WriteMethod,
        path: &str,
        fields: &[(&str, &str)],
        token: &str,
    ) -> Result<Response> {
        match method {
            WriteMethod::Post => {
                let mut all: Vec<(&str, &str)> = fields.to_vec();
                all.push(("authenticity_token", token));
                self.upstream_post_form(path, &all).await
            }
            WriteMethod::Patch => {
                let mut all: Vec<(&str, &str)> = fields.to_vec();
                all.push(("authenticity_token", token));
                self.upstream_patch_form(path, &all).await
            }
            WriteMethod::Delete => self.upstream_delete_with_csrf(path, token).await,
        }
    }

    async fn ensure_csrf_token(&mut self) -> Result<String> {
        if self.csrf.is_none() {
            self.fetch_csrf().await?;
        }
        self.csrf_token().map(str::to_string)
    }

    /// Fetches a fresh CSRF token from `/token_dispenser.json`, falling back to
    /// an authenticated HTML page's `<meta name="csrf-token">` if the dispenser
    /// endpoint fails transiently (403/404/429/503 or network error).
    pub(super) async fn fetch_csrf(&mut self) -> Result<()> {
        let before = self.jar_signature();
        let response = match self
            .upstream_get(consts::TOKEN_PATH, Some(consts::TOKEN_TIMEOUT))
            .await
        {
            Ok(response) => response,
            Err(err) => {
                if self.try_csrf_from_html().await {
                    return Ok(());
                }
                return Err(err);
            }
        };
        if before != self.jar_signature() {
            self.csrf = None;
        }
        match response.status {
            200 => {
                let json = crate::ao3::solver::extract_json_body(&response.body);
                let parsed: TokenDispenser = serde_json::from_str(&json)
                    .map_err(|_| Error::UpstreamChanged("token dispenser is not JSON".into()))?;
                if parsed.token.is_empty() {
                    return Err(Error::CsrfMissing);
                }
                self.csrf = Some(SecretString::new(parsed.token));
                Ok(())
            }
            302 | 303 => {
                if response.location_contains("/lost_cookie") {
                    self.state = SessionState::SessionIncomplete;
                    self.client.set_jar(Jar::new());
                    return Err(Error::SessionIncomplete);
                }
                self.state = SessionState::LoggedOut;
                self.client.set_jar(Jar::new());
                Err(Error::LoggedOut)
            }
            status @ (403 | 404 | 429 | 503) => {
                if self.try_csrf_from_html().await {
                    return Ok(());
                }
                match status {
                    429 => Err(Error::RateLimited {
                        retry_after: response.retry_after,
                    }),
                    403 | 503 => Err(Error::BlockedByEdge { status }),
                    _ => Err(Error::UpstreamChanged(format!(
                        "token dispenser returned HTTP {status}"
                    ))),
                }
            }
            status => Err(Error::UpstreamChanged(format!(
                "token dispenser returned HTTP {status}"
            ))),
        }
    }

    async fn try_csrf_from_html(&mut self) -> bool {
        let Ok(account) = validate_account(&self.account).map(str::to_string) else {
            return false;
        };
        let path = format!(
            "/users/{}/subscriptions",
            percent_encode(&account, SpaceMode::Percent20)
        );
        let Ok(response) = self.upstream_get(&path, Some(consts::TOKEN_TIMEOUT)).await else {
            return false;
        };
        if response.status != 200 {
            return false;
        }
        let Some(token) = crate::ao3::parse::csrf_token_from_html(&response.body) else {
            return false;
        };
        self.csrf = Some(SecretString::new(token));
        true
    }
}

fn is_auth_error_redirect(response: &Response) -> bool {
    response.is_redirect() && response.location_contains("/auth_error")
}
