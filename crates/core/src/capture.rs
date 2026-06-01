//! Capture pipeline — turns working-tree changes into events + attribution.
//!
//! This is the glue that makes `tellur watch`, `tellur event --file`, and
//! adapter imports actually populate the index so that `explain`, `blame`, and
//! `pr-report` have data to work with.
//!
//! Flow for each changed file:
//! 1. skip ignored paths (`.git`, `.tellur`, `node_modules`, build dirs);
//! 2. if policy marks the path `block_ai_read` (e.g. secrets), skip it entirely
//!    — its contents are never read or stored;
//! 3. write a `file.write`/`file.delete` event (metadata only — never the diff
//!    body, so secrets in changed lines are not persisted);
//! 4. parse changed line ranges from the diff and attribute them with the
//!    caller-supplied origin/evidence/confidence;
//! 5. tag ranges with policy/risk tags and write them to the index.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::attribution::engine::AttributionEngine;
use crate::policy::PolicyEngine;
use crate::schema::types::{EvidenceStrength, Origin};
use crate::storage::file_watcher::{FileChangeType, capture_git_diff, should_track};
use crate::storage::{EventWriter, RepoStorage, TraceIndex};

/// How captured changes should be attributed.
#[derive(Debug, Clone)]
pub struct CaptureContext {
    pub session_id: String,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub prompt_hash: Option<String>,
    pub origin: Origin,
    pub evidence_strength: EvidenceStrength,
    pub confidence: f64,
}

impl CaptureContext {
    /// Context for changes recorded directly from an AI tool (hook/transcript):
    /// strong evidence, AI origin, full confidence.
    pub fn recorded_ai(session_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            model_id: None,
            prompt_hash: None,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 1.0,
        }
    }

    /// Context for changes observed by `watch` during an AI-assisted session.
    /// `watch` cannot *prove* a change is AI-authored (it only sees the file
    /// system), so it is attributed to AI with `Inferred` evidence and reduced
    /// confidence — the `evidence_strength`/`confidence` fields communicate that
    /// this is weaker than a recorded hook/transcript capture.
    pub fn inferred_watch(session_id: impl Into<String>) -> Self {
        Self::inferred_watch_with_metadata(session_id, "watch", None)
    }

    pub fn inferred_watch_with_metadata(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        model_id: Option<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            model_id,
            prompt_hash: None,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Inferred,
            confidence: 0.6,
        }
    }
}

/// Result of a capture pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptureSummary {
    pub files_captured: usize,
    pub ranges_attributed: usize,
    pub skipped_blocked: Vec<String>,
}

/// Capture the current working-tree diff into events and attribution ranges.
pub fn capture_working_changes(
    storage: &RepoStorage,
    writer: &mut EventWriter,
    index: &TraceIndex,
    policy: Option<&PolicyEngine>,
    ctx: &CaptureContext,
) -> Result<CaptureSummary> {
    capture_working_changes_inner(storage, writer, index, policy, ctx, None)
}

/// Capture only the changed files listed in `paths`. Used by tool hooks that
/// identify the exact file touched by an agent tool call.
pub fn capture_working_changes_for_paths(
    storage: &RepoStorage,
    writer: &mut EventWriter,
    index: &TraceIndex,
    policy: Option<&PolicyEngine>,
    ctx: &CaptureContext,
    paths: &[String],
) -> Result<CaptureSummary> {
    let normalized = paths
        .iter()
        .filter_map(|path| normalize_capture_path(&storage.root, path))
        .collect::<Vec<_>>();
    capture_working_changes_inner(storage, writer, index, policy, ctx, Some(&normalized))
}

