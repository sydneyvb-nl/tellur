//! Unified setup and upgrade-reconciliation flow.
//!
//! The detailed setup/connect commands remain available for compatibility, but
//! this is the supported user journey: one idempotent command configures global
//! capture, the current repository, Git automation, and an optional Team Hub.

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use anyhow::{Result, bail};
use tellur_core::storage::RepoStorage;

use crate::connect::{self, ConnectOptions};
use crate::hub::Credentials;
use crate::push::cmd_login;
use crate::setup;

pub(crate) struct SetupOptions<'a> {
    pub(crate) hub: Option<&'a str>,
    pub(crate) local_only: bool,
    pub(crate) remote: &'a str,
    pub(crate) no_background: bool,
    pub(crate) no_browser: bool,
    pub(crate) yes: bool,
    pub(crate) update: bool,
}

pub(crate) fn cmd_setup(opts: SetupOptions<'_>) -> Result<()> {
    println!(
        "Tellur {}\n",
        if opts.update { "setup update" } else { "setup" }
    );

    let hub = choose_hub(&opts)?;
    let mut credentials = Credentials::load()?;
    let already_logged_in = hub
        .as_deref()
        .is_some_and(|host| credentials.get(host).is_some());

    // The wizard owns the unattended-sync choice. Selecting an existing hub
    // promotes it to the default; explicitly choosing local-only clears the
    // default without deleting credentials that may still be useful later.
    let selected_default = hub.as_deref().filter(|_| already_logged_in);
    let desired_default = selected_default.map(ToOwned::to_owned);
    let should_persist_selection = opts.local_only || desired_default.is_some();
    let desired_disabled = opts.local_only;
    if should_persist_selection
        && (credentials.default_host != desired_default
            || credentials.unattended_sync_disabled != desired_disabled)
    {
        credentials.default_host = desired_default;
        credentials.unattended_sync_disabled = desired_disabled;
        credentials.save()?;
    }

    if let Some(host) = hub.as_deref()
        && !already_logged_in
        && !opts.update
    {
        match cmd_login(Some(host), opts.no_browser) {
            Ok(()) => {}
            Err(error) => {
                println!("⚠ Team Hub login skipped: {error}");
                println!("  Machine-wide local capture remains active; rerun setup to retry.");
            }
        }
    }

    setup::cmd_setup_agents(None)?;

    let storage = match RepoStorage::discover() {
        Ok(storage) => storage,
        Err(_) => {
            print_global_activation_summary();
            return Ok(());
        }
    };

    if opts.local_only
        && let Some(path) = crate::service::remove(&storage.root)?
    {
        println!("✓ Removed background Team Hub sync ({})", path.display());
    }

    let had_service = crate::service::status(&storage.root).is_some();
    let background = hub.is_some() && !opts.no_background && (!opts.update || had_service);
    connect::cmd_connect(ConnectOptions {
        hub: hub.as_deref(),
        remote: opts.remote,
        no_login: true,
        no_agents: true,
        background,
        push_interval: 900,
        no_browser: opts.no_browser,
        status: false,
        remove: false,
    })?;

    print_global_activation_summary();
    println!("\nNext time Tellur itself changes, run `tellur setup update`.");
    println!("Check the complete installation with `tellur setup status`.");
    Ok(())
}

fn print_global_activation_summary() {
    println!("\n✓ Tellur is installed once for this machine.");
    println!("  • Codex and Claude Code: global lifecycle hooks");
    println!("  • Gemini CLI and Antigravity: global lifecycle hooks + MCP");
    println!("  • Cursor and Windsurf: global MCP + editor capture settings");
    println!("  • VS Code and JetBrains: release-installer packages + global capture settings");
    println!("  • every Git repository activates automatically on first agent/editor use");
    println!("  • auto-activation also installs commit and pre-push Git automation");
    println!("  Create .tellur/disable in a repository to opt out.");
}

fn choose_hub(opts: &SetupOptions<'_>) -> Result<Option<String>> {
    if opts.local_only {
        return Ok(None);
    }
    if let Some(hub) = opts.hub {
        validate_hub_url(hub)?;
        return Ok(Some(hub.trim_end_matches('/').to_string()));
    }
    if let Ok(hub) = std::env::var("TELLUR_HUB_URL")
        && !hub.trim().is_empty()
    {
        validate_hub_url(&hub)?;
        return Ok(Some(hub.trim_end_matches('/').to_string()));
    }

    let credentials = Credentials::load()?;
    if credentials.unattended_sync_disabled {
        return Ok(None);
    }
    if let Some(default) = credentials.default_host.as_deref()
        && credentials.get(default).is_some()
    {
        return Ok(Some(default.to_string()));
    }
    if credentials.hosts.len() == 1 {
        return Ok(credentials.hosts.keys().next().cloned());
    }
    if opts.update || opts.yes || !io::stdin().is_terminal() {
        return Ok(None);
    }

    print!("Team Hub URL (leave empty for local-only): ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim();
    if answer.is_empty() {
        return Ok(None);
    }
    validate_hub_url(answer)?;
    Ok(Some(answer.trim_end_matches('/').to_string()))
}

fn validate_hub_url(hub: &str) -> Result<()> {
    let Some((scheme, rest)) = hub.split_once("://") else {
        bail!("Team Hub URL must include https://")
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        bail!("Team Hub URL must contain a host and must not contain credentials")
    }
    let host = authority
        .strip_prefix('[')
        .and_then(|value| value.split_once(']').map(|(host, _)| host))
        .unwrap_or_else(|| authority.split(':').next().unwrap_or_default());
    if scheme == "https" || (scheme == "http" && matches!(host, "127.0.0.1" | "::1" | "localhost"))
    {
        return Ok(());
    }
    bail!("Team Hub URL must use https (loopback http is allowed for local development)")
}

pub(crate) fn cmd_setup_status(home: Option<&Path>, remote: &str) -> Result<()> {
    println!("Tellur setup status\n");
    println!("Repository activation: automatic on first configured agent/editor activity\n");
    setup::cmd_setup_status(home)?;
    match RepoStorage::discover() {
        Ok(storage) => {
            println!();
            connect::connect_status(&storage, remote)
        }
        Err(_) => {
            println!("\nCurrent repository: not detected");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_hub_url;

    #[test]
    fn accepts_secure_and_loopback_hubs() {
        assert!(validate_hub_url("https://hub.example.com").is_ok());
        assert!(validate_hub_url("http://127.0.0.1:4920").is_ok());
        assert!(validate_hub_url("http://localhost:4920").is_ok());
        assert!(validate_hub_url("http://[::1]:4920").is_ok());
        assert!(validate_hub_url("http://hub.example.com").is_err());
        assert!(validate_hub_url("http://localhost.evil.example").is_err());
        assert!(validate_hub_url("https://user:secret@hub.example.com").is_err());
        assert!(validate_hub_url("https://").is_err());
    }
}
