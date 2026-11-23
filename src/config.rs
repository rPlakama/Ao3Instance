//! Typed configuration.
//!
//! One struct, read once at startup. `std::env` is touched only by
//! [`Config::from_env`]; everything else takes the values it needs explicitly,
//! which keeps tests free of environment mutation.

use std::path::PathBuf;
use std::time::Duration;

use crate::consts;
use crate::error::{Error, Result};

/// The environment prefix for every knob we expose.
const PREFIX: &str = "AO3I_";

#[derive(Debug, Clone)]
pub struct Config {
    /// Upstream origin, without a trailing slash.
    pub base_url: String,
    /// Where keys and (later) the database live.
    pub data_dir: PathBuf,
    /// AES key file for secrets at rest.
    pub key_file: PathBuf,
    /// Request identity sent to AO3.
    pub user_agent: String,
    /// Master switch for anything that mutates upstream (ADR-0002/0005).
    pub write_enabled: bool,
    /// Client-side floor between upstream requests (ADR-0002 §4).
    pub min_request_spacing: Duration,
    /// Address the web app binds to.
    pub bind: String,
    /// Port the web app listens on.
    pub port: u16,
    /// SQLite database file.
    pub db_file: PathBuf,
    /// Emit structured HTTP error lines to stderr (journald-friendly).
    pub log_http_errors: bool,
    /// Maximum rows retained in `http_error_log`.
    pub http_log_retention: i64,
    /// Optional host-provided FlareSolverr fallback (Phase 2.5).
    pub solver: SolverConfig,
}

/// When the host-provided solver may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolverMode {
    /// Probe and offer the fallback when reachable; silent when absent.
    Auto,
    /// Same as `Auto`, but surface "configured but unreachable" in the UI.
    On,
    /// Never probe, never contact.
    Off,
}

impl std::fmt::Display for SolverMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        })
    }
}

/// Solver endpoint settings, validated at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverConfig {
    pub mode: SolverMode,
    /// Origin only, e.g. `http://127.0.0.1:8191`.
    pub origin: String,
    pub allow_remote: bool,
    pub send_cookies: bool,
    pub max_timeout_ms: u64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            mode: SolverMode::Auto,
            origin: consts::FLARESOLVERR_DEFAULT_URL.to_string(),
            allow_remote: false,
            send_cookies: true,
            max_timeout_ms: consts::FLARESOLVERR_MAX_TIMEOUT_MS,
        }
    }
}

