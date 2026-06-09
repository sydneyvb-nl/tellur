//! Line-level AI / review statistics, computed from stored attribution.
//!
//! Implements the **review-gap definition** decided in
//! `docs/proposals/TEAM_DASHBOARD_UI.md §12.1`: an AI-attributed range counts as
//! reviewed only when it has an explicit human reviewer **distinct from the
//! producing agent**, a `reviewed_at` timestamp, and — where tests were run —
//! passing tests. Review coverage = reviewed AI lines / total AI lines.
//!
//! Approximation (documented): the spec also requires `reviewed_at` to be *after
//! the latest AI modification* of the range, but the persisted
//! [`AttributionRange`] has no separate per-range modification timestamp. We
//! treat the presence of a `reviewed_at` (with a distinct human reviewer) as the
//! signal; this can be tightened once ranges carry a modification time.

use tellur_core::schema::types::{AttributionRange, FileAttribution, Origin};

/// Aggregate AI / review line counts for a set of files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct ReviewStats {
    /// Total attributed lines (any origin).
    pub total_attributed_lines: u64,
    /// Lines attributed to AI (origin == Ai).
    pub ai_lines: u64,
    /// AI lines that meet the "reviewed" bar.
    pub reviewed_ai_lines: u64,
}

impl ReviewStats {
    /// Review coverage in `0..=1` (reviewed AI lines / AI lines); `None` when
    /// there are no AI lines to review.
    pub fn review_coverage(&self) -> Option<f64> {
        if self.ai_lines == 0 {
            None
        } else {
            Some(self.reviewed_ai_lines as f64 / self.ai_lines as f64)
        }
    }

    /// AI share of attributed lines (`0..=1`); `None` when nothing is attributed.
    pub fn ai_share(&self) -> Option<f64> {
        if self.total_attributed_lines == 0 {
            None
        } else {
            Some(self.ai_lines as f64 / self.total_attributed_lines as f64)
        }
    }
}

/// Number of lines a range spans (inclusive, 1-based); 0 if malformed.
fn range_lines(r: &AttributionRange) -> u64 {
    if r.start_line == 0 || r.end_line < r.start_line {
        0
    } else {
        u64::from(r.end_line - r.start_line + 1)
    }
}

/// Whether an AI range is "reviewed" per the decided definition.
fn is_reviewed(r: &AttributionRange) -> bool {
    let reviewer = match r.reviewer.as_deref() {
        Some(name) if !name.is_empty() => name,
        _ => return false,
    };
    // Distinct human reviewer (not the producing agent).
    if reviewer == r.agent_id {
        return false;
    }
    // Explicit review timestamp.
    if r.reviewed_at.as_deref().unwrap_or("").is_empty() {
        return false;
    }
    // Where tests were run, they must have passed.
    if !r.tests_run.is_empty() && !r.tests_passed {
        return false;
    }
    true
}

/// Compute [`ReviewStats`] over a set of file attributions.
pub fn review_stats(files: &[FileAttribution]) -> ReviewStats {
    let mut stats = ReviewStats::default();
    for file in files {
        for r in &file.ranges {
            let lines = range_lines(r);
            stats.total_attributed_lines += lines;
            if r.origin == Origin::Ai {
                stats.ai_lines += lines;
                if is_reviewed(r) {
                    stats.reviewed_ai_lines += lines;
                }
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use tellur_core::schema::types::{AttributionState, EvidenceStrength};

    #[allow(clippy::too_many_arguments)]
    fn range(
        start: u32,
        end: u32,
        origin: Origin,
        agent: &str,
        reviewer: Option<&str>,
        reviewed_at: Option<&str>,
        tests_run: Vec<String>,
        tests_passed: bool,
    ) -> AttributionRange {
        AttributionRange {
            range_id: "r".into(),
            start_line: start,
            end_line: end,
            origin,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 1.0,
            state: AttributionState::Exact,
            session_id: "s".into(),
            event_ids: vec![],
            agent_id: agent.into(),
            model_id: None,
            prompt_hash: None,
            context_set_id: None,
            policy_tags: vec![],
            risk_tags: vec![],
            risk_level: None,
            tests_run,
            tests_passed,
            reviewer: reviewer.map(str::to_string),
            reviewed_at: reviewed_at.map(str::to_string),
        }
    }

    fn file(ranges: Vec<AttributionRange>) -> FileAttribution {
        FileAttribution {
            schema: "tellur.attribution.v1".into(),
            file_path: "f.rs".into(),
            git_blob_sha: "sha".into(),
            ranges,
            updated_at: "2026-06-08T00:00:00Z".into(),
        }
    }

    #[test]
    fn counts_ai_and_total_lines() {
        let f = file(vec![
            range(1, 10, Origin::Ai, "claude", None, None, vec![], false), // 10 AI, unreviewed
            range(11, 15, Origin::Human, "human", None, None, vec![], false), // 5 human
        ]);
        let s = review_stats(&[f]);
        assert_eq!(s.total_attributed_lines, 15);
        assert_eq!(s.ai_lines, 10);
        assert_eq!(s.reviewed_ai_lines, 0);
        assert_eq!(s.ai_share(), Some(10.0 / 15.0));
        assert_eq!(s.review_coverage(), Some(0.0));
    }

    #[test]
    fn reviewed_requires_distinct_human_reviewer_and_timestamp() {
        // Reviewed by a human, with timestamp, no tests → reviewed.
        let ok = range(
            1,
            4,
            Origin::Ai,
            "claude",
            Some("alice"),
            Some("2026-06-08T01:00:00Z"),
            vec![],
            false,
        );
        // Reviewer == producing agent → not reviewed.
        let self_review = range(
            1,
            4,
            Origin::Ai,
            "claude",
            Some("claude"),
            Some("2026-06-08T01:00:00Z"),
            vec![],
            false,
        );
        // Reviewer but no timestamp → not reviewed.
        let no_ts = range(
            1,
            4,
            Origin::Ai,
            "claude",
            Some("alice"),
            None,
            vec![],
            false,
        );
        assert_eq!(review_stats(&[file(vec![ok])]).reviewed_ai_lines, 4);
        assert_eq!(
            review_stats(&[file(vec![self_review])]).reviewed_ai_lines,
            0
        );
        assert_eq!(review_stats(&[file(vec![no_ts])]).reviewed_ai_lines, 0);
    }

    #[test]
    fn failing_required_tests_block_review() {
        let failed = range(
            1,
            4,
            Origin::Ai,
            "claude",
            Some("alice"),
            Some("2026-06-08T01:00:00Z"),
            vec!["unit".into()],
            false,
        );
        let passed = range(
            1,
            4,
            Origin::Ai,
            "claude",
            Some("alice"),
            Some("2026-06-08T01:00:00Z"),
            vec!["unit".into()],
            true,
        );
        assert_eq!(review_stats(&[file(vec![failed])]).reviewed_ai_lines, 0);
        assert_eq!(review_stats(&[file(vec![passed])]).reviewed_ai_lines, 4);
    }

    #[test]
    fn malformed_ranges_count_zero_lines() {
        let bad = range(0, 0, Origin::Ai, "claude", None, None, vec![], false);
        let s = review_stats(&[file(vec![bad])]);
        assert_eq!(s.ai_lines, 0);
        assert_eq!(s.ai_share(), None);
        assert_eq!(s.review_coverage(), None);
    }
}
