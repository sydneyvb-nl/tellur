//! PR Risk Report Generator
//!
//! Generates a risk report for a pull request based on AI attribution data,
//! policy violations, sensitive file access, and test evidence.


use crate::schema::types::*;

/// PR Risk Report Generator
pub struct PRReportGenerator;

impl PRReportGenerator {
    /// Generate a PR risk report from attribution data
    pub fn generate(
        base_ref: &str,
        head_ref: &str,
        attributions: &[FileAttribution],
        policy_results: &[PolicyResult],
        commands: Vec<CommandExecution>,
        tests: Vec<TestExecution>,
    ) -> PRReport {
        let mut ai_lines: u64 = 0;
        let mut human_lines: u64 = 0;
        let mut unknown_lines: u64 = 0;
        let mut sensitive_files: Vec<String> = Vec::new();
        let mut unattributed: Vec<String> = Vec::new();
        let mut checklist: Vec<String> = Vec::new();

        for attr in attributions {
            let mut file_has_ai = false;
            for range in &attr.ranges {
                let line_count = (range.end_line - range.start_line + 1) as u64;
                match range.origin {
                    Origin::Ai | Origin::Mixed => {
                        ai_lines += line_count;
                        file_has_ai = true;
                    }
                    Origin::Human => human_lines += line_count,
                    Origin::Unknown => unknown_lines += line_count,
                }

                // Collect risk tags
                if !range.risk_tags.is_empty() {
                    sensitive_files.push(format!("{}: {}", attr.file_path, range.risk_tags.join(", ")));
                }
            }

            if !file_has_ai && attr.ranges.is_empty() {
                unattributed.push(attr.file_path.clone());
            }
        }

        let total = ai_lines + human_lines + unknown_lines;
        let ai_percentage = if total > 0 { (ai_lines as f64 / total as f64) * 100.0 } else { 0.0 };

        // Generate reviewer checklist items
        if ai_percentage > 50.0 {
            checklist.push("⚠ High AI involvement — verify generated code carefully".to_string());
        }
        if !sensitive_files.is_empty() {
            checklist.push("Review sensitive file changes".to_string());
        }
        if tests.is_empty() && ai_lines > 0 {
            checklist.push("⚠ No test evidence for AI-generated code".to_string());
        }
        let violations: Vec<_> = policy_results.iter().filter(|r| !r.passed).collect();
        if !violations.is_empty() {
            checklist.push(format!("Resolve {} policy violation(s)", violations.len()));
        }

        // Determine overall risk
        let overall_risk = Self::compute_risk(ai_percentage, &sensitive_files, &violations, &tests);

        let summary = Self::generate_summary(ai_percentage, total, &sensitive_files, &violations);

        PRReport {
            schema: "tracegit.pr-report.v1".to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            base_ref: base_ref.to_string(),
            head_ref: head_ref.to_string(),
            overall_risk,
            summary,
            ai_involvement: AiInvolvement {
                ai_lines,
                human_lines,
                unknown_lines,
                ai_percentage,
            },
            sensitive_files,
            commands_executed: commands,
            tests_run: tests,
            tests_missing: Vec::new(), // Populated from diff analysis
            policy_violations: policy_results.to_vec(),
            unattributed_changes: unattributed,
            reviewer_checklist: checklist,
        }
    }

    /// Compute overall risk level
    fn compute_risk(
        ai_percentage: f64,
        sensitive_files: &[String],
        violations: &[&PolicyResult],
        tests: &[TestExecution],
    ) -> RiskLevel {
        let mut score = 0u32;

        // AI involvement scoring
        if ai_percentage > 80.0 {
            score += 3;
        } else if ai_percentage > 50.0 {
            score += 2;
        } else if ai_percentage > 20.0 {
            score += 1;
        }

        // Sensitive files
        score += sensitive_files.len().min(3) as u32;

        // Policy violations
        for v in violations {
            match v.severity {
                RiskLevel::Critical => score += 4,
                RiskLevel::High => score += 3,
                RiskLevel::Medium => score += 2,
                RiskLevel::Low => score += 1,
            }
        }

        // Missing tests
        if tests.is_empty() {
            score += 2;
        }

        match score {
            0..=2 => RiskLevel::Low,
            3..=5 => RiskLevel::Medium,
            6..=9 => RiskLevel::High,
            _ => RiskLevel::Critical,
        }
    }

    /// Generate a human-readable summary
    fn generate_summary(
        ai_percentage: f64,
        total_lines: u64,
        sensitive_files: &[String],
        violations: &[&PolicyResult],
    ) -> String {
        let mut parts = Vec::new();

        parts.push(format!("{:.0}% AI-assisted ({} total lines)", ai_percentage, total_lines));

        if !sensitive_files.is_empty() {
            parts.push(format!("{} sensitive file(s) touched", sensitive_files.len()));
        }

        let failed = violations.iter().filter(|v| !v.passed).count();
        if failed > 0 {
            parts.push(format!("{} policy violation(s)", failed));
        }

        parts.join(". ")
    }

