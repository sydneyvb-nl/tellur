//! `tellur connect` — one-time zero-touch setup that wires hub login, agent
//! capture, and managed git hooks, plus its `--status` / `--remove` modes.

use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail};

use tellur_core::storage::RepoStorage;

use crate::git::{git_config_get_all, git_hooks_dir, git_remote_exists, git_try};
use crate::hub;
use crate::notes::cmd_notes_install_config;
use crate::push::cmd_login;
use crate::service;
use crate::setup::cmd_setup_agents;
use crate::util::{shell_quote, tellur_executable_path};

const HOOK_BEGIN: &str = "# >>> tellur connect (managed) >>>";
const HOOK_END: &str = "# <<< tellur connect (managed) <<<";

/// Arguments for `tellur connect` (grouped to keep the dispatch readable).
pub(crate) struct ConnectOptions<'a> {
    pub(crate) hub: Option<&'a str>,
    pub(crate) remote: &'a str,
    pub(crate) no_login: bool,
    pub(crate) no_agents: bool,
    pub(crate) background: bool,
    pub(crate) push_interval: u64,
    pub(crate) no_browser: bool,
    pub(crate) status: bool,
    pub(crate) remove: bool,
}

/// `tellur connect` — one-time zero-touch setup. Wires hub login, agent capture,
/// and git hooks so a developer never has to run a `tellur` command again: every
/// commit refreshes `refs/notes/ai`, and every `git push` flushes events to the
/// hub and pushes the notes alongside the branch. With `--background` it also
/// installs an always-on per-user service that pushes on an interval. All
/// hub-touching steps are best-effort and never block git.
pub(crate) fn cmd_connect(opts: ConnectOptions) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if opts.remove {
        return connect_remove(&storage, opts.remote);
    }
    if opts.status {
        return connect_status(&storage, opts.remote);
    }

    if !storage.is_initialized() {
        storage.init()?;
        println!("✓ Initialized Tellur in {}", storage.root.display());
    }

    // 1. Hub login (best-effort — a missing/unreachable hub must not abort setup).
    if opts.no_login {
        println!("• Skipping hub login (--no-login).");
    } else {
        match cmd_login(opts.hub, opts.no_browser) {
            Ok(()) => {}
            Err(e) => {
                println!("⚠ Hub login skipped: {e}");
                println!(
                    "  Run `tellur login --hub <url>` later — capture and notes still work without it."
                );
            }
        }
    }

    // 2. Editor/agent capture integrations.
    if opts.no_agents {
        println!("• Skipping agent integrations (--no-agents).");
    } else {
        cmd_setup_agents(None)?;
    }

    // 3. Git hooks (chained, never clobbering an existing hook).
    let exe = tellur_executable_path()?;
    let exe_quoted = shell_quote(&exe.to_string_lossy());
    let hooks_dir = git_hooks_dir(&storage.root)?;
    install_managed_hook(&hooks_dir, "post-commit", &post_commit_block(&exe_quoted))?;
    install_managed_hook(&hooks_dir, "pre-push", &pre_push_block(&exe_quoted))?;
    println!(
        "✓ Installed git hooks in {} (post-commit, pre-push)",
        hooks_dir.display()
    );

    // 4. Notes fetch + rewrite config so notes travel with the repo. Only when
    //    the remote actually exists — writing `remote.<remote>.fetch` otherwise
    //    materialises a phantom remote that breaks a later `git remote add`.
    if git_remote_exists(&storage.root, opts.remote) {
        cmd_notes_install_config(opts.remote, tellur_core::notes::GIT_AI_NOTES_REF)?;
    } else {
        println!(
            "• Skipped notes fetch config: remote '{}' does not exist yet.",
            opts.remote
        );
        println!("  The pre-push hook will configure it automatically after the remote is added.");
    }

    // 5. Optional always-on background push service.
    if opts.background {
        let svc = service::install(&storage.root, &exe, opts.push_interval)?;
        println!(
            "✓ Installed background push service '{}' every {}s\n  {}",
            svc.label,
            opts.push_interval,
            svc.path.display()
        );
    }

    let hub_connected = hub::Credentials::load()
        .map(|credentials| !credentials.hosts.is_empty())
        .unwrap_or(false);

    println!("\n✓ Automatic capture is active for this repository.");
    println!("  • each commit refreshes refs/notes/ai locally");
    if hub_connected {
        println!("  • each `git push` flushes events to the Team Hub");
    }
    println!(
        "  • each `git push` publishes provenance notes to '{}'",
        opts.remote
    );
    if opts.background {
        println!(
            "  • a background service pushes events every {}s",
            opts.push_interval
        );
    } else {
        println!("  • background Team Hub sync is not installed");
    }
    println!("\nNote: pushing notes publishes commit-level AI attribution to anyone with");
    println!("repo read access. Undo any time with `tellur connect --remove`.");
    Ok(())
}

fn post_commit_block(exe: &str) -> String {
    format!(
        "{HOOK_BEGIN}\n# Refresh refs/notes/ai for the new commit (best-effort; never blocks).\n{exe} notes export >/dev/null 2>&1 || true\n{HOOK_END}"
    )
}

