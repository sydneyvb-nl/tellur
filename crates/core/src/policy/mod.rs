//! Policy engine — evaluate rules against events and attributions
//!
//! Supports YAML-defined policies with rules for sensitive paths,
//! AI behavior restrictions, and review requirements.

use anyhow::{Context, Result};
use std::path::Path;

use crate::glob::glob_match;
use crate::schema::types::*;

/// Policy engine evaluates rules against Tellur data
pub struct PolicyEngine {
    policy: PolicyFile,
}

impl PolicyEngine {
    /// Load a policy from a YAML file
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).context("Failed to read policy file")?;
        let policy: PolicyFile =
            serde_yaml::from_str(&content).context("Failed to parse policy YAML")?;
        Ok(Self { policy })
    }

    /// Create from an existing policy
    pub fn from_policy(policy: PolicyFile) -> Self {
        Self { policy }
    }

    /// Check if a file is in a sensitive path
    pub fn get_sensitive_tags(&self, file_path: &str) -> Vec<String> {
        let mut tags = Vec::new();
        if let Some(ref paths) = self.policy.sensitive_paths {
            for sp in paths {
                if glob_match(&sp.path, file_path) {
                    tags.extend(sp.tags.clone());
                }
            }
        }
        tags.sort();
        tags.dedup();
        tags
    }

    /// Check if a file requires human review
    pub fn requires_human_review(&self, file_path: &str) -> bool {
        if let Some(ref paths) = self.policy.sensitive_paths {
            for sp in paths {
                if glob_match(&sp.path, file_path) && sp.require_human_review.unwrap_or(false) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a file requires tests
    pub fn requires_tests(&self, file_path: &str) -> bool {
        if let Some(ref paths) = self.policy.sensitive_paths {
            for sp in paths {
                if glob_match(&sp.path, file_path) && sp.require_tests.unwrap_or(false) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if AI agents are forbidden from reading this path (e.g. secrets).
    /// Capture skips matching files entirely so their contents are never stored.
    pub fn blocks_ai_read(&self, file_path: &str) -> bool {
        if let Some(ref paths) = self.policy.sensitive_paths {
            for sp in paths {
                if glob_match(&sp.path, file_path) && sp.block_ai_read.unwrap_or(false) {
                    return true;
                }
            }
        }
        false
    }

    /// Evaluate an attribution range against all policy rules
    pub fn evaluate_attribution(
        &self,
        attr: &AttributionRange,
        file_path: &str,
    ) -> Vec<PolicyResult> {
        let mut results = Vec::new();

        // Check sensitive paths
        let tags = self.get_sensitive_tags(file_path);
        if !tags.is_empty() && attr.origin == Origin::Ai {
            let requires_review = self.requires_human_review(file_path);
            let requires_tests = self.requires_tests(file_path);

            if requires_review && attr.reviewer.is_none() {
                results.push(PolicyResult {
                    rule_id: "sensitive-path-review".to_string(),
                    passed: false,
                    severity: RiskLevel::High,
                    message: format!(
                        "AI-generated code in sensitive area ({}) requires human review",
                        tags.join(", ")
                    ),
                    evidence: vec![format!("File: {}", file_path)],
                });
            }

            if requires_tests && !attr.tests_passed {
                results.push(PolicyResult {
                    rule_id: "sensitive-path-tests".to_string(),
                    passed: false,
                    severity: RiskLevel::Medium,
                    message: format!(
                        "AI-generated code in sensitive area ({}) requires passing tests",
                        tags.join(", ")
                    ),
                    evidence: vec![format!("File: {}", file_path)],
                });
            }
        }

        // Evaluate custom rules
        if let Some(ref rules) = self.policy.rules {
            for rule in rules {
                if let Some(result) = self.evaluate_rule(rule, attr, file_path) {
                    results.push(result);
                }
            }
        }

        results
    }

    /// Evaluate a single rule against an attribution
    fn evaluate_rule(
        &self,
        rule: &PolicyRule,
        attr: &AttributionRange,
        _file_path: &str,
    ) -> Option<PolicyResult> {
        let when = &rule.when;

        // Check if the rule applies based on origin
        if let Some(origin) = when.get("attribution.origin").and_then(|v| v.as_str()) {
            let attr_origin = serde_json::to_string(&attr.origin).unwrap_or_default();
            if attr_origin.trim_matches('"') != origin {
                return None;
            }
        }

        // Check line count threshold
        if let Some(threshold) = when
            .get("changed_lines.greater_than")
            .and_then(|v| v.as_u64())
        {
            let changed_lines = (attr.end_line - attr.start_line + 1) as u64;
            if changed_lines <= threshold {
                return None;
            }
        }

        // Rule matches — check requirements
        let mut passed = true;
        let mut evidence = Vec::new();

        if let Some(ref require) = rule.require {
            if require
                .get("tests_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && attr.tests_run.is_empty()
            {
                passed = false;
                evidence.push("No tests were run".to_string());
            }
            if require
                .get("reviewer_from_codeowners")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && attr.reviewer.is_none()
            {
                passed = false;
                evidence.push("No code owner review".to_string());
            }
        }

        Some(PolicyResult {
            rule_id: rule.id.clone(),
            passed,
            severity: if passed {
                RiskLevel::Low
            } else {
                RiskLevel::Medium
            },
            message: rule.description.clone(),
            evidence,
        })
    }

    /// Get the loaded policy
    pub fn policy(&self) -> &PolicyFile {
        &self.policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy() -> PolicyFile {
        PolicyFile {
            version: 1,
            sensitive_paths: Some(vec![
                SensitivePath {
                    path: "src/auth/**".to_string(),
                    tags: vec!["auth".to_string(), "security-sensitive".to_string()],
                    require_human_review: Some(true),
                    require_tests: Some(true),
                    block_ai_automerge: None,
                    block_ai_read: None,
                },
                SensitivePath {
                    path: "infra/**".to_string(),
                    tags: vec!["infrastructure".to_string()],
                    require_human_review: None,
                    require_tests: None,
                    block_ai_automerge: Some(true),
                    block_ai_read: None,
                },
            ]),
            rules: None,
        }
    }

    fn test_range() -> AttributionRange {
        AttributionRange {
            range_id: "rng_test".to_string(),
            start_line: 1,
            end_line: 10,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 1.0,
            state: AttributionState::Exact,
            session_id: "sess_test".to_string(),
            event_ids: vec![],
            agent_id: "claude-code".to_string(),
            model_id: None,
            prompt_hash: None,
            context_set_id: None,
            policy_tags: vec![],
            risk_tags: vec![],
            risk_level: None,
            tests_run: vec![],
            tests_passed: false,
            reviewer: None,
            reviewed_at: None,
        }
    }

    #[test]
    fn test_sensitive_tags() {
        let engine = PolicyEngine::from_policy(test_policy());
        let tags = engine.get_sensitive_tags("src/auth/session.ts");
        assert!(tags.contains(&"auth".to_string()));
        assert!(tags.contains(&"security-sensitive".to_string()));
    }

    #[test]
    fn test_no_tags_for_normal_file() {
        let engine = PolicyEngine::from_policy(test_policy());
        let tags = engine.get_sensitive_tags("src/utils/helpers.ts");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_requires_review() {
        let engine = PolicyEngine::from_policy(test_policy());
        assert!(engine.requires_human_review("src/auth/session.ts"));
        assert!(!engine.requires_human_review("src/utils/helpers.ts"));
    }

    #[test]
    fn test_policy_violation_no_review() {
        let engine = PolicyEngine::from_policy(test_policy());
        let range = test_range();
        let results = engine.evaluate_attribution(&range, "src/auth/session.ts");
        assert!(
            results
                .iter()
                .any(|r| !r.passed && r.rule_id == "sensitive-path-review")
        );
    }

    #[test]
    fn test_policy_pass_with_reviewer() {
        let engine = PolicyEngine::from_policy(test_policy());
        let mut range = test_range();
        range.reviewer = Some("john".to_string());
        range.tests_passed = true;
        range.tests_run = vec!["npm test".to_string()];
        let results = engine.evaluate_attribution(&range, "src/auth/session.ts");
        // Should still have results but they should all pass
        assert!(results.iter().all(|r| r.passed));
    }
}
