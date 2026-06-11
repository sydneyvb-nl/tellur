//! Hub client: device-login flow, stored credentials, and event push.
//!
//! Couples a local Tellur checkout to a self-hosted team hub with a seamless,
//! `gh`-style flow: `tellur login` runs an OAuth 2.0 Device Authorization Grant
//! (RFC 8628) against the hub and stores the minted token under the per-user
//! config dir; `tellur push` reads that token and forwards locally-captured
//! events to the hub's ingest API, tracking a per-target high-water mark so
//! repeated pushes are incremental and idempotent.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Credentials and identity stored for a single hub host after `tellur login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCredentials {
    /// Bearer token (a member API token minted by the hub on approval).
    pub token: String,
    pub org_id: String,
    pub member_id: String,
    pub role: String,
}

/// The on-disk credentials file: a map of normalized hub URL → credentials.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub hosts: BTreeMap<String, HostCredentials>,
}

impl Credentials {
    /// Path to the credentials file (`$XDG_CONFIG_HOME|~/.config|%APPDATA%`
    /// `/tellur/hosts.json`). Resolved from env so we add no `dirs` dependency.
    pub fn path() -> Result<PathBuf> {
        let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from)
            && xdg.is_absolute()
        {
            xdg
        } else if cfg!(windows) {
            std::env::var("APPDATA")
                .map(PathBuf::from)
                .context("APPDATA not set")?
        } else {
            let home = std::env::var("HOME").context("HOME not set")?;
            PathBuf::from(home).join(".config")
        };
        Ok(base.join("tellur").join("hosts.json"))
    }

    /// Load the credentials file, or an empty set if it does not exist yet.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&body).context("credentials file is corrupt (delete it to reset)")
    }

    /// Persist the credentials file with owner-only permissions on Unix (the
    /// file holds bearer tokens).
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self)?;
        write_private(&path, body.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn get(&self, host: &str) -> Option<&HostCredentials> {
        self.hosts.get(&normalize_host(host))
    }
}

/// Write a file readable/writable only by the owner (0600) on Unix; a plain
/// write elsewhere.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    let mut f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?
    };
    #[cfg(not(unix))]
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    Ok(())
}

/// Normalize a hub URL for use as a stable credential key (strip trailing `/`).
pub fn normalize_host(hub: &str) -> String {
    hub.trim_end_matches('/').to_string()
}

/// The device-authorization ticket returned by `POST /v1/device/authorize`.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    #[serde(default)]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: u64,
}

/// Begin a device-authorization flow against the hub.
pub fn device_authorize(hub: &str) -> Result<DeviceAuthorization> {
    let url = format!("{}/v1/device/authorize", normalize_host(hub));
    let resp = ureq::post(&url)
        .send_json(serde_json::json!({}))
        .map_err(map_transport)?;
    resp.into_json()
        .context("hub returned an invalid device-authorization response")
}

/// One poll of `POST /v1/device/token`.
pub enum DevicePoll {
    /// Approved: the minted credentials, ready to store.
    Approved(HostCredentials),
    /// Still waiting for the human to approve in the browser.
    Pending,
    /// Hub asked us to back off; increase the interval by this many seconds.
    SlowDown,
    /// The human denied the request.
    Denied,
    /// The request expired before approval.
    Expired,
}

/// Poll the hub once for the outcome of a device login.
pub fn device_poll(hub: &str, device_code: &str) -> Result<DevicePoll> {
    let url = format!("{}/v1/device/token", normalize_host(hub));
    match ureq::post(&url).send_json(serde_json::json!({ "device_code": device_code })) {
        Ok(resp) => {
            let creds: HostCredentials = resp
                .into_json()
                .context("hub returned an invalid token response")?;
            Ok(DevicePoll::Approved(creds))
        }
        Err(ureq::Error::Status(400, resp)) => {
            let body: serde_json::Value = resp
                .into_json()
                .context("hub returned an invalid pending response")?;
            match body.get("error").and_then(|e| e.as_str()) {
                Some("authorization_pending") => Ok(DevicePoll::Pending),
                Some("slow_down") => Ok(DevicePoll::SlowDown),
                Some("access_denied") => Ok(DevicePoll::Denied),
                Some("expired_token") => Ok(DevicePoll::Expired),
                other => bail!(
                    "hub returned unexpected error: {}",
                    other.unwrap_or("(none)")
                ),
            }
        }
        Err(e) => Err(map_transport(e)),
    }
}

/// POST a batch of ingest-wire events to the hub. Returns the number accepted.
pub fn ingest_events(
    hub: &str,
    token: &str,
    org: &str,
    repo: &str,
    events: &[serde_json::Value],
) -> Result<usize> {
    let url = format!(
        "{}/v1/orgs/{}/repos/{}/events",
        normalize_host(hub),
        org,
        repo
    );
    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "events": events }))
        .map_err(map_transport)?;
    let body: serde_json::Value = resp.into_json().unwrap_or(serde_json::json!({}));
    let accepted = body
        .get("count")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(events.len());
    Ok(accepted)
}

/// Turn a ureq transport/status error into a readable message (ureq embeds the
/// whole response in its Debug, which is noisy for users).
fn map_transport(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let detail = resp
                .into_json::<serde_json::Value>()
                .ok()
                .and_then(|v| {
                    v.get("title")
                        .or_else(|| v.get("error"))
                        .and_then(|s| s.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            if detail.is_empty() {
                anyhow::anyhow!("hub returned HTTP {code}")
            } else {
                anyhow::anyhow!("hub returned HTTP {code}: {detail}")
            }
        }
        ureq::Error::Transport(t) => anyhow::anyhow!("could not reach hub: {t}"),
    }
}