fn capture_working_changes_inner(
    storage: &RepoStorage,
    writer: &mut EventWriter,
    index: &TraceIndex,
    policy: Option<&PolicyEngine>,
    ctx: &CaptureContext,
    only_paths: Option<&[String]>,
) -> Result<CaptureSummary> {
    let engine = AttributionEngine::new();
    let mut summary = CaptureSummary::default();

    let changes = capture_git_diff(&storage.root)?;
    for change in changes {
        if let Some(only_paths) = only_paths
            && !only_paths.iter().any(|path| path == &change.path)
        {
            continue;
        }
        let abs = storage.root.join(&change.path);
        if !should_track(&abs, &storage.root) {
            continue;
        }

        // Never read or store contents of paths the policy forbids AI to read.
        if let Some(p) = policy
            && p.blocks_ai_read(&change.path)
        {
            summary.skipped_blocked.push(change.path.clone());
            continue;
        }

        let event_type = match change.change_type {
            FileChangeType::Deleted => "file.delete",
            _ => "file.write",
        };

        // Metadata only — the diff body is intentionally not persisted so that
        // secrets in changed lines never reach the event log.
        let payload = serde_json::json!({
            "file_path": change.path,
            "change_type": change.change_type,
            "blob_sha_before": change.blob_sha_before,
            "blob_sha_after": change.blob_sha_after,
        });

        let actor = match ctx.origin {
            Origin::Ai => "agent",
            Origin::Human => "human",
            _ => "unknown",
        };

        let event = writer.write_event(&ctx.session_id, event_type, actor, payload, None)?;
        index.index_event(&event)?;
        summary.files_captured += 1;

        // Attribute changed line ranges (skip pure deletes — nothing remains).
        if change.change_type == FileChangeType::Deleted {
            continue;
        }
        let Some(diff) = change.diff.as_deref() else {
            continue;
        };
        if diff.trim().is_empty() {
            continue;
        }

        let mut ranges = engine.attribute_patch(
            &ctx.session_id,
            &ctx.agent_id,
            diff,
            ctx.origin.clone(),
            ctx.evidence_strength.clone(),
            ctx.confidence,
            ctx.model_id.as_deref(),
            ctx.prompt_hash.as_deref(),
            std::slice::from_ref(&event.id),
        )?;

        // Apply policy/risk tags from sensitive-path rules.
        if let Some(p) = policy {
            let tags = p.get_sensitive_tags(&change.path);
            if !tags.is_empty() {
                for r in ranges.iter_mut() {
                    r.policy_tags = tags.clone();
                    r.risk_tags = tags.clone();
                }
            }
        }

        let blob_sha = change
            .blob_sha_after
            .clone()
            .or_else(|| change.blob_sha_before.clone())
            .unwrap_or_default();
        let updated_at = change.timestamp.clone();

        // Replace prior ranges for this file so captures don't accumulate.
        index.clear_file_attributions(&change.path)?;
        for r in &ranges {
            index.index_attribution(r, &change.path, &blob_sha, &updated_at)?;
            summary.ranges_attributed += 1;
        }
    }

    Ok(summary)
}

