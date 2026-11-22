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
                "WRITE_ENABLED" => write_enabled = parse_bool(&value)?,
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
        })
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        other => Err(Error::Config(format!(
            "AO3I_WRITE_ENABLED expects a boolean, got {other:?}"
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
        ]);
        assert_eq!(c.base_url, "http://127.0.0.1:3000");
        assert_eq!(c.data_dir, PathBuf::from("/srv/ao3i"));
        assert_eq!(c.key_file, PathBuf::from("/srv/ao3i/secret.key"));
        assert!(c.write_enabled);
        assert_eq!(c.min_request_spacing, Duration::ZERO);
        assert_eq!(c.bind, "0.0.0.0");
        assert_eq!(c.port, 9000);
        assert_eq!(c.db_file, PathBuf::from("/srv/ao3i/db.sqlite"));
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
