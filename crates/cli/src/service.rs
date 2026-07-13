//! Per-user background push service for `tellur connect --background`.
//!
//! Registers a per-repository OS service that runs `tellur push` on an interval,
//! so locally-captured events reach the hub **without** waiting for a `git push`
//! (the `pre-push` hook covers the push-time flush; this covers idle machines).
//! It is the always-on half of Team Hub onboarding. The unified setup wizard
//! installs it when a hub is selected unless the user passes `--no-background`;
//! the legacy `tellur connect` surface retains its explicit `--background` flag.
//!
//! Backends: launchd (macOS), systemd `--user` (Linux). Other platforms return a
//! clear "unsupported" error. The activation step (`launchctl`/`systemctl`) is
//! best-effort and skipped when `TELLUR_CONNECT_NO_ACTIVATE` is set (used by the
//! integration tests so they never touch the real session).

use anyhow::{Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Stable per-repo id used in the service label/filename.
fn repo_id(repo_root: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    repo_root.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Human-readable summary of what `install`/`status` operates on.
pub struct InstalledService {
    pub path: PathBuf,
    pub label: String,
}

fn activation_enabled() -> bool {
    std::env::var_os("TELLUR_CONNECT_NO_ACTIVATE").is_none()
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .context("HOME is not set — cannot locate the per-user service directory")
}

// ---------------------------------------------------------------------------
// macOS (launchd)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn label(repo_root: &Path) -> String {
    format!("dev.tellur.push.{}", repo_id(repo_root))
}

#[cfg(target_os = "macos")]
fn service_path(repo_root: &Path) -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label(repo_root))))
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
pub fn install(repo_root: &Path, exe: &Path, interval_secs: u64) -> Result<InstalledService> {
    let label = label(repo_root);
    let path = service_path(repo_root)?;
    std::fs::create_dir_all(path.parent().unwrap())
        .with_context(|| format!("failed to create {}", path.parent().unwrap().display()))?;
    let log = repo_root.join(".tellur/connect-push.log");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\t<string>{label}</string>\n\
\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{exe}</string>\n\t\t<string>push</string>\n\t</array>\n\
\t<key>WorkingDirectory</key>\n\t<string>{root}</string>\n\
\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>TELLUR_UNATTENDED_SYNC</key>\n\t\t<string>1</string>\n\t</dict>\n\
\t<key>StartInterval</key>\n\t<integer>{interval_secs}</integer>\n\
\t<key>RunAtLoad</key>\n\t<true/>\n\
\t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\
\t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n\
</dict>\n\
</plist>\n",
        label = xml_escape(&label),
        exe = xml_escape(&exe.to_string_lossy()),
        root = xml_escape(&repo_root.to_string_lossy()),
        log = xml_escape(&log.to_string_lossy()),
    );
    std::fs::write(&path, plist).with_context(|| format!("failed to write {}", path.display()))?;

    if activation_enabled() {
        // Best-effort: a LaunchAgent also auto-loads at next login regardless.
        let _ = std::process::Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&path)
            .output();
    }
    Ok(InstalledService { path, label })
}

#[cfg(target_os = "macos")]
pub fn remove(repo_root: &Path) -> Result<Option<PathBuf>> {
    let path = service_path(repo_root)?;
    if !path.exists() {
        return Ok(None);
    }
    if activation_enabled() {
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&path)
            .output();
    }
    std::fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    Ok(Some(path))
}

#[cfg(target_os = "macos")]
pub fn status(repo_root: &Path) -> Option<PathBuf> {
    service_path(repo_root).ok().filter(|p| p.exists())
}

// ---------------------------------------------------------------------------
// Linux (systemd --user)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn unit_name(repo_root: &Path) -> String {
    format!("tellur-push-{}", repo_id(repo_root))
}

#[cfg(target_os = "linux")]
fn systemd_user_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .map(Ok)
        .unwrap_or_else(|| home_dir().map(|h| h.join(".config")))?;
    Ok(base.join("systemd/user"))
}

#[cfg(target_os = "linux")]
fn service_path(repo_root: &Path) -> Result<PathBuf> {
    Ok(systemd_user_dir()?.join(format!("{}.service", unit_name(repo_root))))
}

#[cfg(target_os = "linux")]
fn timer_path(repo_root: &Path) -> Result<PathBuf> {
    Ok(systemd_user_dir()?.join(format!("{}.timer", unit_name(repo_root))))
}

#[cfg(target_os = "linux")]
pub fn install(repo_root: &Path, exe: &Path, interval_secs: u64) -> Result<InstalledService> {
    let dir = systemd_user_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let name = unit_name(repo_root);
    let root = repo_root.to_string_lossy();
    let exe = exe.to_string_lossy();

    let service = format!(
        "[Unit]\n\
Description=Tellur background push for {root}\n\n\
[Service]\n\
Type=oneshot\n\
WorkingDirectory={root}\n\
Environment=TELLUR_UNATTENDED_SYNC=1\n\
ExecStart={exe} push\n"
    );
    let timer = format!(
        "[Unit]\n\
Description=Tellur background push timer for {root}\n\n\
[Timer]\n\
OnBootSec={interval_secs}\n\
OnUnitActiveSec={interval_secs}\n\
Persistent=true\n\n\
[Install]\n\
WantedBy=timers.target\n"
    );
    let service_path = service_path(repo_root)?;
    let timer_path = timer_path(repo_root)?;
    std::fs::write(&service_path, service)
        .with_context(|| format!("failed to write {}", service_path.display()))?;
    std::fs::write(&timer_path, timer)
        .with_context(|| format!("failed to write {}", timer_path.display()))?;

    if activation_enabled() {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", &format!("{name}.timer")])
            .output();
    }
    Ok(InstalledService {
        path: timer_path,
        label: name,
    })
}

#[cfg(target_os = "linux")]
pub fn remove(repo_root: &Path) -> Result<Option<PathBuf>> {
    let service_path = service_path(repo_root)?;
    let timer_path = timer_path(repo_root)?;
    if !service_path.exists() && !timer_path.exists() {
        return Ok(None);
    }
    if activation_enabled() {
        let _ = std::process::Command::new("systemctl")
            .args([
                "--user",
                "disable",
                "--now",
                &format!("{}.timer", unit_name(repo_root)),
            ])
            .output();
    }
    if timer_path.exists() {
        std::fs::remove_file(&timer_path)
            .with_context(|| format!("failed to remove {}", timer_path.display()))?;
    }
    if service_path.exists() {
        std::fs::remove_file(&service_path)
            .with_context(|| format!("failed to remove {}", service_path.display()))?;
    }
    Ok(Some(timer_path))
}

#[cfg(target_os = "linux")]
pub fn status(repo_root: &Path) -> Option<PathBuf> {
    timer_path(repo_root).ok().filter(|p| p.exists())
}

// ---------------------------------------------------------------------------
// Unsupported platforms
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install(_repo_root: &Path, _exe: &Path, _interval_secs: u64) -> Result<InstalledService> {
    anyhow::bail!(
        "background push service is only supported on macOS (launchd) and Linux (systemd --user)"
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn remove(_repo_root: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn status(_repo_root: &Path) -> Option<PathBuf> {
    None
}