    /// Format the report as Markdown
    pub fn to_markdown(report: &PRReport) -> String {
        let mut md = String::new();

        md.push_str("# TraceLens PR Risk Report\n\n");

        md.push_str(&format!("**Risk Level: {:?}** | Base: `{}` → Head: `{}`\n\n",
            report.overall_risk, report.base_ref, report.head_ref));

        md.push_str(&format!("{}\n\n", report.summary));

        // AI Involvement
        md.push_str("## AI Involvement\n\n");
        md.push_str(&format!("- **AI lines:** {} ({:.0}%)\n", report.ai_involvement.ai_lines, report.ai_involvement.ai_percentage));
        md.push_str(&format!("- **Human lines:** {}\n", report.ai_involvement.human_lines));
        md.push_str(&format!("- **Unknown:** {}\n", report.ai_involvement.unknown_lines));

        // Sensitive Files
        if !report.sensitive_files.is_empty() {
            md.push_str("\n## ⚠ Sensitive Files\n\n");
            for f in &report.sensitive_files {
                md.push_str(&format!("- {}\n", f));
            }
        }

        // Commands
        if !report.commands_executed.is_empty() {
            md.push_str("\n## Commands Executed\n\n");
            for cmd in &report.commands_executed {
                let status = cmd.exit_code.map(|c| if c == 0 { "✓" } else { "✗" }).unwrap_or("?");
                md.push_str(&format!("- {} `{}`\n", status, cmd.command));
            }
        }

        // Tests
        if !report.tests_run.is_empty() {
            md.push_str("\n## Tests\n\n");
            for test in &report.tests_run {
                let status = if test.exit_code == 0 { "✓" } else { "✗" };
                md.push_str(&format!("- {} {} ({} passed, {} failed)\n",
                    status, test.command, test.passed, test.failed));
            }
        }

        // Policy Violations
        if !report.policy_violations.is_empty() {
            md.push_str("\n## Policy Violations\n\n");
            for v in &report.policy_violations {
                let icon = if v.passed { "✓" } else { "✗" };
                md.push_str(&format!("- {} **{}**: {}\n", icon, v.rule_id, v.message));
            }
        }

        // Reviewer Checklist
        if !report.reviewer_checklist.is_empty() {
            md.push_str("\n## Reviewer Checklist\n\n");
            for item in &report.reviewer_checklist {
                md.push_str(&format!("- [ ] {}\n", item));
            }
        }

        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_empty_report() {
        let report = PRReportGenerator::generate("main", "HEAD", &[], &[], vec![], vec![]);
        assert_eq!(report.overall_risk, RiskLevel::Low);
        assert_eq!(report.ai_involvement.ai_percentage, 0.0);
    }

    #[test]
    fn test_generate_ai_heavy_report() {
        let attr = FileAttribution {
            schema: "tracegit.attribution.v1".to_string(),
            file_path: "src/auth/session.ts".to_string(),
            git_blob_sha: "abc".to_string(),
            ranges: vec![AttributionRange {
                range_id: "rng_1".to_string(),
                start_line: 1,
                end_line: 100,
                origin: Origin::Ai,
                evidence_strength: EvidenceStrength::Recorded,
                confidence: 0.95,
                state: AttributionState::Exact,
                session_id: "sess_1".to_string(),
                event_ids: vec![],
                agent_id: "claude-code".to_string(),
                model_id: Some("anthropic:claude-opus-4.7".to_string()),
                prompt_hash: None,
                context_set_id: None,
                policy_tags: vec!["auth".to_string()],
                risk_tags: vec!["security-sensitive".to_string()],
                risk_level: Some(RiskLevel::High),
                tests_run: vec![],
                tests_passed: false,
                reviewer: None,
                reviewed_at: None,
            }],
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        let report = PRReportGenerator::generate("main", "feature/auth", &[attr], &[], vec![], vec![]);
        assert_eq!(report.ai_involvement.ai_lines, 100);
        assert_eq!(report.ai_involvement.ai_percentage, 100.0);
        assert!(matches!(report.overall_risk, RiskLevel::High | RiskLevel::Critical));
        assert!(!report.sensitive_files.is_empty());
        assert!(!report.reviewer_checklist.is_empty());
    }

    #[test]
    fn test_to_markdown() {
        let report = PRReportGenerator::generate("main", "HEAD", &[], &[], vec![], vec![]);
        let md = PRReportGenerator::to_markdown(&report);
        assert!(md.contains("# TraceLens PR Risk Report"));
        assert!(md.contains("AI Involvement"));
    }
}
