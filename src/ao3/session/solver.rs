//! Solver integration on the account session: explicit solves, cookie merging
//! and the scoped Cloudflare clearance window.
//!
//! Read-only by construction: these methods only ask the host-provided
//! FlareSolverr for one `GET` at a time. Writes keep the direct client.

use super::{AccountSession, Jar, Response, SessionState};
use crate::ao3::solver::{SolveOutcome, SolverStatus};
use crate::config::SolverMode;
use crate::consts;
use crate::diag::{Transport, sanitize_location, sanitize_path};
use crate::error::{Error, Result};
use crate::time::now_secs;

impl AccountSession {
    pub fn solver_mode(&self) -> SolverMode {
        self.solver.mode()
    }

    /// Operator allow-list for handing the AO3 jar to the local browser.
    pub fn solver_may_receive_cookies(&self) -> bool {
        self.solver_send_cookies
    }

    /// Liveness probe; never touches AO3 and never fatal.
    pub async fn probe_solver(&self) -> SolverStatus {
        self.solver.probe().await
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

    /// Solves exactly one user-requested read through the host-provided solver.
    ///
    /// Read-only by construction: the method only ever asks FlareSolverr for a
    /// `GET`, never touches a write path, and returns a [`Response`] shaped like
    /// a direct one so the existing parser/cache code is reused.
    pub async fn solve_get(&mut self, path: &str, send_cookies: bool) -> Result<Response> {
        if self.state == SessionState::Killed {
            return Err(Error::Killed);
        }
        let url = format!("{}{}", self.client.base_url(), path);
        let sent_cookies = send_cookies && self.solver_send_cookies;
        let cookies = if sent_cookies {
            jar_cookie_pairs(self.client.jar())
        } else {
            Vec::new()
        };
        let outcome = self.solver.solve_get(&url, &cookies).await?;

        let final_path = sanitize_location(Some(&outcome.solution_url)).unwrap_or_default();
        if !path.starts_with(consts::LOGIN_PATH) && final_path.starts_with("/users/login") {
            return Err(Error::LoggedOut);
        }
        if final_path.starts_with("/lost_cookie") {
            return Err(Error::SessionIncomplete);
        }
        if let Some(error) = status_error(outcome.status) {
            return Err(error);
        }

        let jar_changed = self.merge_solver_cookies(&outcome, sent_cookies);
        Ok(Response {
            status: outcome.status,
            location: None,
            retry_after: None,
            body: outcome.html,
            set_cookies: Vec::new(),
            // Only signal jar rotation when AO3 cookies were merged back.
            jar_changed,
            method: "GET".to_string(),
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
            let cookie_path = cookie.path.as_deref().unwrap_or("/");
            let mut header = format!(
                "{}={}; Path={cookie_path}",
                cookie.name,
                cookie.value.expose()
            );
            if let Some(domain) = &cookie.domain {
                header.push_str(&format!("; Domain={domain}"));
            }
            if merge_jar {
                self.client.jar_mut().apply(&header);
            }
            if cookie.name == "cf_clearance" || cookie.name == "__cf_bm" {
                clearance_parts.push(format!("{}={}", cookie.name, cookie.value.expose()));
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
        merge_jar
    }
}

/// Cookie pairs worth handing to a browser that may need the signed-in view.
fn jar_cookie_pairs(jar: &Jar) -> Vec<(String, String)> {
    ["_otwarchive_session", "user_credentials", "view_adult"]
        .into_iter()
        .filter_map(|name| {
            jar.get(name)
                .map(|value| (name.to_string(), value.to_string()))
        })
        .collect()
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
