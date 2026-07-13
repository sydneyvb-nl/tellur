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
///
/// Doubles as the device-token response shape: the hub returns the bearer token
/// as `access_token` (RFC 8628), so the field accepts that name as an alias
/// while still being stored on disk as `token`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCredentials {
    /// Bearer token (a member API token minted by the hub on approval).
    #[serde(alias = "access_token")]
    pub token: String,
    pub org_id: String,
    pub member_id: String,
    pub role: String,
}

/// The on-disk credentials file: a map of normalized hub URL → credentials,
/// plus the machine-wide default used by unattended Git automation.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub default_host: Option<String>,
    /// Explicit opt-out selected by `tellur setup --local-only`. This is
    /// distinct from a missing default in legacy credential files.
    #[serde(default)]
    pub unattended_sync_disabled: bool,
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

/// Percent-encode a single URL **path segment** (RFC 3986 unreserved set kept
/// verbatim, everything else `%XX`). The default repo name comes from the local
/// directory name, which can contain spaces or `#`, so segments must be encoded
/// or the request builds an invalid/truncated path that misses the repo.
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
        encode_segment(org),
        encode_segment(repo)
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

/// POST a batch of line-level file attributions to the hub. Returns the number
/// accepted. The hub upserts per file, so re-pushing the same files is
/// idempotent (it overwrites with the current attribution state).
pub fn ingest_attributions(
    hub: &str,
    token: &str,
    org: &str,
    repo: &str,
    files: &[serde_json::Value],
) -> Result<usize> {
    let url = format!(
        "{}/v1/orgs/{}/repos/{}/attributions",
        normalize_host(hub),
        encode_segment(org),
        encode_segment(repo)
    );
    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "attributions": files }))
        .map_err(map_transport)?;
    let body: serde_json::Value = resp.into_json().unwrap_or(serde_json::json!({}));
    let accepted = body
        .get("files")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(files.len());
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

#[cfg(test)]
mod tests {
    use super::{HostCredentials, encode_segment};

    #[test]
    fn device_token_response_deserializes_into_credentials() {
        // The exact shape the hub returns from POST /v1/device/token (RFC 8628
        // `access_token`); it must map onto the stored `token` field.
        let body = serde_json::json!({
            "access_token": "tlr_abc123",
            "token_type": "Bearer",
            "org_id": "org_1",
            "member_id": "mbr_1",
            "role": "admin",
        });
        let creds: HostCredentials = serde_json::from_value(body).unwrap();
        assert_eq!(creds.token, "tlr_abc123");
        assert_eq!(creds.org_id, "org_1");
        // Round-trips to disk as `token` (not `access_token`).
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("\"token\":\"tlr_abc123\""));
        assert!(!json.contains("access_token"));
    }

    #[test]
    fn encode_segment_escapes_reserved_chars() {
        assert_eq!(encode_segment("my repo"), "my%20repo");
        assert_eq!(encode_segment("foo#bar"), "foo%23bar");
        assert_eq!(encode_segment("a/b?c"), "a%2Fb%3Fc");
        // Unreserved characters pass through untouched.
        assert_eq!(encode_segment("repo-1_v.2~x"), "repo-1_v.2~x");
    }
}