fn pre_push_block(exe: &str) -> String {
    // git passes the remote name as $1. The recursion guard stops the nested
    // `tellur notes push` (which runs `git push`) from re-entering this hook.
    format!(
        "{HOOK_BEGIN}\n# Flush events to the hub and publish authorship notes (best-effort; never blocks).\nif [ -z \"$TELLUR_CONNECT_PREPUSH\" ]; then\n\tTELLUR_CONNECT_PREPUSH=1 {exe} notes install-config \"${{1:-origin}}\" >/dev/null 2>&1 || true\n\tTELLUR_CONNECT_PREPUSH=1 {exe} push >/dev/null 2>&1 || true\n\tTELLUR_CONNECT_PREPUSH=1 {exe} notes push \"${{1:-origin}}\" >/dev/null 2>&1 || true\nfi\n{HOOK_END}"
    )
}

/// Remove the managed block from a hook body. Returns `None` if not present.
fn excise_managed_block(content: &str) -> Option<String> {
    let begin = content.lines().position(|l| l.trim() == HOOK_BEGIN)?;
    let end_rel = content
        .lines()
        .skip(begin)
        .position(|l| l.trim() == HOOK_END)?;
    let end = begin + end_rel;
    let kept: Vec<&str> = content
        .lines()
        .enumerate()
        .filter(|(i, _)| *i < begin || *i > end)
        .map(|(_, l)| l)
        .collect();
    Some(kept.join("\n"))
}

/// Append (or replace) Tellur's managed block in a hook body, preserving any
/// pre-existing user hook content.
fn splice_managed_block(existing: &str, block: &str) -> String {
    let base = excise_managed_block(existing).unwrap_or_else(|| existing.to_string());
    let trimmed = base.trim_end();
    if trimmed.is_empty() {
        format!("#!/bin/sh\n{block}\n")
    } else {
        format!("{trimmed}\n\n{block}\n")
    }
}

fn install_managed_hook(hooks_dir: &Path, name: &str, block: &str) -> Result<()> {
    let path = hooks_dir.join(name);
    let new_content = match std::fs::read_to_string(&path) {
        Ok(existing) if !existing.trim().is_empty() => {
            if let Some(first) = existing.lines().next()
                && first.starts_with("#!")
                && !first.contains("sh")
            {
                bail!(
                    "existing {name} hook uses a non-shell interpreter ({first}); \
                     add Tellur's commands to it manually"
                );
            }
            splice_managed_block(&existing, block)
        }
        _ => format!("#!/bin/sh\n{block}\n"),
    };
    std::fs::write(&path, new_content)
        .with_context(|| format!("failed to write hook {}", path.display()))?;
    set_executable(&path)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("failed to chmod {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn connect_remove(storage: &RepoStorage, remote: &str) -> Result<()> {
    let hooks_dir = git_hooks_dir(&storage.root)?;
    for name in ["post-commit", "pre-push"] {
        let path = hooks_dir.join(name);
        let Ok(existing) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(stripped) = excise_managed_block(&existing) else {
            continue;
        };
        let trimmed = stripped.trim();
        if trimmed.is_empty() || trimmed == "#!/bin/sh" {
            std::fs::remove_file(&path)?;
            println!("✓ Removed {name} hook");
        } else {
            std::fs::write(&path, format!("{}\n", stripped.trim_end()))?;
            println!("✓ Removed Tellur block from {name} hook (kept your hook)");
        }
    }

    let notes_ref = tellur_core::notes::GIT_AI_NOTES_REF;
    let fetch_key = format!("remote.{remote}.fetch");
    // `+` `/` `:` are all literal in git's basic-regex value pattern.
    git_try(
        &storage.root,
        &[
            "config",
            "--unset-all",
            &fetch_key,
            &format!("{notes_ref}:{notes_ref}"),
        ],
    );
    git_try(
        &storage.root,
        &["config", "--unset-all", "notes.rewriteRef", notes_ref],
    );
    println!("✓ Removed notes fetch config for '{remote}'");

    if let Some(path) = service::remove(&storage.root)? {
        println!("✓ Removed background push service ({})", path.display());
    }

    println!("\nDisconnected. Editor/agent integrations and hub credentials are untouched");
    println!("(use `tellur setup uninstall` and `tellur logout` for those).");
    Ok(())
}

pub(crate) fn connect_status(storage: &RepoStorage, remote: &str) -> Result<()> {
    let hooks_dir = git_hooks_dir(&storage.root)?;
    let mark = |present: bool| if present { "✓" } else { "✗" };

    let hook_installed = |name: &str| {
        std::fs::read_to_string(hooks_dir.join(name))
            .map(|c| c.contains(HOOK_BEGIN))
            .unwrap_or(false)
    };
    let notes_fetch = git_config_get_all(&storage.root, &format!("remote.{remote}.fetch"))
        .iter()
        .any(|v| v.contains(tellur_core::notes::GIT_AI_NOTES_REF));
    let logged_in = hub::Credentials::load()
        .map(|c| !c.hosts.is_empty())
        .unwrap_or(false);

    println!("tellur connect status — {}", storage.root.display());
    println!("  {} hub login", mark(logged_in));
    println!(
        "  {} post-commit hook (refresh notes)",
        mark(hook_installed("post-commit"))
    );
    println!(
        "  {} pre-push hook (push events + notes)",
        mark(hook_installed("pre-push"))
    );
    println!("  {} notes fetch config for '{remote}'", mark(notes_fetch));
    match service::status(&storage.root) {
        Some(path) => println!("  ✓ background push service ({})", path.display()),
        None => println!("  ✗ background push service (add --background)"),
    }
    Ok(())
}
