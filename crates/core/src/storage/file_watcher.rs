//! File change capture — filesystem watcher and git diff integration
//!
//! Watches for file changes in a repository and captures them as
//! TraceGit events with before/after blob SHAs.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A captured file change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub change_type: FileChangeType,
    pub blob_sha_before: Option<String>,
    pub blob_sha_after: Option<String>,
    pub diff: Option<String>,
    pub timestamp: String,
}

/// Type of file change
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// Capture the current git diff as file changes
pub fn capture_git_diff(repo_root: &Path) -> Result<Vec<FileChange>> {
    let output = std::process::Command::new("git")
        .args(["diff", "HEAD", "--name-status"])
        .current_dir(repo_root)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut changes = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() < 2 {
            continue;
        }

        let status = parts[0].trim();
        let path = parts[1].trim();

        let change_type = match status.chars().next() {
            Some('A') => FileChangeType::Created,
            Some('D') => FileChangeType::Deleted,
            Some('R') => FileChangeType::Renamed,
            _ => FileChangeType::Modified,
        };

        let blob_sha_after = if change_type == FileChangeType::Deleted {
            None
        } else {
            get_working_blob_sha(repo_root, path)
        };

        changes.push(FileChange {
            path: path.to_string(),
            change_type,
            blob_sha_before: get_blob_sha(repo_root, path, "HEAD"),
            blob_sha_after,
            diff: get_file_diff(repo_root, path).ok(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }

    // `git diff HEAD` omits untracked files, but an AI agent creating a *new*
    // file is exactly what we want to capture. Enumerate untracked, non-ignored
    // files and add them as `Created` with a synthesized whole-file hunk.
    if let Ok(out) = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(repo_root)
        .output()
        && out.status.success() {
            let listing = String::from_utf8_lossy(&out.stdout);
            for path in listing.lines().map(str::trim).filter(|p| !p.is_empty()) {
                let abs = repo_root.join(path);
                let line_count = std::fs::read_to_string(&abs)
                    .map(|c| c.lines().count().max(1))
                    .unwrap_or(1);
                changes.push(FileChange {
                    path: path.to_string(),
                    change_type: FileChangeType::Created,
                    blob_sha_before: None,
                    blob_sha_after: get_working_blob_sha(repo_root, path),
                    diff: Some(format!("@@ -0,0 +1,{} @@\n", line_count)),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

    Ok(changes)
}

/// Get the blob SHA for a file at a given git ref
fn get_blob_sha(repo_root: &Path, file_path: &str, git_ref: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", &format!("{}:{}", git_ref, file_path)])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Get the diff for a specific file
fn get_file_diff(repo_root: &Path, file_path: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "HEAD", "--", file_path])
        .current_dir(repo_root)
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Compute the git blob SHA for a working-tree file (matches `git rev-parse`).
///
/// Uses `git hash-object` so the result is the real git blob object id (the
/// same hash algorithm git uses for `blob_sha_before`), keeping before/after
/// SHAs directly comparable.
pub fn get_working_blob_sha(repo_root: &Path, file_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["hash-object", "--", file_path])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Check if a path should be ignored (gitignore-aware)
pub fn should_track(path: &Path, repo_root: &Path) -> bool {
    // Skip .tracegit directory
    if path.starts_with(repo_root.join(".tracegit")) {
        return false;
    }
    // Skip .git directory
    if path.starts_with(repo_root.join(".git")) {
        return false;
    }
    // Skip node_modules, target, etc.
    let path_str = path.to_string_lossy();
    if path_str.contains("node_modules") || path_str.contains("/target/") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_should_track() {
        let repo = PathBuf::from("/tmp/testrepo");
        assert!(should_track(&repo.join("src/main.rs"), &repo));
        assert!(!should_track(&repo.join(".tracegit/config.yml"), &repo));
        assert!(!should_track(&repo.join(".git/HEAD"), &repo));
        assert!(!should_track(&repo.join("node_modules/foo/bar.js"), &repo));
    }

    #[test]
    fn test_file_change_type_serde() {
        let ct = FileChangeType::Modified;
        let json = serde_json::to_string(&ct).unwrap();
        assert_eq!(json, "\"modified\"");
    }
}
