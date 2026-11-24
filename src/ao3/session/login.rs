//! AO3 username/password login flow and redirect classification.

use crate::ao3::client::Response;
use crate::ao3::jar::Jar;
use crate::ao3::parse::percent_decode;
use crate::consts;
use crate::crypto::SecretString;
use crate::error::{Error, Result};

use super::{AccountSession, SessionState, validate_account};

impl AccountSession {
    /// Logs in with an AO3 username/password (ADR-0001 option B).
    pub async fn login(&mut self, account: &str, password: SecretString) -> Result<SessionState> {
        self.ensure_alive()?;
        let account = validate_account(account)?.to_string();
        match self.login_once(&account, password.expose()).await {
            Err(Error::CsrfExpired) if self.solver_status.reachable => {
                // A stale FlareSolverr browser session can hold an older
                // `_otwarchive_session` than the freshly parsed form token.
                // Reset the browser session and try once more.
                self.solver.reset_session().await;
                self.login_once(&account, password.expose()).await
            }
            other => other,
        }
    }

    async fn login_once(&mut self, account: &str, password: &str) -> Result<SessionState> {
        self.account = account.to_string();
        self.csrf = None;
        self.pseud = None;
        // One account per jar: never authenticate on top of a previous identity.
        self.client.set_jar(Jar::new());
        self.state = SessionState::LoggedOut;

        let login_page = match self.upstream_get(consts::LOGIN_PATH, None).await {
            Ok(page) => page,
            Err(err) => {
                self.wipe();
                return Err(err);
            }
        };
        if let Err(err) = self.classify_pre_login(&login_page) {
            if !matches!(err, Error::BlockedByEdge { .. }) {
                self.wipe();
            }
            return Err(err);
        }
        let Some(token) = crate::ao3::parse::csrf_token_from_html(&login_page.body) else {
            self.wipe();
            return Err(Error::CsrfMissing);
        };

        let response = match self
            .upstream_post_form(
                consts::LOGIN_PATH,
                &[
                    ("user[login]", account),
                    ("user[password]", password),
                    ("user[remember_me]", "1"),
                    ("authenticity_token", &token),
                ],
            )
            .await
        {
            Ok(response) => response,
            Err(err) => {
                self.wipe();
                return Err(err);
            }
        };

        match response.status {
            // Stock Rails re-renders the form with a flash on a failed
            // navigational login.
            200 => {
                let message = crate::ao3::parse::flash_message(&response.body)
                    .unwrap_or_else(|| "login rejected by upstream".to_string());
                self.wipe();
                Err(Error::LoginRejected(message))
            }
            302 | 303 => self.classify_login_redirect(&response).await,
            429 => {
                self.wipe();
                Err(Error::RateLimited {
                    retry_after: response.retry_after,
                })
            }
            403 | 503 => {
                self.wipe();
                self.state = SessionState::Blocked;
                Err(Error::BlockedByEdge {
                    status: response.status,
                })
            }
            status => {
                self.wipe();
                Err(Error::UpstreamChanged(format!(
                    "login returned HTTP {status}"
                )))
            }
        }
    }

    /// Classifies the page served for `GET /users/login`.
    fn classify_pre_login(&mut self, response: &Response) -> Result<()> {
        match response.status {
            200 => Ok(()),
            429 => Err(Error::RateLimited {
                retry_after: response.retry_after,
            }),
            403 | 503 => {
                // Wipe first: wipe() resets the state, so set Blocked after it.
                self.wipe();
                self.state = SessionState::Blocked;
                Err(Error::BlockedByEdge {
                    status: response.status,
                })
            }
            status => Err(Error::UpstreamChanged(format!(
                "login page returned HTTP {status}"
            ))),
        }
    }

    /// Decides what the post-login redirect means. A 302 alone is not success.
    async fn classify_login_redirect(&mut self, response: &Response) -> Result<SessionState> {
        let Some(location) = response.location.clone() else {
            self.wipe();
            return Err(Error::UpstreamChanged(
                "login redirect carried no Location".into(),
            ));
        };
        let Some(path) = self.location_path(&location) else {
            self.wipe();
            return Err(Error::UpstreamChanged(format!(
                "login redirected off-origin to {location:?}"
            )));
        };

        match classify_login_path(&path) {
            Some(LoginRedirect::AuthError) => {
                self.wipe();
                Err(Error::CsrfExpired)
            }
            Some(LoginRedirect::LoginPage) => {
                self.wipe();
                Err(Error::LoginRejected("login rejected by upstream".into()))
            }
            Some(LoginRedirect::ChangePassword) => {
                // Signed in, but AO3 demands a password change before use.
                self.wipe();
                Err(Error::PasswordChangeRequired)
            }
            Some(LoginRedirect::User(login)) => {
                self.account = login;
                self.state = SessionState::Imported;
                self.finish_validation().await
            }
            None => {
                self.wipe();
                Err(Error::UpstreamChanged(format!(
                    "login redirected to {location:?}"
                )))
            }
        }
    }