impl Config {
    /// Reads `AO3I_*` from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_pairs(std::env::vars())
    }

    /// Pure constructor: takes the environment as pairs. Ordering-independent;
    /// later duplicate keys win, matching `std::env::vars` semantics.
    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut base_url = consts::BASE_URL.to_string();
        let mut data_dir: Option<PathBuf> = None;
        let mut key_file: Option<PathBuf> = None;
        let mut user_agent = consts::USER_AGENT.to_string();
        let mut write_enabled = false;
        let mut min_request_spacing = consts::MIN_REQUEST_SPACING;
        let mut bind = consts::DEFAULT_BIND.to_string();
        let mut port = consts::DEFAULT_PORT;
        let mut db_file: Option<PathBuf> = None;
        let mut log_http_errors = true;
        let mut http_log_retention = consts::HTTP_ERROR_LOG_RETENTION;
        let mut solver_mode = SolverMode::Auto;
        let mut solver_url = consts::FLARESOLVERR_DEFAULT_URL.to_string();
        let mut solver_allow_remote = false;
        let mut solver_send_cookies = true;
        let mut solver_max_timeout = consts::FLARESOLVERR_MAX_TIMEOUT_MS;
        let mut xdg_config_home: Option<PathBuf> = None;
        let mut home: Option<PathBuf> = None;

        for (key, value) in pairs {
            let key = key.into();
            let value = value.into();
            // The two variables that locate the data dir are read from the same
            // iterator, so `from_pairs` is genuinely pure (tests need no env).
            match key.as_str() {
                "XDG_CONFIG_HOME" => {
                    xdg_config_home = Some(PathBuf::from(value));
                    continue;
                }
                "HOME" => {
                    home = Some(PathBuf::from(value));
                    continue;
                }
                _ => {}
            }
            let Some(name) = key.strip_prefix(PREFIX) else {
                continue;
            };
            match name {
                "BASE_URL" => {
                    let trimmed = value.trim().trim_end_matches('/');
                    if trimmed.is_empty() {
                        return Err(Error::Config("AO3I_BASE_URL is empty".into()));
                    }
                    base_url = trimmed.to_string();
                }
                "DATA_DIR" => data_dir = Some(PathBuf::from(value)),
                "KEY_FILE" => key_file = Some(PathBuf::from(value)),
                "USER_AGENT" => user_agent = value,
                "BIND" => {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return Err(Error::Config("AO3I_BIND is empty".into()));
                    }
                    bind = trimmed.to_string();
                }
                "PORT" => {
                    port = value.trim().parse::<u16>().map_err(|_| {
                        Error::Config(format!("AO3I_PORT expects a port number, got {value:?}"))
                    })?;
                }
                "DB_FILE" => db_file = Some(PathBuf::from(value)),
                "WRITE_ENABLED" => write_enabled = parse_bool("AO3I_WRITE_ENABLED", &value)?,
                "LOG_HTTP_ERRORS" => {
                    log_http_errors = parse_bool("AO3I_LOG_HTTP_ERRORS", &value)?;
                }
                "FLARESOLVERR" => {
                    solver_mode = match value.trim().to_ascii_lowercase().as_str() {
                        "auto" | "" => SolverMode::Auto,
                        "on" => SolverMode::On,
                        "off" => SolverMode::Off,
                        other => {
                            return Err(Error::Config(format!(
                                "AO3I_FLARESOLVERR expects auto, on or off, got {other:?}"
                            )));
                        }
                    };
                }
                "FLARESOLVERR_URL" => solver_url = value,
                "FLARESOLVERR_ALLOW_REMOTE" => {
                    solver_allow_remote = parse_bool("AO3I_FLARESOLVERR_ALLOW_REMOTE", &value)?;
                }
                "FLARESOLVERR_SEND_COOKIES" => {
                    solver_send_cookies = parse_bool("AO3I_FLARESOLVERR_SEND_COOKIES", &value)?;
                }
                "FLARESOLVERR_MAX_TIMEOUT_MS" => {
                    let timeout: u64 = value.trim().parse().map_err(|_| {
                        Error::Config(format!(
                            "AO3I_FLARESOLVERR_MAX_TIMEOUT_MS expects milliseconds, got {value:?}"
                        ))
                    })?;
                    if !(5_000..=180_000).contains(&timeout) {
                        return Err(Error::Config(format!(
                            "AO3I_FLARESOLVERR_MAX_TIMEOUT_MS must be 5000..=180000, got {timeout}"
                        )));
                    }
                    solver_max_timeout = timeout;
                }
                "HTTP_LOG_RETENTION" => {
                    let retention: i64 = value.trim().parse().map_err(|_| {
                        Error::Config(format!(
                            "AO3I_HTTP_LOG_RETENTION expects a positive integer, got {value:?}"
                        ))
                    })?;
                    if retention <= 0 {
                        return Err(Error::Config(format!(
                            "AO3I_HTTP_LOG_RETENTION expects a positive integer, got {value:?}"
                        )));
                    }
                    http_log_retention = retention;
                }
                "MIN_SPACING_MS" => {
                    let millis: u64 = value.trim().parse().map_err(|_| {
                        Error::Config(format!(
                            "AO3I_MIN_SPACING_MS expects a number, got {value:?}"
                        ))
                    })?;
                    min_request_spacing = Duration::from_millis(millis);
                }
                // Unknown AO3I_* keys are ignored: forward compatibility.
                _ => {}
            }
        }

        let data_dir = match data_dir {
            Some(dir) => dir,
            None => default_data_dir(xdg_config_home.as_deref(), home.as_deref())?,
        };
        let key_file = key_file.unwrap_or_else(|| data_dir.join("key"));
        let db_file = db_file.unwrap_or_else(|| data_dir.join(consts::DB_FILE_NAME));

        let solver = SolverConfig {
            mode: solver_mode,
            origin: parse_solver_origin(&solver_url, solver_allow_remote)?,
            allow_remote: solver_allow_remote,
            send_cookies: solver_send_cookies,
            max_timeout_ms: solver_max_timeout,
        };

        Ok(Self {
            base_url,
            data_dir,
            key_file,
            user_agent,
            write_enabled,
            min_request_spacing,
            bind,
            port,
            db_file,
            log_http_errors,
            http_log_retention,
            solver,
        })
    }
}

