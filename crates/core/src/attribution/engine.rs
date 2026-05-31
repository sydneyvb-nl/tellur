//! Line attribution engine
//!
//! Maps code ranges to origin information, surviving line movement,
//! small edits, formatting, and rebases through layered attribution.

use anyhow::Result;

use crate::schema::types::*;

/// Attribution engine — maps file changes to AI/human origin
pub struct AttributionEngine;

impl AttributionEngine {
    /// Create a new attribution engine
    pub fn new() -> Self {
        Self
    }

    /// Attribute a patch to a session
    ///
    /// Takes a diff (unified format) and creates attribution ranges
    /// for the changed lines, linked to the session that produced them.
    pub fn attribute_patch(
        &self,
        session_id: &str,
        agent_id: &str,
        _file_path: &str,
        _blob_sha_before: &str,
        _blob_sha_after: &str,
        unified_diff: &str,
        model_id: Option<&str>,
        prompt_hash: Option<&str>,
    ) -> Result<Vec<AttributionRange>> {
        let ranges = parse_diff_ranges(unified_diff);
        let mut attributions = Vec::new();

        for (start, end) in ranges {
            let range = AttributionRange {
                range_id: crate::schema::ids::generate_range_id(),
                start_line: start,
                end_line: end,
                origin: Origin::Ai,
                evidence_strength: EvidenceStrength::Recorded,
                confidence: 1.0,
                state: AttributionState::Exact,
                session_id: session_id.to_string(),
                event_ids: Vec::new(),
                agent_id: agent_id.to_string(),
                model_id: model_id.map(|s| s.to_string()),
                prompt_hash: prompt_hash.map(|s| s.to_string()),
                context_set_id: None,
                policy_tags: Vec::new(),
                risk_tags: Vec::new(),
                risk_level: None,
                tests_run: Vec::new(),
                tests_passed: false,
                reviewer: None,
                reviewed_at: None,
            };
            attributions.push(range);
        }

        Ok(attributions)
    }

    /// Compute confidence degradation based on how much the code has changed
    /// since the original attribution
    pub fn compute_confidence(
        &self,
        original: &AttributionRange,
        current_lines: u32,
        matched_lines: u32,
    ) -> f64 {
        if current_lines == 0 {
            return 0.0;
        }
        let line_ratio = matched_lines as f64 / current_lines as f64;
        let base = original.confidence;
        (base * line_ratio).min(1.0).max(0.0)
    }

    /// Determine the attribution state based on how ranges have changed
    pub fn determine_state(
        &self,
        original: &AttributionRange,
        new_start: u32,
        new_end: u32,
        fingerprint_match: bool,
    ) -> AttributionState {
        if original.start_line == new_start && original.end_line == new_end {
            if fingerprint_match {
                AttributionState::Exact
            } else {
                AttributionState::Modified
            }
        } else if fingerprint_match {
            AttributionState::Moved
        } else {
            AttributionState::Modified
        }
    }
}

/// Parse unified diff to extract changed line ranges (new file line numbers)
fn parse_diff_ranges(diff: &str) -> Vec<(u32, u32)> {
    let mut ranges: Vec<(u32, u32)> = Vec::new();

    for line in diff.lines() {
        if let Some(hunk) = line.strip_prefix("@@") {
            // Parse @@ -a,b +c,d @@
            if let Some(rest) = hunk.split('@').next() {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    // We want the + part (new file lines)
                    if let Some(plus_part) = parts.iter().find(|p| p.starts_with('+')) {
                        let plus_str = plus_part.trim_start_matches('+');
                        let start: u32 = plus_str
                            .split(',')
                            .next()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        let count: u32 = plus_str
                            .split(',')
                            .nth(1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1);
                        if count > 0 {
                            ranges.push((start, start + count.saturating_sub(1)));
                        }
                    }
                }
            }
        }
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_diff_ranges() {
        let diff = "@@ -10,5 +20,8 @@\n+new line\n context\n+another new";
        let ranges = parse_diff_ranges(diff);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (20, 27)); // start=20, count=8, end=20+8-1=27
    }

    #[test]
    fn test_attribute_patch() {
        let engine = AttributionEngine::new();
        let diff = "@@ -1,3 +10,5 @@\n+line1\n+line2\n context\n+line3";
        let ranges = engine
            .attribute_patch(
                "sess_test",
                "claude-code",
                "src/main.rs",
                "abc",
                "def",
                diff,
                Some("anthropic:claude-opus-4.7"),
                Some("sha256:abc"),
            )
            .unwrap();

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].origin, Origin::Ai);
        assert_eq!(ranges[0].evidence_strength, EvidenceStrength::Recorded);
        assert_eq!(ranges[0].confidence, 1.0);
        assert_eq!(ranges[0].state, AttributionState::Exact);
        assert_eq!(ranges[0].start_line, 10);
        assert_eq!(ranges[0].end_line, 14);
    }

    #[test]
    fn test_confidence_degradation() {
        let engine = AttributionEngine::new();
        let range = AttributionRange {
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
        };

        let confidence = engine.compute_confidence(&range, 10, 8);
        assert!((confidence - 0.8).abs() < 0.01);

        let confidence_zero = engine.compute_confidence(&range, 10, 0);
        assert!((confidence_zero - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_determine_state_exact() {
        let engine = AttributionEngine::new();
        let range = AttributionRange {
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
        };

        let state = engine.determine_state(&range, 1, 10, true);
        assert_eq!(state, AttributionState::Exact);

        let state_moved = engine.determine_state(&range, 5, 14, true);
        assert_eq!(state_moved, AttributionState::Moved);

        let state_modified = engine.determine_state(&range, 5, 14, false);
        assert_eq!(state_modified, AttributionState::Modified);
    }
}
