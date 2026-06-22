//! Thin wrappers around the `git` CLI shared by the notes and connect commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Best-effort `git` invocation whose failure is ignored (used for idempotent
/// teardown of config that may or may not exist).
pub(crate) fn git_try(repo_root: &Path, args: &[&str]) {
    let _ = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output();
}

/// All configured values for a git config key (empty if unset/unreadable).
pub(crate) fn git_config_get_all(repo_root: &Path, key: &str) -> Vec<String> {
    std::process::Command::new("git")
        .args(["config", "--get-all", key])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a git remote of this name is configured in the repo.
pub(crate) fn git_remote_exists(repo_root: &Path, remote: &str) -> bool {
    git_output(repo_root, &["remote"])
        .map(|out| out.lines().any(|l| l.trim() == remote))
        .unwrap_or(false)
}

/// Resolve this repo's hooks directory (honours `core.hooksPath` and worktrees).
pub(crate) fn git_hooks_dir(repo_root: &Path) -> Result<PathBuf> {
    let raw = git_output(repo_root, &["rev-parse", "--git-path", "hooks"])?;
    let p = PathBuf::from(raw.trim());
    let dir = if p.is_absolute() {
        p
    } else {
        repo_root.join(p)
    };
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create hooks dir {}", dir.display()))?;
    Ok(dir)
}

pub(crate) fn resolve_commit(repo_root: &Path, commit: &str) -> Result<String> {
    let output = git_output(repo_root, &["rev-parse", commit])?;
    Ok(output.trim().to_string())
}

pub(crate) fn write_git_note(
    repo_root: &Path,
    notes_ref: &str,
    commit: &str,
    note: &str,
) -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "tellur-note-{}-{}.txt",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&path, note)?;
    let result = run_git(
        repo_root,
        &[
            "notes",
            "--ref",
            notes_ref,
            "add",
            "-f",
            "-F",
            &path.to_string_lossy(),
            commit,
        ],
    );
    let _ = std::fs::remove_file(path);
    result
}

pub(crate) fn read_git_note(repo_root: &Path, notes_ref: &str, commit: &str) -> Result<String> {
    git_output(repo_root, &["notes", "--ref", notes_ref, "show", commit])
}

pub(crate) fn run_git(repo_root: &Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(crate) fn git_output(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}