/// Validates the solver origin: `http(s)://host[:port]`, no path or credentials,
/// and loopback-only unless the operator opted into a remote host.
fn parse_solver_origin(value: &str, allow_remote: bool) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err(Error::Config(format!(
            "AO3I_FLARESOLVERR_URL must be an http(s) origin, got {value:?}"
        )));
    };
    if !matches!(scheme, "http" | "https") {
        return Err(Error::Config(format!(
            "AO3I_FLARESOLVERR_URL must use http or https, got {scheme:?}"
        )));
    }
    if rest.is_empty() || rest.contains(['/', '?', '#', '@']) {
        return Err(Error::Config(format!(
            "AO3I_FLARESOLVERR_URL must be an origin without a path, got {value:?}"
        )));
    }

    let (host, port) = split_host_port(rest)?;
    if port == Some(0) {
        return Err(Error::Config(
            "AO3I_FLARESOLVERR_URL port must not be zero".into(),
        ));
    }
    if !allow_remote {
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if !loopback {
            return Err(Error::Config(format!(
                "AO3I_FLARESOLVERR_URL host {host:?} is not loopback; \
                 set AO3I_FLARESOLVERR_ALLOW_REMOTE=1 only if the service is \
                 reachable on a private address"
            )));
        }
    }

    Ok(format!("{scheme}://{rest}"))
}

/// Splits `host`, `host:port` or `[::1]:port` without a URL crate.
fn split_host_port(rest: &str) -> Result<(&str, Option<u16>)> {
    if let Some(stripped) = rest.strip_prefix('[') {
        let Some(end) = stripped.find(']') else {
            return Err(Error::Config(format!(
                "AO3I_FLARESOLVERR_URL has an unterminated IPv6 host: {rest:?}"
            )));
        };
        let host = &stripped[..end];
        let remainder = &stripped[end + 1..];
        let port = match remainder.strip_prefix(':') {
            Some(port) => Some(parse_port(port)?),
            None if remainder.is_empty() => None,
            None => {
                return Err(Error::Config(format!(
                    "AO3I_FLARESOLVERR_URL has trailing characters: {remainder:?}"
                )));
            }
        };
        return Ok((host, port));
    }
    match rest.split_once(':') {
        Some((host, port)) => Ok((host, Some(parse_port(port)?))),
        None => Ok((rest, None)),
    }
}

fn parse_port(port: &str) -> Result<u16> {
    port.parse::<u16>().map_err(|_| {
        Error::Config(format!(
            "AO3I_FLARESOLVERR_URL has an invalid port: {port:?}"
        ))
    })
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        other => Err(Error::Config(format!(
            "{name} expects a boolean, got {other:?}"
        ))),
    }
}

