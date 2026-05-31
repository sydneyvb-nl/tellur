//! Repository-local storage management
//!
//! Manages the `.tellur/` directory structure within a Git repository.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Default directory name for Tellur metadata
pub const TELLUR_DIR: &str = ".tellur";

/// Repository storage layout
pub struct RepoStorage {
    pub root: PathBuf,
    pub tellur_dir: PathBuf,
    pub traces_dir: PathBuf,
    pub index_path: PathBuf,
    pub config_path: PathBuf,
    pub policies_dir: PathBuf,
    pub exports_dir: PathBuf,
}

impl RepoStorage {
    /// Discover the Tellur storage for the current working directory
    pub fn discover() -> Result<Self> {
        let git_root =
            find_git_root(std::env::current_dir()?).context("Not inside a Git repository")?;
        Self::from_git_root(&git_root)
    }

    /// Create storage layout from a known Git root
    pub fn from_git_root(git_root: &Path) -> Result<Self> {
        let tellur_dir = git_root.join(TELLUR_DIR);
        Ok(Self {
            root: git_root.to_path_buf(),
            traces_dir: tellur_dir.join("traces"),
            index_path: tellur_dir.join("index").join("tellur.db"),
            config_path: tellur_dir.join("config.yml"),
            policies_dir: tellur_dir.join("policies"),
            exports_dir: tellur_dir.join("exports"),
            tellur_dir,
        })
    }

    /// Initialize the storage directory structure
    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(&self.tellur_dir)?;
        std::fs::create_dir_all(&self.traces_dir)?;
        let index_parent = self
            .index_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Index path has no parent: {:?}", self.index_path))?;
        std::fs::create_dir_all(index_parent)?;
        std::fs::create_dir_all(&self.policies_dir)?;
        std::fs::create_dir_all(&self.exports_dir)?;

        if !self.config_path.exists() {
            std::fs::write(&self.config_path, DEFAULT_CONFIG)?;
        }

        if !self.policies_dir.join("default.yml").exists() {
            std::fs::write(self.policies_dir.join("default.yml"), DEFAULT_POLICY)?;
        }

        Ok(())
    }

    /// Check if Tellur is initialized in this repository
    pub fn is_initialized(&self) -> bool {
        self.config_path.exists()
    }
}

/// Find the Git root directory by walking up from the given path
fn find_git_root(start: PathBuf) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
}

const DEFAULT_CONFIG: &str = r#"# Tellur Configuration
version: 1

storage:
  mode: local
  traces_dir: traces
  index_type: sqlite

redaction:
  mode: automatic
  hash_prompts: true
  store_prompt_excerpt: false
  redact_patterns:
    - "(?i)api[_-]?key\\s*=\\s*.+"
    - "(?i)password\\s*=\\s*.+"
    - "(?i)token\\s*=\\s*.+"

retention:
  keep_days: 90
  keep_release_related: true
  delete_prompts_after_days: 30

attribution:
  confidence_threshold: 0.7
  range_fingerprint_window: 5
  semantic_anchors: true
"#;

const DEFAULT_POLICY: &str = r#"# Tellur Default Policy
version: 1

sensitive_paths:
  - path: "src/auth/**"
    tags: ["auth", "security-sensitive"]
    require_human_review: true
    require_tests: true

  - path: "**/.env*"
    tags: ["secrets"]
    block_ai_read: true

  - path: "infra/**"
    tags: ["infrastructure"]
    block_ai_automerge: true

rules: []
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let git_root = tmp.path();
        std::fs::create_dir_all(git_root.join(".git")).unwrap();

        let storage = RepoStorage::from_git_root(git_root).unwrap();
        assert!(!storage.is_initialized());

        storage.init().unwrap();
        assert!(storage.is_initialized());
        assert!(storage.config_path.exists());
        assert!(storage.traces_dir.exists());
        assert!(storage.policies_dir.join("default.yml").exists());
    }
}
