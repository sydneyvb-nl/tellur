//! Private-repo source proxy (A12). When a repo has a `raw` source template and
//! an access token configured, the hub fetches raw file bytes from the provider
//! on the browser's behalf, so the inline source gutter works for **private**
//! repos that the browser can't fetch cross-origin.
//!
//! Security posture: the fetch target is rebuilt from the admin-set template and
//! re-validated against a fixed host **allowlist** (SSRF guard — an admin typo or
//! a tampered template can't make the hub call an arbitrary host), restricted to
//! `https`, size-capped, and the token is sent only as the provider's auth header
//! and never returned to any client. The bytes are the org's own source served to
//! org members (viewer+), so they are returned faithfully (not redacted); keep the
//! configured token least-privilege (read-only, scoped to the connected repo).

use anyhow::{Result, bail};

/// Hosts the proxy is allowed to fetch from. Matches the dashboard CSP
/// `connect-src` set plus the providers' authenticated raw/content APIs.
const ALLOWED_HOSTS: &[&str] = &[
    "raw.githubusercontent.com",
    "api.github.com",
    "gitlab.com",
    "bitbucket.org",
    "api.bitbucket.org",
];

/// Cap on a proxied file (2 MB) — bounds memory and matches the browser-side cap.
pub const MAX_SOURCE_BYTES: usize = 2_000_000;

/// Percent-encode a path, preserving `/` between segments (so `{path}` expands to
/// a valid multi-segment path without corrupting `#`/`?`/spaces in a filename).
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            let mut out = String::with_capacity(seg.len());
            for b in seg.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        out.push(b as char)
                    }
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Extract the lowercased host from an `https://` URL, rejecting userinfo and an
/// explicit port (the allowlisted providers are all default-443; userinfo is a
/// classic SSRF/credential-smuggling vector).
fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains(':') {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}

/// Build and validate the raw-bytes URL for `path` from the template. Substitutes
/// `{path}`, requires `https`, and requires an allowlisted host (SSRF guard).
pub fn resolve_raw_url(raw_template: &str, path: &str) -> Result<String> {
    let url = raw_template.replace("{path}", &encode_path(path));
    if !url.starts_with("https://") {
        bail!("source template must be an https:// URL");
    }
    let host = host_of(&url).ok_or_else(|| anyhow::anyhow!("source URL has no valid host"))?;
    if !ALLOWED_HOSTS.contains(&host.as_str()) {
        bail!("source host '{host}' is not in the allowed provider list");
    }
    Ok(url)
}

/// Provider auth headers for an allowlisted host (only when a token is set).
/// GitHub/Bitbucket use `Authorization: Bearer`; GitLab uses `PRIVATE-TOKEN`. The
/// GitHub contents API additionally needs an `Accept` for raw bytes.
pub fn provider_auth_headers(host: &str, token: &str) -> Vec<(String, String)> {
    match host {
        "api.github.com" => vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("Accept".into(), "application/vnd.github.raw".into()),
        ],
        "raw.githubusercontent.com" | "bitbucket.org" | "api.bitbucket.org" => {
            vec![("Authorization".into(), format!("Bearer {token}"))]
        }
        "gitlab.com" => vec![("PRIVATE-TOKEN".into(), token.to_string())],
        _ => vec![],
    }
}

/// Fetch the (validated) URL over HTTPS with the optional provider token, capped
/// at [`MAX_SOURCE_BYTES`]. Network call — thin by design; the validation above
/// is what's unit-tested. Returns the file text.
pub fn fetch_blob(url: &str, token: Option<&str>) -> Result<String> {
    let host = host_of(url).ok_or_else(|| anyhow::anyhow!("invalid source URL"))?;
    let mut req = ureq::get(url);
    if let Some(tok) = token {
        for (k, v) in provider_auth_headers(&host, tok) {
            req = req.set(&k, &v);
        }
    }
    let resp = req
        .call()
        .map_err(|e| anyhow::anyhow!("provider fetch failed: {e}"))?;
    if let Some(len) = resp
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        && len > MAX_SOURCE_BYTES
    {
        bail!("source file too large");
    }
    // Read with a hard cap even if Content-Length was absent or lied.
    use std::io::Read;
    let mut buf = Vec::with_capacity(8192);
    resp.into_reader()
        .take((MAX_SOURCE_BYTES as u64) + 1)
        .read_to_end(&mut buf)?;
    if buf.len() > MAX_SOURCE_BYTES {
        bail!("source file too large");
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_and_encodes_allowlisted_url() {
        let url = resolve_raw_url(
            "https://raw.githubusercontent.com/acme/app/main/{path}",
            "src/a b#c.rs",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/acme/app/main/src/a%20b%23c.rs"
        );
    }

    #[test]
    fn rejects_non_allowlisted_host() {
        let err = resolve_raw_url("https://evil.example.com/{path}", "x").unwrap_err();
        assert!(err.to_string().contains("not in the allowed"));
    }

    #[test]
    fn rejects_non_https_and_userinfo_and_port() {
        assert!(resolve_raw_url("http://raw.githubusercontent.com/{path}", "x").is_err());
        assert!(
            resolve_raw_url("https://user@raw.githubusercontent.com/{path}", "x").is_err(),
            "userinfo must be rejected (SSRF)"
        );
        assert!(
            resolve_raw_url("https://raw.githubusercontent.com:8080/{path}", "x").is_err(),
            "explicit port must be rejected"
        );
    }

    #[test]
    fn provider_auth_headers_per_host() {
        assert_eq!(
            provider_auth_headers("gitlab.com", "T"),
            vec![("PRIVATE-TOKEN".to_string(), "T".to_string())]
        );
        let gh = provider_auth_headers("api.github.com", "T");
        assert!(gh.contains(&("Authorization".into(), "Bearer T".into())));
        assert!(gh.contains(&("Accept".into(), "application/vnd.github.raw".into())));
        assert_eq!(
            provider_auth_headers("raw.githubusercontent.com", "T"),
            vec![("Authorization".to_string(), "Bearer T".to_string())]
        );
    }
}
