//! Account session state machine.
//!
//! Owns the one thing the rest of the client trusts: whether we may talk to AO3
//! as a logged-in user, and with which cookie jar. States and transitions follow
//! `docs/write-client-plan.md` §2 and `docs/adr/0001-auth-model.md`.
//!
//! Invariants, each of which a reviewer has already tried to break:
//! 1. `Active` requires an authenticated read **and** a CSRF token. A verified
//!    jar without a token is `Imported`, which is not usable.
//! 2. A rejected or incomplete jar is never installed, and unused jars are wiped
//!    (leaving one in memory after declaring it unusable would let later code
//!    send it, and AO3 would "fix" it back into a live session).
//! 3. `Killed` is terminal for every network method until an operator re-enables.
//! 4. The CSRF token is in memory only, is dropped whenever the jar changes, and
//!    is a [`SecretString`] so it cannot be logged by accident.
//! 5. Login outcomes are classified from `Location`, not from "302 means happy":
//!    AO3's CSRF failure is a redirect, and a pwned-password login is redirected
//!    to a different page while already signed in.

use serde::Deserialize;

use crate::ao3::client::{Client, Response};
use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_decode, percent_encode};
use crate::ao3::pseuds::Pseud;
use crate::ao3::solver::{Solver, SolverStatus};
use crate::config::Config;
use crate::consts;
use crate::crypto::SecretString;
use crate::error::{Error, Result};
use crate::time::now_secs;

mod solver;

/// What the session currently is. See `docs/write-client-plan.md` §2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// A jar is held but the session is not proven usable (awaiting the first
    /// read, or validation failed ambiguously). Never lets a write through.
    Imported,
    /// Verified by an authenticated read and holding a CSRF token. The only
    /// state that may act.
    Active,
    /// Upstream redirected to the login page.
    LoggedOut,
    /// Upstream redirected to `/lost_cookie`: the jar is missing a cookie.
    SessionIncomplete,
    /// Upstream refused with a challenge/403.
    Blocked,
    /// An operator switched all upstream traffic off (terminal).
    Killed,
}

impl SessionState {
    /// Only `Active` may perform a write (ADR-0005).
    pub fn is_usable(self) -> bool {
        matches!(self, Self::Active)
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Imported => "imported",
            Self::Active => "active",
            Self::LoggedOut => "logged-out",
            Self::SessionIncomplete => "session-incomplete",
            Self::Blocked => "blocked",
            Self::Killed => "killed",
        };
        f.write_str(name)
    }
}

#[derive(Deserialize)]
struct TokenDispenser {
    token: String,
}

/// A logged-in AO3 identity plus the transport that carries it.
pub struct AccountSession {
    account: String,
    state: SessionState,
    csrf: Option<SecretString>,
    pseud: Option<Pseud>,
    client: Client,
    solver: Solver,
    solver_status: SolverStatus,
    /// Operator gate: the jar may only reach the solver when this is true.
    solver_send_cookies: bool,
}

