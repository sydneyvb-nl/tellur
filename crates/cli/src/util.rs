//! Cross-cutting helpers shared by multiple command modules: the current actor,
//! policy loading, prompt redaction, retention/index maintenance, and small
//! path/shell utilities.

use std::path::PathBuf;

use anyhow::{Context, Result};

use tellur_core::policy::PolicyEngine;
use tellur_core::schema::types::{Actor, EventActor};
use tellur_core::storage::{RepoStorage, TraceIndex};

/// Build an Actor for the current OS/git user.
pub(crate) fn current_actor() -> Actor {
    let name = std::env::var("GIT_AUTHOR_NAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string());
    Actor {
        name,
        email: std::env::var("GIT_AUTHOR_EMAIL").ok(),
        email_hash: None,
        actor_type: EventActor::Human,
    }
}

/// Load the first policy engine from the policies dir, if any.
pub(crate) fn load_policy(storage: &RepoStorage) -> Option<PolicyEngine> {
    let path = storage.policies_dir.join("default.yml");
    PolicyEngine::load_from_file(&path).ok()
}

/// Read the `redaction:` block from `.tellur/config.yml`, falling back to the
/// defaults when it is absent or unparseable.
pub(crate) fn read_redaction_config(
    storage: &RepoStorage,
) -> tellur_core::redaction::RedactionConfig {
    std::fs::read_to_string(&storage.config_path)
        .ok()
        .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
        .and_then(|v| v.get("redaction").cloned())
        .and_then(|r| serde_yaml::from_value(r).ok())
        .unwrap_or_default()
}

/// A redaction engine for prompt excerpts built from the repo's own config — or
/// `None` when the repo hasn't opted into `store_prompt_excerpt`. The repo's
/// project-specific `redact_patterns` are honoured, **and** the built-in default
/// secret patterns are always added, so a custom secret in a prompt is stripped
/// before it is ever stored or pushed.
pub(crate) fn prompt_redaction_engine(
    storage: &RepoStorage,
) -> Option<tellur_core::redaction::RedactionEngine> {
    use tellur_core::redaction::{RedactionConfig, RedactionEngine};
    let mut cfg = read_redaction_config(storage);
    if !cfg.store_prompt_excerpt {
        return None;
    }
    for p in RedactionConfig::default().redact_patterns {
        if !cfg.redact_patterns.contains(&p) {
            cfg.redact_patterns.push(p);
        }
    }
    Some(RedactionEngine::new(cfg))
}

/// Maximum characters kept of a prompt excerpt (the rest is elided).
pub(crate) const PROMPT_EXCERPT_MAX: usize = 600;

/// Build a secret-redacted, length-bounded excerpt of a prompt for storage.
/// Secrets are stripped first (using the repo's redaction rules), then it is
/// truncated on a char boundary with an ellipsis so it stays a compact preview.
pub(crate) fn prompt_excerpt(
    engine: &tellur_core::redaction::RedactionEngine,
    text: &str,
) -> String {
    let cleaned = engine
        .scan_and_redact(text)
        .redacted_content
        .unwrap_or_else(|| text.to_string());
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= PROMPT_EXCERPT_MAX {
        return cleaned.to_string();
    }
    let truncated: String = cleaned.chars().take(PROMPT_EXCERPT_MAX).collect();
    format!("{truncated}…")
}

/// Read `retention.keep_days` from `.tellur/config.yml`.
pub(crate) fn read_retention_days(storage: &RepoStorage) -> Option<u32> {
    let content = std::fs::read_to_string(&storage.config_path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    value
        .get("retention")
        .and_then(|r| r.get("keep_days"))
        .and_then(|d| d.as_u64())
        .map(|d| d as u32)
}

/// Rebuild the SQLite index from the JSONL logs (events table only).
pub(crate) fn rebuild_index(storage: &RepoStorage) -> Result<()> {
    // Start a fresh database file.
    if storage.index_path.exists() {
        std::fs::remove_file(&storage.index_path)?;
    }
    let index = TraceIndex::open(&storage.index_path)?;
    let events = tellur_core::storage::read_events(&storage.traces_dir)?;
    for event in &events {
        index.index_event(event)?;
    }
    Ok(())
}

/// Replace every non-alphanumeric ASCII character with `_`, producing a token
/// safe to embed in identifiers (attribution range ids, event type suffixes).
pub(crate) fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Absolute path of the running `tellur` executable.
pub(crate) fn tellur_executable_path() -> Result<PathBuf> {
    std::env::current_exe().context("failed to resolve tellur executable path")
}

/// Quote a value for safe inclusion in a generated `/bin/sh` command line.
pub(crate) fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_excerpt_redacts_secrets_and_truncates() {
        use tellur_core::redaction::RedactionEngine;
        let engine = RedactionEngine::default_engine();
        // Default secret patterns are stripped from the stored preview.
        let red = prompt_excerpt(
            &engine,
            "deploy with token=ghp_0123456789012345678901234567890123456789",
        );
        assert!(!red.contains("ghp_0123456789"), "secret must be redacted");
        // Short prompts pass through (trimmed).
        assert_eq!(
            prompt_excerpt(&engine, "  refactor the parser  "),
            "refactor the parser"
        );
        // Long prompts are truncated with an ellipsis.
        let long = "a".repeat(PROMPT_EXCERPT_MAX + 50);
        let ex = prompt_excerpt(&engine, &long);
        assert!(ex.ends_with('…'));
        assert_eq!(ex.chars().count(), PROMPT_EXCERPT_MAX + 1);
    }

    #[test]
    fn prompt_excerpt_honours_repo_custom_redact_patterns() {
        use tellur_core::redaction::{RedactionConfig, RedactionEngine};
        // A project-specific pattern (not in the defaults) must still be applied.
        let cfg = RedactionConfig {
            redact_patterns: vec![r"ACME-[0-9]{4}".to_string()],
            ..RedactionConfig::default()
        };
        let engine = RedactionEngine::new(cfg);
        let red = prompt_excerpt(&engine, "the deploy key is ACME-4242 keep it safe");
        assert!(!red.contains("ACME-4242"), "custom secret must be redacted");
    }
}
