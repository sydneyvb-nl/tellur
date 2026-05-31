//! Report generation module

pub mod pr_report;

pub use pr_report::PRReportGenerator;

use anyhow::Result;

use crate::policy::PolicyEngine;
use crate::schema::types::{FileAttribution, PolicyResult, PRReport};
use crate::storage::{RepoStorage, TraceIndex};

/// Build a PR risk report for a repository by combining the working-tree diff,
/// indexed attribution data, and all policy files. Shared by the CLI
/// (`tracegit pr-report`) and the MCP server so they cannot drift.
pub fn build_repo_pr_report(storage: &RepoStorage, base: &str, head: &str) -> Result<PRReport> {
    let changes = crate::storage::capture_git_diff(&storage.root).unwrap_or_default();
    let index = TraceIndex::open(&storage.index_path)?;

    // Gather indexed attribution ranges for every changed file.
    let mut file_attrs: Vec<FileAttribution> = Vec::new();
    let mut all_ranges: Vec<(String, crate::schema::types::AttributionRange)> = Vec::new();
    for change in &changes {
        let attrs = index.get_file_attributions(&change.path).unwrap_or_default();
        if attrs.is_empty() {
            continue;
        }
        let mut ranges = Vec::new();
        let mut blob = String::new();
        for (blob_sha, range) in attrs {
            all_ranges.push((change.path.clone(), range.clone()));
            ranges.push(range);
            blob = blob_sha;
        }
        file_attrs.push(FileAttribution {
            schema: "tracegit.attribution.v1".to_string(),
            file_path: change.path.clone(),
            git_blob_sha: blob,
            ranges,
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    // Evaluate every policy file against every range.
    let mut policy_results: Vec<PolicyResult> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&storage.policies_dir) {
        for entry in rd.flatten() {
            if entry.path().extension().is_some_and(|e| e == "yml" || e == "yaml")
                && let Ok(engine) = PolicyEngine::load_from_file(&entry.path()) {
                    for (file_path, range) in &all_ranges {
                        policy_results.extend(engine.evaluate_attribution(range, file_path));
                    }
                }
        }
    }

    Ok(PRReportGenerator::generate(
        base,
        head,
        &file_attrs,
        &policy_results,
        Vec::new(),
        Vec::new(),
    ))
}