impl AccountSession {
    pub fn new(config: &Config) -> Result<Self> {
        let client = Client::with_spacing(
            &config.base_url,
            &config.user_agent,
            config.min_request_spacing,
        )?;
        let solver = Solver::new(&config.solver)?;
        let solver_status = SolverStatus {
            mode: config.solver.mode,
            reachable: false,
            version: None,
            checked_at: now_secs(),
        };
        Ok(Self {
            account: String::new(),
            state: SessionState::LoggedOut,
            csrf: None,
            pseud: None,
            client,
            solver,
            solver_status,
            solver_send_cookies: config.solver.send_cookies,
        })
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn jar(&self) -> &Jar {
        self.client.jar()
    }

    /// The pseud used for comments, once discovered for this session.
    pub fn pseud(&self) -> Option<&Pseud> {
        self.pseud.as_ref()
    }

    pub fn set_pseud(&mut self, pseud: Pseud) {
        self.pseud = Some(pseud);
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut Client {
        &mut self.client
    }

    /// Restores a persisted jar without touching the network.
    ///
    /// The jar is only proven by the first real read (or an explicit
    /// [`AccountSession::probe`]), so the state is `Imported` — never `Active`.
    pub fn restore(&mut self, account: &str, jar: Jar) -> Result<SessionState> {
        self.ensure_alive()?;
        let account = validate_account(account)?;
        self.account = account.to_string();
        self.csrf = None;
        self.pseud = None;
        if !jar.is_complete() {
            self.client.set_jar(Jar::new());
            self.state = SessionState::SessionIncomplete;
            return Ok(self.state);
        }
        self.client.set_jar(jar);
        self.state = SessionState::Imported;
        Ok(self.state)
    }

    /// The masked CSRF token for mutating requests, if the session has one.
    pub fn csrf_token(&self) -> Result<&str> {
        self.csrf
            .as_ref()
            .map(SecretString::expose)
            .ok_or(Error::CsrfMissing)
    }

    /// Adopts a cookie jar supplied by the user and proves it works.
    ///
    /// An incomplete jar is rejected **before installation** and without network
    /// traffic: the two required cookies are a local fact, and installing then
    /// probing would let AO3 append `user_credentials` and revive it.
    pub async fn import(&mut self, account: &str, jar: Jar) -> Result<SessionState> {
        self.ensure_alive()?;
        let account = validate_account(account)?;
        self.account = account.to_string();
        self.csrf = None;

        if !jar.is_complete() {
            self.client.set_jar(Jar::new());
            self.state = SessionState::SessionIncomplete;
            return Ok(self.state);
        }
        self.client.set_jar(jar);
        self.state = SessionState::Imported;
        self.finish_validation().await
    }

    /// Logs in with an AO3 username/password (ADR-0001 option B).
    pub async fn login(&mut self, account: &str, password: SecretString) -> Result<SessionState> {
        self.ensure_alive()?;
        let account = validate_account(account)?;
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
        self.classify_pre_login(&login_page)?;
        let token = parse_token_from_page(&login_page.body)?;

        let response = match self
            .upstream_post_form(
                consts::LOGIN_PATH,
                &[
                    ("user[login]", account),
                    ("user[password]", password.expose()),
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

    /// Proves the session with an authenticated read.
    ///
    /// The return value is the **proof** from this read: `Active` means the
    /// read authenticated. [`SessionState::is_usable`] must still be read from
    /// [`AccountSession::state`], which stays `Imported` when no CSRF token is
    /// held — a proof alone is not permission to act.
    pub async fn probe(&mut self) -> Result<SessionState> {
        self.ensure_alive()?;
        // A dead or unusable state must not be "probed" back to life.
        if self.state == SessionState::SessionIncomplete {
            return Err(Error::SessionIncomplete);
        }
        if self.state == SessionState::LoggedOut {
            return Err(Error::LoggedOut);
        }
        let account = validate_account(&self.account)?;
        let path = format!(
            "/users/{}/subscriptions",
            percent_encode(account, SpaceMode::Percent20)
        );

        let before = self.jar_signature();
        let response = match self.upstream_get(&path, None).await {
            Ok(response) => response,
            Err(err) => {
                // Not proven either way: keep the jar, but stop claiming Active.
                self.state = SessionState::Imported;
                self.csrf = None;
                return Err(err);
            }
        };
        if before != self.jar_signature() {
            self.csrf = None;
        }

        let state = match response.status {
            200 => SessionState::Active,
            302 | 303 => {
                if response.location_contains("/lost_cookie") {
                    SessionState::SessionIncomplete
                } else if response.location_contains("/users/login") {
                    SessionState::LoggedOut
                } else {
                    // Unknown redirect: fail closed, do not guess.
                    self.state = SessionState::Imported;
                    self.csrf = None;
                    return Err(Error::UpstreamChanged(format!(
                        "probe redirected to {:?}",
                        response.location
                    )));
                }
            }
            429 => {
                return Err(Error::RateLimited {
                    retry_after: response.retry_after,
                });
            }
            403 | 503 => {
                self.state = SessionState::Blocked;
                self.csrf = None;
                return Err(Error::BlockedByEdge {
                    status: response.status,
                });
            }
            status => {
                self.state = SessionState::Imported;
                self.csrf = None;
                return Err(Error::UpstreamChanged(format!(
                    "probe returned HTTP {status}"
                )));
            }
        };

        match state {
            SessionState::Active => {}
            SessionState::SessionIncomplete | SessionState::LoggedOut => {
                // A dead or unusable jar is wiped, never kept around.
                self.client.set_jar(Jar::new());
                self.csrf = None;
            }
            SessionState::Imported | SessionState::Blocked | SessionState::Killed => {}
        }
        self.state = state;
        // A verified jar without a token is `Imported`, never `Active`.
        if self.state == SessionState::Active && self.csrf.is_none() {
            self.state = SessionState::Imported;
        }
        Ok(state)
    }

    /// Ends the upstream session and wipes local material.
    ///
    /// The DELETE carries the CSRF token; without it AO3 answers
    /// `/auth_error` while looking like a redirect. Local state is wiped even
    /// when the upstream call fails: being unable to reach AO3 must not leave a
    /// usable jar behind.
    pub async fn logout(&mut self) -> Result<SessionState> {
        self.ensure_alive()?;
        if self.account.is_empty() {
            self.wipe();
            return Ok(self.state);
        }

        if self.csrf.is_none() {
            // Best effort: a fresh token makes the delete valid.
            drop(self.fetch_csrf().await);
        }
        let mut attempt = self.delete_logout().await;
        if matches!(attempt, Err(Error::CsrfExpired)) {
            // The token was stale; refresh and try exactly once more. Signing out
            // is idempotent, so one retry cannot double-post anything.
            self.csrf = None;
            drop(self.fetch_csrf().await);
            attempt = self.delete_logout().await;
            if matches!(attempt, Err(Error::CsrfExpired)) {
                attempt = Err(Error::LogoutFailed(
                    "csrf token rejected after refresh".into(),
                ));
            }
        }

        self.wipe();
        attempt.map(|()| self.state)
    }

    /// Removes the local account, cookie jar, CSRF token and pseud without
    /// contacting AO3.
    pub fn clear_local(&mut self) {
        self.account.clear();
        self.wipe();
    }

    async fn delete_logout(&mut self) -> Result<()> {
        let Some(token) = self.csrf.as_ref().map(|token| token.expose().to_string()) else {
            return Err(Error::LogoutFailed("no csrf token available".into()));
        };
        match self
            .upstream_delete_with_csrf(consts::LOGOUT_PATH, &token)
            .await
        {
            Ok(response) if response.location_contains("/auth_error") => Err(Error::CsrfExpired),
            Ok(response) if response.is_redirect() || response.status == 200 => Ok(()),
            Ok(response) => Err(Error::LogoutFailed(format!("HTTP {}", response.status))),
            Err(err) => Err(Error::LogoutFailed(err.to_string())),
        }
    }

    /// Operator kill switch: stop all upstream traffic until re-enabled.
    pub fn kill(&mut self) {
        self.wipe();
        self.state = SessionState::Killed;
    }

    async fn finish_validation(&mut self) -> Result<SessionState> {
        let state = self.probe().await?;
        if state != SessionState::Active {
            return Ok(state);
        }
        match self.fetch_csrf().await {
            Ok(()) => {
                self.state = SessionState::Active;
                Ok(SessionState::Active)
            }
            Err(err) => {
                // A verified jar without a token is `Imported`: not usable.
                if matches!(err, Error::LoggedOut) {
                    self.client.set_jar(Jar::new());
                    self.state = SessionState::LoggedOut;
                } else if matches!(err, Error::SessionIncomplete) {
                    self.client.set_jar(Jar::new());
                    self.state = SessionState::SessionIncomplete;
                } else if matches!(err, Error::BlockedByEdge { .. }) {
                    self.state = SessionState::Blocked;
                } else if matches!(err, Error::Killed) {
                    self.state = SessionState::Killed;
                } else {
                    self.state = SessionState::Imported;
                }
                self.csrf = None;
                Err(err)
            }
        }
    }

    /// CSRF-protected form POST for a write action.
    ///
    /// The caller classifies the response (201/200/422/302); this only guarantees
    /// the token is present and sent.
    pub async fn post_write(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let token = self.csrf_token()?.to_string();
        let mut all: Vec<(&str, &str)> = fields.to_vec();
        all.push(("authenticity_token", token.as_str()));
        self.upstream_post_form(path, &all).await
    }

    /// CSRF-protected PATCH for a write action (reading flags).
    pub async fn patch_write(&mut self, path: &str, fields: &[(&str, &str)]) -> Result<Response> {
        let token = self.csrf_token()?.to_string();
        let mut all: Vec<(&str, &str)> = fields.to_vec();
        all.push(("authenticity_token", token.as_str()));
        self.upstream_patch_form(path, &all).await
    }

    /// CSRF-protected DELETE for a write action (e.g. unsubscribe).
    pub async fn delete_write(&mut self, path: &str) -> Result<Response> {
        let token = self.csrf_token()?.to_string();
        self.upstream_delete_with_csrf(path, &token).await
    }

    /// Fetches a fresh CSRF token from `/token_dispenser.json`.
    async fn fetch_csrf(&mut self) -> Result<()> {
        let before = self.jar_signature();
        let response = self
            .upstream_get(consts::TOKEN_PATH, Some(consts::TOKEN_TIMEOUT))
            .await?;
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
            429 => Err(Error::RateLimited {
                retry_after: response.retry_after,
            }),
            403 | 503 => Err(Error::BlockedByEdge {
                status: response.status,
            }),
            status => Err(Error::UpstreamChanged(format!(
                "token dispenser returned HTTP {status}"
            ))),
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

    fn jar_signature(&self) -> Option<String> {
        self.client.jar().cookie_header()
    }

    fn ensure_alive(&self) -> Result<()> {
        if self.state == SessionState::Killed {
            return Err(Error::Killed);
        }
        Ok(())
    }

    /// Local wipe: jar, token, pseud, state.
    fn wipe(&mut self) {
        self.client.set_jar(Jar::new());
        self.csrf = None;
        self.pseud = None;
        self.state = SessionState::LoggedOut;
    }
}

fn validate_account(account: &str) -> Result<&str> {
    let trimmed = account.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("AO3 account login is empty".into()));
    }
    Ok(trimmed)
}

/// Recovers the canonical login from a success redirect such as
/// `/users/some_one` or `https://archiveofourown.org/users/some_one`.
///
/// The login typed by the user may be an email, so the redirect is the only
/// reliable source of the canonical username for the probe path.
/// Extracts the path of a same-origin or root-relative redirect.
///
/// Rejects absolute URLs to other origins and protocol-relative references
/// (`//host/path`), which would otherwise be misread as root-relative.
fn redirect_path(location: &str, base: &str) -> Option<String> {
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

fn parse_token_from_page(html: &str) -> Result<String> {
    crate::ao3::parse::csrf_token_from_meta(html)
        .map(str::to_string)
        .ok_or(Error::CsrfMissing)
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