fn normalize_capture_path(root: &std::path::Path, path: &str) -> Option<String> {
    let path = std::path::Path::new(path);
    let rel = if path.is_absolute() {
        path.strip_prefix(root).ok()?
    } else {
        path
    };
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(repo: &std::path::Path, args: &[&str]) {
        Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap();
    }

    #[test]
    fn test_capture_attributes_changes_end_to_end() {
        // Skip gracefully if git is unavailable in the test environment.
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "t@t.dev"]);
        git(root, &["config", "user.name", "T"]);
        std::fs::write(root.join("app.rs"), "fn main() {}\n").unwrap();
        git(root, &["add", "app.rs"]);
        git(root, &["commit", "-m", "init"]);

        // Modify the file (the "AI" edit).
        std::fs::write(
            root.join("app.rs"),
            "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
        )
        .unwrap();

        let storage = RepoStorage::from_git_root(root).unwrap();
        storage.init().unwrap();
        let index = TraceIndex::open(&storage.index_path).unwrap();
        let mut writer = EventWriter::new(&storage.traces_dir);
        writer.open().unwrap();

        let ctx = CaptureContext::recorded_ai("sess_test", "claude-code");
        let summary = capture_working_changes(&storage, &mut writer, &index, None, &ctx).unwrap();
        writer.close();

        assert_eq!(summary.files_captured, 1);
        assert!(summary.ranges_attributed >= 1);

        let attrs = index.get_file_attributions("app.rs").unwrap();
        assert!(!attrs.is_empty(), "attribution should be indexed");
        assert_eq!(attrs[0].1.origin, Origin::Ai);
        assert_eq!(attrs[0].1.agent_id, "claude-code");
    }

    #[test]
    fn test_inferred_watch_accepts_agent_and_model_metadata() {
        let ctx = CaptureContext::inferred_watch_with_metadata(
            "sess_vscode",
            "vscode-copilot",
            Some("openai:gpt-5".to_string()),
        );

        assert_eq!(ctx.session_id, "sess_vscode");
        assert_eq!(ctx.agent_id, "vscode-copilot");
        assert_eq!(ctx.model_id.as_deref(), Some("openai:gpt-5"));
        assert_eq!(ctx.origin, Origin::Ai);
        assert_eq!(ctx.evidence_strength, EvidenceStrength::Inferred);
        assert_eq!(ctx.confidence, 0.6);
    }

    #[test]
    fn test_capture_filtered_paths_only_attributes_matching_file() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "t@t.dev"]);
        git(root, &["config", "user.name", "T"]);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        git(root, &["add", "a.rs", "b.rs"]);
        git(root, &["commit", "-m", "init"]);

        std::fs::write(root.join("a.rs"), "fn a() {\n    let x = 1;\n}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {\n    let y = 2;\n}\n").unwrap();

        let storage = RepoStorage::from_git_root(root).unwrap();
        storage.init().unwrap();
        let index = TraceIndex::open(&storage.index_path).unwrap();
        let mut writer = EventWriter::new(&storage.traces_dir);
        writer.open().unwrap();
        let ctx = CaptureContext::recorded_ai("sess_filter", "claude-code");

        let summary = capture_working_changes_for_paths(
            &storage,
            &mut writer,
            &index,
            None,
            &ctx,
            &["a.rs".to_string()],
        )
        .unwrap();
        writer.close();

        assert_eq!(summary.files_captured, 1);
        assert!(!index.get_file_attributions("a.rs").unwrap().is_empty());
        assert!(index.get_file_attributions("b.rs").unwrap().is_empty());
    }

    #[test]
    fn test_block_ai_read_skips_file() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "t@t.dev"]);
        git(root, &["config", "user.name", "T"]);
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-m", "init"]);
        std::fs::write(
            root.join("creds.secret"),
            "API_KEY=sk-abcdefghij1234567890\n",
        )
        .unwrap();

        let storage = RepoStorage::from_git_root(root).unwrap();
        storage.init().unwrap();
        let policy = PolicyEngine::from_policy(crate::schema::types::PolicyFile {
            version: 1,
            sensitive_paths: Some(vec![crate::schema::types::SensitivePath {
                path: "**/*.secret".to_string(),
                tags: vec!["secret".to_string()],
                require_human_review: None,
                require_tests: None,
                block_ai_automerge: None,
                block_ai_read: Some(true),
            }]),
            rules: None,
        });
        let index = TraceIndex::open(&storage.index_path).unwrap();
        let mut writer = EventWriter::new(&storage.traces_dir);
        writer.open().unwrap();
        let ctx = CaptureContext::recorded_ai("s", "claude-code");
        let summary =
            capture_working_changes(&storage, &mut writer, &index, Some(&policy), &ctx).unwrap();
        writer.close();

        assert!(
            summary
                .skipped_blocked
                .iter()
                .any(|p| p.ends_with("creds.secret"))
        );
        assert!(
            index
                .get_file_attributions("creds.secret")
                .unwrap()
                .is_empty()
        );
    }
}
