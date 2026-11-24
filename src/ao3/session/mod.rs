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

mod csrf;
mod login;
mod solver;

use crate::ao3::client::Client;
use crate::ao3::jar::Jar;
use crate::ao3::parse::{SpaceMode, percent_encode};
use crate::ao3::pseuds::Pseud;
use crate::ao3::solver::{Solver, SolverStatus};
use crate::config::Config;
use crate::consts;
use crate::crypto::SecretString;
use crate::error::{Error, Result};
use crate::time::now_secs;

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

    /// Proves a restored (`Imported`) or transiently `Blocked` session and
    /// fetches a fresh CSRF token so it transitions to `Active`. No-op when
    /// already `Active` with a token.
    pub async fn ensure_active(&mut self) -> Result<SessionState> {
        self.ensure_alive()?;
        if self.state == SessionState::Active && self.csrf.is_some() {
            return Ok(SessionState::Active);
        }
        if matches!(self.state, SessionState::Imported | SessionState::Blocked)
            && self.jar().is_complete()
            && !self.account.is_empty()
        {
            return self.finish_validation().await;
        }
        Ok(self.state)
    }

    /// Re-verifies the current session and refreshes its CSRF token so
    /// "Verify session now" can promote a restored (`Imported`) session to
    /// `Active`.
    pub async fn verify_and_refresh(&mut self) -> Result<SessionState> {
        self.ensure_alive()?;
        self.finish_validation().await
    }

    /// Detects the signed-in AO3 login name for an imported cookie jar by
    /// fetching `/` with that jar and inspecting the `#greeting` header.
    /// Restores the previous session state before returning.
    pub async fn discover_account_from_jar(&mut self, jar: &Jar) -> Result<String> {
        self.ensure_alive()?;
        if !jar.is_complete() {
            return Err(Error::SessionIncomplete);
        }
        let saved_jar = self.client.jar().clone();
        let saved_state = self.state;
        let saved_csrf = self
            .csrf
            .as_ref()
            .map(|token| SecretString::new(token.expose()));
        let saved_account = self.account.clone();

        self.client.set_jar(jar.clone());
        self.state = SessionState::Imported;
        self.csrf = None;

        let result = self.upstream_get("/", None).await;

        self.client.set_jar(saved_jar);
        self.state = saved_state;
        self.csrf = saved_csrf;
        self.account = saved_account;

        let response = result?;
        if response.status != 200 {
            return Err(Error::Validation(format!(
                "could not detect AO3 login from cookie file (HTTP {})",
                response.status
            )));
        }
        crate::ao3::parse::logged_in_account_from_html(&response.body).ok_or_else(|| {
            Error::Validation(
                "could not detect AO3 username from cookie file — enter your AO3 login manually"
                    .into(),
            )
        })
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
            200 => {
                if let Some(token) = crate::ao3::parse::csrf_token_from_html(&response.body) {
                    self.csrf = Some(SecretString::new(token));
                }
                SessionState::Active
            }
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

    pub(super) async fn finish_validation(&mut self) -> Result<SessionState> {
        let state = self.probe().await?;
        if state != SessionState::Active {
            return Ok(state);
        }
        let probed_csrf = self.csrf.as_ref().map(|token| token.expose().to_string());
        match self.fetch_csrf().await {
            Ok(()) => {
                self.state = SessionState::Active;
                Ok(SessionState::Active)
            }
            Err(err) => {
                // If `/token_dispenser.json` hit a transient edge/rate-limit
                // error, keep the authenticated CSRF meta token we already
                // captured from the probe page.
                if let Some(token) = probed_csrf
                    && matches!(
                        err,
                        Error::RateLimited { .. }
                            | Error::BlockedByEdge { .. }
                            | Error::Network(_)
                            | Error::SolverFailed(_)
                            | Error::SolverUnavailable(_)
                    )
                {
                    self.csrf = Some(SecretString::new(token));
                    self.state = SessionState::Active;
                    return Ok(SessionState::Active);
                }
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

    pub(super) fn jar_signature(&self) -> Option<String> {
        self.client.jar().cookie_header()
    }

    pub(super) fn ensure_alive(&self) -> Result<()> {
        if self.state == SessionState::Killed {
            return Err(Error::Killed);
        }
        Ok(())
    }

    /// Local wipe: jar, token, pseud, state.
    pub(super) fn wipe(&mut self) {
        self.client.set_jar(Jar::new());
        self.csrf = None;
        self.pseud = None;
        self.state = SessionState::LoggedOut;
    }
}

pub(super) fn validate_account(account: &str) -> Result<&str> {
    let trimmed = account.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("AO3 account login is empty".into()));
    }
    Ok(trimmed)
}