    /// The path of a same-origin (or root-relative, not protocol-relative)
    /// redirect target.
    fn location_path(&self, location: &str) -> Option<String> {
        redirect_path(location, self.client.base_url())
    }
}

/// Extracts the path of a same-origin or root-relative redirect.
///
/// Rejects absolute URLs to other origins and protocol-relative references
/// (`//host/path`), which would otherwise be misread as root-relative.
pub(super) fn redirect_path(location: &str, base: &str) -> Option<String> {
    if location.starts_with("//") {
        return None;
    }
    let rest = if let Some(rest) = location.strip_prefix(base) {
        rest
    } else if location.starts_with('/') {
        location
    } else {
        return None;
    };
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LoginRedirect {
    AuthError,
    LoginPage,
    ChangePassword,
    /// Success: the canonical login for the probe path.
    User(String),
}

/// Classifies a same-origin login redirect path.
///
/// Whole segments only: `/users/auth_error` is a *user*, `/auth_error` is the
/// CSRF page; `/users/someone/change_password` is the forced change, while a
/// user called `change_password` alone is just a user.
///
/// Known ambiguity: an account literally named `login` redirects to
/// `/users/login`, which is byte-identical to the login page. We fail closed
/// (`LoginRejected`); cookie import still works for such an account.
fn classify_login_path(path: &str) -> Option<LoginRedirect> {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match segments.as_slice() {
        ["auth_error"] => Some(LoginRedirect::AuthError),
        ["users", "login"] => Some(LoginRedirect::LoginPage),
        ["users", _login, "change_password"] => Some(LoginRedirect::ChangePassword),
        ["users", login] => percent_decode(login)
            .filter(|login| !login.is_empty())
            .map(LoginRedirect::User),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_redirects_are_classified_by_whole_segments() {
        assert_eq!(
            classify_login_path("/users/someone"),
            Some(LoginRedirect::User("someone".into()))
        );
        assert_eq!(
            classify_login_path("/users/some_one?x=1"),
            Some(LoginRedirect::User("some_one".into()))
        );
        assert_eq!(
            classify_login_path("/users/a%20b"),
            Some(LoginRedirect::User("a b".into()))
        );
        assert_eq!(
            classify_login_path("/auth_error"),
            Some(LoginRedirect::AuthError)
        );
        assert_eq!(
            classify_login_path("/users/login"),
            Some(LoginRedirect::LoginPage)
        );
        assert_eq!(
            classify_login_path("/users/someone/change_password"),
            Some(LoginRedirect::ChangePassword)
        );
        // Usernames that merely contain the magic words are users, not pages.
        assert_eq!(
            classify_login_path("/users/change_password_fan"),
            Some(LoginRedirect::User("change_password_fan".into()))
        );
        assert_eq!(
            classify_login_path("/users/auth_error"),
            Some(LoginRedirect::User("auth_error".into()))
        );
        assert_eq!(classify_login_path("/users/"), None);
        assert_eq!(classify_login_path("/somewhere/new"), None);
    }

    #[test]
    fn redirect_paths_are_same_origin_or_root_relative_only() {
        let base = "https://archiveofourown.org";
        assert_eq!(
            redirect_path("https://archiveofourown.org/users/x", base).as_deref(),
            Some("/users/x")
        );
        assert_eq!(redirect_path("/users/x", base).as_deref(), Some("/users/x"));
        assert_eq!(
            redirect_path("/users/x?y=1#z", base).as_deref(),
            Some("/users/x")
        );
        assert_eq!(redirect_path("https://evil.example/users/x", base), None);
        assert_eq!(redirect_path("//evil.example/users/x", base), None);
        assert_eq!(redirect_path("users/x", base), None);
        assert_eq!(redirect_path("/", base).as_deref(), Some("/"));
        // A root redirect is a valid path but not a login-redirect class.
        assert_eq!(classify_login_path("/"), None);
    }
}