/// `$XDG_CONFIG_HOME/ao3instance`, else `$HOME/.config/ao3instance`.
fn default_data_dir(
    xdg_config_home: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> Result<PathBuf> {
    if let Some(xdg) = xdg_config_home
        && !xdg.as_os_str().is_empty()
    {
        return Ok(xdg.join("ao3instance"));
    }
    let home =
        home.ok_or_else(|| Error::Config("neither XDG_CONFIG_HOME nor HOME is set".into()))?;
    Ok(home.join(".config").join("ao3instance"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: &[(&str, &str)]) -> Config {
        let mut pairs = vec![
            ("XDG_CONFIG_HOME".to_string(), "/tmp/xdg".to_string()),
            ("HOME".to_string(), "/tmp/home".to_string()),
        ];
        for (k, v) in extra {
            pairs.push((k.to_string(), v.to_string()));
        }
        Config::from_pairs(pairs).unwrap()
    }

    #[test]
    fn defaults_are_conservative() {
        let c = cfg(&[]);
        assert_eq!(c.base_url, consts::BASE_URL);
        assert_eq!(c.data_dir, PathBuf::from("/tmp/xdg/ao3instance"));
        assert_eq!(c.key_file, PathBuf::from("/tmp/xdg/ao3instance/key"));
        assert!(!c.write_enabled, "writes must be off by default");
        assert_eq!(c.min_request_spacing, consts::MIN_REQUEST_SPACING);
        assert_eq!(c.bind, "127.0.0.1");
        assert_eq!(c.port, consts::DEFAULT_PORT);
        assert_eq!(c.db_file, PathBuf::from("/tmp/xdg/ao3instance/ao3i.db"));
        assert!(
            c.log_http_errors,
            "structured HTTP diagnostics are on by default"
        );
        assert_eq!(c.http_log_retention, consts::HTTP_ERROR_LOG_RETENTION);
        assert!(c.user_agent.starts_with("AO3Instance/"));
    }

    #[test]
    fn overrides_apply_and_base_url_is_trimmed() {
        let c = cfg(&[
            ("AO3I_BASE_URL", "http://127.0.0.1:3000/"),
            ("AO3I_DATA_DIR", "/srv/ao3i"),
            ("AO3I_KEY_FILE", "/srv/ao3i/secret.key"),
            ("AO3I_WRITE_ENABLED", "true"),
            ("AO3I_MIN_SPACING_MS", "0"),
            ("AO3I_BIND", "0.0.0.0"),
            ("AO3I_PORT", "9000"),
            ("AO3I_DB_FILE", "/srv/ao3i/db.sqlite"),
            ("AO3I_LOG_HTTP_ERRORS", "off"),
            ("AO3I_HTTP_LOG_RETENTION", "42"),
        ]);
        assert_eq!(c.base_url, "http://127.0.0.1:3000");
        assert_eq!(c.data_dir, PathBuf::from("/srv/ao3i"));
        assert_eq!(c.key_file, PathBuf::from("/srv/ao3i/secret.key"));
        assert!(c.write_enabled);
        assert_eq!(c.min_request_spacing, Duration::ZERO);
        assert_eq!(c.bind, "0.0.0.0");
        assert_eq!(c.port, 9000);
        assert_eq!(c.db_file, PathBuf::from("/srv/ao3i/db.sqlite"));
        assert!(!c.log_http_errors);
        assert_eq!(c.http_log_retention, 42);
    }

    #[test]
    fn solver_overrides_apply() {
        let c = cfg(&[
            ("AO3I_FLARESOLVERR", "off"),
            ("AO3I_FLARESOLVERR_URL", "http://127.0.0.1:9999/"),
            ("AO3I_FLARESOLVERR_SEND_COOKIES", "true"),
            ("AO3I_FLARESOLVERR_MAX_TIMEOUT_MS", "90000"),
        ]);
        assert_eq!(c.solver.mode, SolverMode::Off);
        assert_eq!(c.solver.origin, "http://127.0.0.1:9999");
        assert!(c.solver.send_cookies);
        assert_eq!(c.solver.max_timeout_ms, 90_000);

        let remote = cfg(&[
            ("AO3I_FLARESOLVERR", "on"),
            ("AO3I_FLARESOLVERR_URL", "https://localhost:8191"),
            ("AO3I_FLARESOLVERR_ALLOW_REMOTE", "1"),
        ]);
        assert_eq!(remote.solver.mode, SolverMode::On);
        assert_eq!(remote.solver.origin, "https://localhost:8191");
        assert!(remote.solver.allow_remote);
    }

    #[test]
    fn solver_settings_are_validated() {
        for mode in ["sometimes", "yes"] {
            assert!(
                matches!(
                    Config::from_pairs([
                        ("XDG_CONFIG_HOME", "/tmp/x-dir"),
                        ("AO3I_FLARESOLVERR", mode),
                    ]),
                    Err(Error::Config(_))
                ),
                "mode {mode:?} must be rejected"
            );
        }
        for url in [
            "ftp://127.0.0.1:8191",
            "127.0.0.1:8191",
            "http://127.0.0.1:8191/v1",
            "http://127.0.0.1:8191/?x=1",
            "http://user:pass@127.0.0.1:8191",
            "http://10.1.2.3:8191",
            "http://[::1",
            "http://127.0.0.1:notaport",
            "http://127.0.0.1:0",
        ] {
            assert!(
                matches!(
                    Config::from_pairs([
                        ("XDG_CONFIG_HOME", "/tmp/x-dir"),
                        ("AO3I_FLARESOLVERR_URL", url),
                    ]),
                    Err(Error::Config(_))
                ),
                "url {url:?} must be rejected"
            );
        }
        let lan = cfg(&[
            ("AO3I_FLARESOLVERR_URL", "http://10.1.2.3:8191"),
            ("AO3I_FLARESOLVERR_ALLOW_REMOTE", "on"),
        ]);
        assert_eq!(lan.solver.origin, "http://10.1.2.3:8191");
        for timeout in ["4999", "180001", "soon"] {
            assert!(
                matches!(
                    Config::from_pairs([
                        ("XDG_CONFIG_HOME", "/tmp/x-dir"),
                        ("AO3I_FLARESOLVERR_MAX_TIMEOUT_MS", timeout),
                    ]),
                    Err(Error::Config(_))
                ),
                "timeout {timeout:?} must be rejected"
            );
        }
    }

    #[test]
    fn http_diagnostic_knobs_are_strict() {
        for value in ["0", "-5", "many"] {
            assert!(
                matches!(
                    Config::from_pairs([
                        ("XDG_CONFIG_HOME", "/tmp/x-dir"),
                        ("AO3I_HTTP_LOG_RETENTION", value),
                    ]),
                    Err(Error::Config(_))
                ),
                "retention {value:?} must be rejected"
            );
        }
        let err = Config::from_pairs([("AO3I_LOG_HTTP_ERRORS", "maybe")]).unwrap_err();
        assert!(matches!(err, Error::Config(_)));
        assert!(err.to_string().contains("AO3I_LOG_HTTP_ERRORS"), "{err}");
    }

    #[test]
    fn key_file_follows_data_dir_when_unset() {
        let c = cfg(&[("AO3I_DATA_DIR", "/srv/ao3i")]);
        assert_eq!(c.key_file, PathBuf::from("/srv/ao3i/key"));
    }

    #[test]
    fn empty_base_url_is_rejected() {
        let err = Config::from_pairs([("AO3I_BASE_URL", "/")]).unwrap_err();
        assert!(matches!(err, Error::Config(_)));
    }

    #[test]
    fn spacing_must_be_a_number() {
        assert_eq!(
            Config::from_pairs([
                ("XDG_CONFIG_HOME", "/tmp/x-dir"),
                ("AO3I_MIN_SPACING_MS", "0"),
            ])
            .unwrap()
            .min_request_spacing,
            Duration::ZERO
        );
        assert!(matches!(
            Config::from_pairs([
                ("XDG_CONFIG_HOME", "/tmp/x-dir"),
                ("AO3I_MIN_SPACING_MS", "fast"),
            ]),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn boolean_parsing_is_strict() {
        assert!(cfg(&[("AO3I_WRITE_ENABLED", "1")]).write_enabled);
        assert!(!cfg(&[("AO3I_WRITE_ENABLED", "off")]).write_enabled);
        let err = Config::from_pairs([("AO3I_WRITE_ENABLED", "sometimes")]).unwrap_err();
        assert!(matches!(err, Error::Config(_)));
    }

    #[test]
    fn foreign_variables_are_ignored() {
        let c = cfg(&[("PATH", "/usr/bin"), ("AO3I_UNKNOWN", "x")]);
        assert_eq!(c.base_url, consts::BASE_URL);
    }
}
