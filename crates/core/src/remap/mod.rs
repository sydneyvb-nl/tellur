//! Git remapping — preserve attribution across rebase, squash, amend
//!
//! When git history is rewritten, blob SHAs change. This module remaps
//! TraceGit attributions from old blob SHAs to new ones.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A remapping entry: old SHA → new SHA
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemapEntry {
    pub old_sha: String,
    pub new_sha: String,
    pub file_path: String,
}

/// Result of a remap operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemapResult {
    pub remapped: u32,
    pub unchanged: u32,
    pub missing: u32,
    pub entries: Vec<RemapEntry>,
}

/// Build a SHA remap from git diff-tree output (before → after)
pub fn build_remap_from_diff(
    repo_root: &Path,
    old_ref: &str,
    new_ref: &str,
) -> Result<Vec<RemapEntry>> {
    let output = std::process::Command::new("git")
        .args([
            "diff-tree",
            "-r",
            "--no-commit-id",
            old_ref,
            new_ref,
        ])
        .current_dir(repo_root)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        // Format: :old_mode new_mode old_sha new_sha status\tfile_path
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() < 2 { continue; }

        let meta: Vec<&str> = parts[0].trim_start_matches(':').split_whitespace().collect();
        if meta.len() < 4 { continue; }

        let old_sha = meta[2].to_string();
        let new_sha = meta[3].to_string();
        let file_path = parts[1].to_string();

        if old_sha != new_sha {
            entries.push(RemapEntry {
                old_sha,
                new_sha,
                file_path,
            });
        }
    }

    Ok(entries)
}

/// Apply a SHA remap to a map of file → blob_sha
pub fn apply_remap(
    file_shas: &mut HashMap<String, String>,
    remap: &[RemapEntry],
) -> RemapResult {
    let mut remapped = 0u32;
    let mut unchanged = 0u32;
    let mut missing = 0u32;

    let remap_lookup: HashMap<&str, &RemapEntry> = remap.iter()
        .map(|e| (e.old_sha.as_str(), e))
        .collect();

    for (file_path, current_sha) in file_shas.iter_mut() {
        if let Some(entry) = remap_lookup.get(current_sha.as_str()) {
            *current_sha = entry.new_sha.clone();
            remapped += 1;
        } else {
            unchanged += 1;
        }
    }

    // Count entries in remap that didn't match any file
    let matched_shas: std::collections::HashSet<&str> = remap.iter().map(|e| e.old_sha.as_str()).collect();
    let file_sha_set: std::collections::HashSet<&str> = file_shas.values().map(|s| s.as_str()).collect();
    for sha in &matched_shas {
        if !file_sha_set.contains(sha) {
            missing += 1;
        }
    }

    RemapResult {
        remapped,
        unchanged,
        missing,
        entries: remap.to_vec(),
    }
}

/// Detect if a rebase/squash happened by comparing ref SHAs
pub fn detect_rewrite(
    repo_root: &Path,
    ref_before: &str,
    ref_after: &str,
) -> Result<bool> {
    let sha_before = get_ref_sha(repo_root, ref_before)?;
    let sha_after = get_ref_sha(repo_root, ref_after)?;
    Ok(sha_before != sha_after)
}

fn get_ref_sha(repo_root: &Path, git_ref: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", git_ref])
        .current_dir(repo_root)
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_remap_basic() {
        let mut file_shas = HashMap::new();
        file_shas.insert("src/main.rs".to_string(), "old_sha_1".to_string());
        file_shas.insert("src/lib.rs".to_string(), "old_sha_2".to_string());
        file_shas.insert("README.md".to_string(), "unchanged_sha".to_string());

        let remap = vec![
            RemapEntry {
                old_sha: "old_sha_1".to_string(),
                new_sha: "new_sha_1".to_string(),
                file_path: "src/main.rs".to_string(),
            },
        ];

        let result = apply_remap(&mut file_shas, &remap);

        assert_eq!(result.remapped, 1);
        assert_eq!(result.unchanged, 2);
        assert_eq!(file_shas.get("src/main.rs").unwrap(), "new_sha_1");
        assert_eq!(file_shas.get("README.md").unwrap(), "unchanged_sha");
    }

    #[test]
    fn test_apply_remap_empty() {
        let mut file_shas = HashMap::new();
        let result = apply_remap(&mut file_shas, &[]);
        assert_eq!(result.remapped, 0);
        assert_eq!(result.unchanged, 0);
    }

    #[test]
    fn test_remap_entry_serde() {
        let entry = RemapEntry {
            old_sha: "abc".to_string(),
            new_sha: "def".to_string(),
            file_path: "foo.rs".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: RemapEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.old_sha, "abc");
    }
}
