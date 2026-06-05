//! 12-factor configuration, validated at boot (fail fast on insecure config).

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::ServerError;

/// Server configuration, sourced from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address to bind. Defaults to loopback.
    pub bind: SocketAddr,
    /// SQLite database path (use `:memory:` for ephemeral). Ignored when
    /// [`Config::database_url`] is set.
    pub db_path: PathBuf,
    /// Optional Postgres connection string. When set, the server uses the
    /// Postgres backend (horizontal scale) instead of the embedded SQLite
    /// store. Sourced from `TELLUR_DATABASE_URL`.
    pub database_url: Option<String>,
    /// Explicit opt-in required to bind a non-loopback address (B0 has no
    /// auth/TLS yet, so we refuse to expose the server by accident).
    pub allow_non_loopback: bool,
}

impl Config {
    /// Build configuration from environment variables.
    pub fn from_env() -> Result<Self, ServerError> {
        let bind_raw =
            std::env::var("TELLUR_SERVER_BIND").unwrap_or_else(|_| "127.0.0.1:4920".to_string());
        let bind: SocketAddr = bind_raw
            .parse()
            .map_err(|e| ServerError::Config(format!("invalid TELLUR_SERVER_BIND: {e}")))?;
        let db_path = std::env::var("TELLUR_SERVER_DB")
            .unwrap_or_else(|_| "tellur-hub.db".to_string())
            .into();
        let database_url = std::env::var("TELLUR_DATABASE_URL")
            .ok()
            .filter(|v| !v.is_empty());
        let allow_non_loopback = env_flag("TELLUR_SERVER_ALLOW_NON_LOOPBACK");

        let config = Self {
            bind,
            db_path,
            database_url,
            allow_non_loopback,
        };
        config.validate()?;
        Ok(config)
    }

    /// Reject insecure configurations.
    pub fn validate(&self) -> Result<(), ServerError> {
        if !self.bind.ip().is_loopback() && !self.allow_non_loopback {
            return Err(ServerError::Config(format!(
                "refusing to bind non-loopback address {} without \
                 TELLUR_SERVER_ALLOW_NON_LOOPBACK=1 (no auth/TLS yet in this build)",
                self.bind
            )));
        }
        Ok(())
    }
}

fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(bind: &str, allow: bool) -> Config {
        Config {
            bind: bind.parse().unwrap(),
            db_path: ":memory:".into(),
            database_url: None,
            allow_non_loopback: allow,
        }
    }

    #[test]
    fn loopback_is_allowed() {
        assert!(cfg("127.0.0.1:4920", false).validate().is_ok());
    }

    #[test]
    fn non_loopback_refused_without_optin() {
        assert!(cfg("0.0.0.0:4920", false).validate().is_err());
    }

    #[test]
    fn non_loopback_allowed_with_optin() {
        assert!(cfg("0.0.0.0:4920", true).validate().is_ok());
    }
}
