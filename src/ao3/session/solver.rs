//! Solver integration on the account session: unified transport routing,
//! cookie merging, redirect synthesis, and the scoped Cloudflare clearance
//! window.
//!
//! Whenever the host-provided FlareSolverr is reachable (and not disabled via
//! `AO3I_FLARESOLVERR=off`), every AO3 interaction (`GET`, `POST`, `PATCH`,
//! `DELETE`) goes through it, falling back to the direct client only when the
//! solver is absent or unreachable.

use std::time::Duration;

use super::login::redirect_path;
use super::{AccountSession, Jar, SessionState};
use crate::ao3::client::{Response, classify_status, form_urlencode};
use crate::ao3::solver::{SolveOutcome, SolverStatus};
use crate::config::SolverMode;
use crate::consts;
use crate::diag::{HttpDirection, HttpErrorEvent, Transport, sanitize_path};
use crate::error::{Error, Result};
use crate::time::now_secs;

/// How long to wait before re-probing an unreachable local solver.
const SOLVER_REPROBE_INTERVAL_SECS: i64 = 30;

impl AccountSession {
    pub fn solver_mode(&self) -> SolverMode {
        self.solver.mode()
    }

    pub fn solver_status(&self) -> &SolverStatus {
        &self.solver_status
    }

    /// Operator allow-list for handing the AO3 jar to the local browser.
    pub fn solver_may_receive_cookies(&self) -> bool {
        self.solver_send_cookies
    }

    /// Liveness probe; never touches AO3 and never fatal. Updates the cached
    /// status on the session.
    pub async fn probe_solver(&mut self) -> SolverStatus {
        let status = self.solver.probe().await;
        self.solver_status = status.clone();
        status
    }

    /// True when the local solver is enabled and reachable. Re-probes when a
    /// previous failure is older than [`SOLVER_REPROBE_INTERVAL_SECS`].
    pub async fn solver_available(&mut self) -> bool {
        if self.solver.mode() == SolverMode::Off {
            return false;
        }
        if self.solver_status.reachable {
            return true;
        }
        if now_secs().saturating_sub(self.solver_status.checked_at) < SOLVER_REPROBE_INTERVAL_SECS {
            return false;
        }
        self.probe_solver().await.reachable
    }

    /// The active clearance, for encrypted persistence. Never log the cookie.
    pub fn clearance_snapshot(&self) -> Option<(String, String, i64)> {
        self.client.clearance().map(|clearance| {
            (
                clearance.user_agent.clone(),
                clearance.cookie_header.expose().to_string(),
                clearance.expires_at,
            )
        })
    }

    /// Restores a persisted clearance when it is still inside its window.
    pub fn restore_clearance(&mut self, user_agent: &str, cookie_header: &str, expires_at: i64) {
        if expires_at > now_secs() && !user_agent.is_empty() && !cookie_header.is_empty() {
            self.client
                .set_clearance(user_agent, cookie_header, expires_at);
        }
    }

    pub fn clear_clearance(&mut self) {
        self.client.clear_clearance();
    }

    /// Destroys any persistent FlareSolverr browser session (e.g. on logout,
    /// account switch, or a login CSRF mismatch).
    pub async fn reset_solver_session(&self) {
        self.solver.reset_session().await;
    }

    /// Runs an upstream `GET` through FlareSolverr when available, falling back
    /// to the direct HTTP client only when the solver is absent or unreachable.
    pub async fn upstream_get(
        &mut self,
        path: &str,
        timeout: Option<Duration>,
    ) -> Result<Response> {
        if self.solver_available().await {
            match self.solve_request("GET", path, None, true).await {
                Ok(response) => return Ok(response),
                Err(Error::SolverUnavailable(_)) => {}
                Err(error) => return Err(error),
            }
        }
        match timeout {
            Some(budget) => self.client.get_with_timeout(path, budget).await,
            None => self.client.get(path).await,
        }
    }

    /// Runs an upstream form `POST` through FlareSolverr when available,
    /// falling back to the direct HTTP client if the solver is unreachable or
    /// if FlareSolverr's synthetic form POST triggers a Rails `/auth_error`
    /// (`Origin: null` / `postData` encoding quirk).
    pub async fn upstream_post_form(
        &mut self,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<Response> {
        if self.solver_available().await {
            let body = form_urlencode(fields);
            match self.solve_request("POST", path, Some(&body), true).await {
                Ok(response)
                    if response.is_redirect() && response.location_contains("/auth_error") =>
                {
                    if let Ok(direct) = self.client.post_form(path, fields).await
                        && !matches!(direct.status, 403 | 503)
                    {
                        return Ok(direct);
                    }
                    return Ok(response);
                }
                Ok(response) => return Ok(response),
                Err(Error::SolverUnavailable(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.client.post_form(path, fields).await
    }

    /// Runs an upstream form `PATCH` through FlareSolverr (using Rails'
    /// `_method=patch` override) when available, falling back to direct `PATCH`
    /// if unreachable or if the browser form POST triggers `/auth_error`.
    pub async fn upstream_patch_form(
        &mut self,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<Response> {
        if self.solver_available().await {
            let mut all = Vec::with_capacity(fields.len() + 1);
            all.push(("_method", "patch"));
            all.extend_from_slice(fields);
            let body = form_urlencode(&all);
            match self.solve_request("PATCH", path, Some(&body), true).await {
                Ok(response)
                    if response.is_redirect() && response.location_contains("/auth_error") =>
                {
                    if let Ok(direct) = self.client.patch_form(path, fields).await
                        && !matches!(direct.status, 403 | 503)
                    {
                        return Ok(direct);
                    }
                    return Ok(response);
                }
                Ok(response) => return Ok(response),
                Err(Error::SolverUnavailable(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.client.patch_form(path, fields).await
    }

    /// Runs an upstream form `PUT` through FlareSolverr (using Rails'
    /// `_method=put` override) when available, falling back to direct `PUT` if
    /// unreachable or if the browser form POST triggers `/auth_error`.
    pub async fn upstream_put_form(
        &mut self,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<Response> {
        if self.solver_available().await {
            let mut all = Vec::with_capacity(fields.len() + 1);
            all.push(("_method", "put"));
            all.extend_from_slice(fields);
            let body = form_urlencode(&all);
            match self.solve_request("PUT", path, Some(&body), true).await {
                Ok(response)
                    if response.is_redirect() && response.location_contains("/auth_error") =>
                {
                    if let Ok(direct) = self.client.put_form(path, fields).await
                        && !matches!(direct.status, 403 | 503)
                    {
                        return Ok(direct);
                    }
                    return Ok(response);
                }
                Ok(response) => return Ok(response),
                Err(Error::SolverUnavailable(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.client.put_form(path, fields).await
    }

    /// Runs a CSRF-protected `DELETE` through FlareSolverr (using Rails'
    /// `_method=delete` override) when available, falling back to direct
    /// `DELETE` if unreachable or if the browser form POST triggers `/auth_error`.
    pub async fn upstream_delete_with_csrf(&mut self, path: &str, token: &str) -> Result<Response> {
        if self.solver_available().await {
            let body = form_urlencode(&[("_method", "delete"), ("authenticity_token", token)]);
            match self.solve_request("DELETE", path, Some(&body), true).await {
                Ok(response)
                    if response.is_redirect() && response.location_contains("/auth_error") =>
                {
                    if let Ok(direct) = self.client.delete_with_csrf(path, token).await
                        && !matches!(direct.status, 403 | 503)
                    {
                        return Ok(direct);
                    }
                    return Ok(response);
                }
                Ok(response) => return Ok(response),
                Err(Error::SolverUnavailable(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.client.delete_with_csrf(path, token).await
    }

    /// Solves one `GET` through the host-provided solver and maps auth redirects
    /// and edge/rate-limit statuses onto typed errors.
    pub async fn solve_get(&mut self, path: &str, send_cookies: bool) -> Result<Response> {
        let response = self.solve_request("GET", path, None, send_cookies).await?;
        if !path.starts_with(consts::LOGIN_PATH) && response.location_contains("/users/login") {
            return Err(Error::LoggedOut);
        }
        if response.location_contains("/lost_cookie") {
            return Err(Error::SessionIncomplete);
        }
        if let Some(error) = status_error(response.status) {
            return Err(error);
        }
        Ok(response)
    }

    /// Low-level solver transport for `GET` and `POST` (`PATCH`/`DELETE` via
    /// Rails `_method`). Synthesizes redirect `302` + `Location` from the
    /// browser's final URL so callers classify responses identically to direct
    /// requests.
    async fn solve_request(
        &mut self,
        method: &str,
        path: &str,
        post_data: Option<&str>,
        send_cookies: bool,
    ) -> Result<Response> {
        if self.state == SessionState::Killed {
            return Err(Error::Killed);
        }
        let url = format!("{}{}", self.client.base_url(), path);
        // Login POST always carries the pre-login `_otwarchive_session` cookie
        // minted by `GET /users/login` so Rails accepts its CSRF token.
        let is_login_post = post_data.is_some() && path.starts_with(consts::LOGIN_PATH);
        let sent_cookies = (send_cookies && self.solver_send_cookies) || is_login_post;
        let cookies = if sent_cookies {
            jar_cookie_pairs(self.client.jar())
        } else {
            Vec::new()
        };

        let result = match post_data {
            Some(data) => self.solver.solve_post(&url, data, &cookies).await,
            None => self.solver.solve_get(&url, &cookies).await,
        };
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                if matches!(error, Error::SolverUnavailable(_)) {
                    self.solver_status.reachable = false;
                    self.solver_status.checked_at = now_secs();
                }
                self.client.report(HttpErrorEvent::new(
                    HttpDirection::Upstream,
                    Transport::Solver,
                    method,
                    path,
                    solver_error_kind(&error),
                    error.to_string(),
                ));
                return Err(error);
            }
        };

        let jar_changed = self.merge_solver_cookies(&outcome, sent_cookies);
        let (status, location) = classify_solver_landing(
            path,
            post_data.is_some(),
            outcome.status,
            &outcome.solution_url,
            self.client.base_url(),
        );
        if jar_changed {
            self.csrf = None;
        }
        if sent_cookies
            && status == 200
            && let Some(token) = crate::ao3::parse::csrf_token_from_html(&outcome.html)
        {
            self.csrf = Some(crate::crypto::SecretString::new(token));
        }
        if status >= 400 {
            self.client.report(
                HttpErrorEvent::new(
                    HttpDirection::Upstream,
                    Transport::Solver,
                    method,
                    path,
                    classify_status(status),
                    format!("upstream answered HTTP {status} via solver"),
                )
                .with_status(status)
                .with_latency(outcome.latency_ms),
            );
        }

        Ok(Response {
            status,
            location,
            retry_after: None,
            body: outcome.html,
            set_cookies: Vec::new(),
            jar_changed,
            method: method.to_string(),
            path: sanitize_path(path),
            latency_ms: outcome.latency_ms,
            server_header: None,
            cf_ray: None,
            content_type: Some("text/html".to_string()),
            transport: Transport::Solver,
        })
    }

    /// Captures Cloudflare clearance cookies into the scoped browser-UA window,
    /// and merges AO3 cookies back into the jar only when the solver was handed
    /// the session (or no complete session jar is installed yet).
    fn merge_solver_cookies(&mut self, outcome: &SolveOutcome, sent_cookies: bool) -> bool {
        let host = base_host(self.client.base_url());
        let merge_jar = sent_cookies || !self.client.jar().is_complete();
        let before = self.client.jar().cookie_header();
        let mut clearance_parts: Vec<String> = Vec::new();
        for cookie in &outcome.cookies {
            let domain_ok = cookie
                .domain
                .as_deref()
                .map(|domain| domain_matches(domain, &host))
                .unwrap_or(true);
            if !domain_ok {
                continue;
            }
            if cookie.name == "cf_clearance" || cookie.name == "__cf_bm" {
                clearance_parts.push(format!("{}={}", cookie.name, cookie.value.expose()));
                continue;
            }
            if merge_jar {
                let cookie_path = cookie.path.as_deref().unwrap_or("/");
                let mut header = format!(
                    "{}={}; Path={cookie_path}",
                    cookie.name,
                    cookie.value.expose()
                );
                if let Some(domain) = &cookie.domain {
                    header.push_str(&format!("; Domain={domain}"));
                }
                self.client.jar_mut().apply(&header);
            }
        }

        if !clearance_parts.is_empty() && !outcome.user_agent.is_empty() {
            let ttl = i64::try_from(consts::FLARESOLVERR_CLEARANCE_TTL.as_secs()).unwrap_or(0);
            self.client.set_clearance(
                &outcome.user_agent,
                &clearance_parts.join("; "),
                now_secs().saturating_add(ttl),
            );
        }
        before != self.client.jar().cookie_header()
    }
}

/// Synthesizes a `302` + `Location` when Chromium followed a redirect so
/// downstream classifiers see the same shape as direct no-redirect requests.
fn classify_solver_landing(
    requested: &str,
    is_mutation: bool,
    status: u16,
    solution_url: &str,
    base_url: &str,
) -> (u16, Option<String>) {
    if status != 200 || solution_url.is_empty() {
        return (status, None);
    }
    let requested_path = requested.split(['?', '#']).next().unwrap_or(requested);
    let Some(final_path) = redirect_path(solution_url, base_url) else {
        // Off-origin redirect after a mutation (e.g. login) should still surface
        // as a redirect so the session layer rejects it fail-closed.
        if is_mutation {
            return (302, Some(solution_url.to_string()));
        }
        return (status, None);
    };
    let auth_redirect = (final_path.starts_with("/users/login")
        && !requested_path.starts_with("/users/login"))
        || final_path.starts_with("/lost_cookie")
        || final_path.starts_with("/auth_error");
    let redirected = if is_mutation {
        final_path != requested_path || auth_redirect
    } else {
        auth_redirect
    };
    if redirected {
        (302, Some(solution_url.to_string()))
    } else {
        (status, None)
    }
}

fn solver_error_kind(error: &Error) -> &'static str {
    match error {
        Error::SolverCaptcha => "solver_captcha",
        Error::SolverUnavailable(_) => "solver_unreachable",
        Error::SolverBodyCap => "solver_body_cap",
        Error::Config(_)
        | Error::Io(_)
        | Error::Store(_)
        | Error::Crypto(_)
        | Error::Network(_)
        | Error::LoginRejected(_)
        | Error::PasswordChangeRequired
        | Error::LoggedOut
        | Error::Validation(_)
        | Error::SpamRejected
        | Error::SessionIncomplete
        | Error::CsrfMissing
        | Error::CsrfExpired
        | Error::LogoutFailed(_)
        | Error::Killed
        | Error::RateLimited { .. }
        | Error::BlockedByEdge { .. }
        | Error::UpstreamChanged(_)
        | Error::SolverDisabled
        | Error::SolverFailed(_) => "solver_error",
    }
}

/// Cookie pairs worth handing to a browser that may need the signed-in view.
/// Includes `remember_user_token` and all AO3 session cookies so Devise keeps
/// the session alive across restarts and updates.
fn jar_cookie_pairs(jar: &Jar) -> Vec<(String, String)> {
    jar.all_pairs()
}

/// Maps a final browser status onto the same typed errors as direct reads.
fn status_error(status: u16) -> Option<Error> {
    match status {
        403 | 503 => Some(Error::BlockedByEdge { status }),
        429 => Some(Error::RateLimited { retry_after: None }),
        _ => None,
    }
}

/// Host part of the configured origin, lowercased, without a port.
fn base_host(base_url: &str) -> String {
    let rest = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    host.to_ascii_lowercase()
}

/// Cookie domains may be exact (`.archiveofourown.org`) or a parent domain.
fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solver_cookie_domains_match_ao3_only() {
        assert!(domain_matches(
            ".archiveofourown.org",
            "archiveofourown.org"
        ));
        assert!(domain_matches("archiveofourown.org", "archiveofourown.org"));
        assert!(domain_matches(
            ".archiveofourown.org",
            "www.archiveofourown.org"
        ));
        assert!(!domain_matches(".example.org", "archiveofourown.org"));
        assert!(!domain_matches(
            ".archiveofourown.org",
            "evilarchiveofourown.org"
        ));
        assert_eq!(base_host("http://127.0.0.1:3000"), "127.0.0.1");
        assert_eq!(
            base_host("https://archiveofourown.org"),
            "archiveofourown.org"
        );
    }

    #[test]
    fn solver_status_errors_match_the_direct_ones() {
        assert!(matches!(
            status_error(403),
            Some(Error::BlockedByEdge { status: 403 })
        ));
        assert!(matches!(
            status_error(503),
            Some(Error::BlockedByEdge { status: 503 })
        ));
        assert!(matches!(
            status_error(429),
            Some(Error::RateLimited { retry_after: None })
        ));
        assert!(status_error(200).is_none());
    }
}
