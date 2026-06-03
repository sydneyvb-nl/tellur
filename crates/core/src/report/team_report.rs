//! Team AI-involvement report (Tier 0, Git-native).
//!
//! Aggregates the `refs/notes/ai` authorship notes of many commits — typically
//! every commit in a PR/branch range, contributed by different people — into one
//! team-level view: how much of the range is AI-assisted, broken down by tool,
//! model and author, plus which commits carry provenance at all.
//!
//! This needs no server: notes travel over the existing Git remote
//! (`tellur notes push`/`fetch`). This module is the pure aggregation core; the
//! CLI gathers the commits + notes from Git and renders the result.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::notes::parse_git_ai_note;

/// One commit in the range plus its raw `refs/notes/ai` note, if any.
pub struct TeamCommitNote {
    pub sha: String,
    /// Raw note text, or `None` if the commit has no authorship note.
    pub note: Option<String>,
}

/// Per-author line tallies (AI-assisted vs. human-authored).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TeamAuthorStats {
    pub ai_lines: u64,
    pub human_lines: u64,
}

/// Aggregated team AI-involvement report over a commit range.
#[derive(Debug, Clone, Serialize)]
pub struct TeamReport {
    pub schema: String,
    pub generated_at: String,
    pub base_ref: String,
    pub head_ref: String,
    pub commits_total: usize,
    pub commits_with_provenance: usize,
    /// Short SHAs of commits with no (or unparseable) authorship note.
    pub commits_without_provenance: Vec<String>,
    pub ai_lines: u64,
    pub human_lines: u64,
    pub unknown_lines: u64,
    pub ai_percentage: f64,
    /// AI-assisted lines per tool (e.g. `claude-code`).
    pub by_tool: BTreeMap<String, u64>,
    /// AI-assisted lines per model.
    pub by_model: BTreeMap<String, u64>,
    /// AI vs human lines per author.
    pub by_author: BTreeMap<String, TeamAuthorStats>,
}

/// Aggregate the authorship notes of a commit range into a team report.
///
/// Tolerant by design: a commit with no note, or a note that fails to parse, is
/// counted under `commits_without_provenance` rather than failing the report —
/// partial team adoption should still produce a useful view.
pub fn aggregate_team_report(
    base_ref: &str,
    head_ref: &str,
    commits: &[TeamCommitNote],
) -> TeamReport {
    let mut ai_lines: u64 = 0;
    let mut human_lines: u64 = 0;
    let mut unknown_lines: u64 = 0;
    let mut by_tool: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_model: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_author: BTreeMap<String, TeamAuthorStats> = BTreeMap::new();
    let mut with_provenance = 0usize;
    let mut without_provenance: Vec<String> = Vec::new();

    for commit in commits {
        let parsed = match commit.note.as_deref() {
            Some(note) => match parse_git_ai_note(note) {
                Ok(parsed) => parsed,
                Err(_) => {
                    without_provenance.push(short_sha(&commit.sha));
                    continue;
                }
            },
            None => {
                without_provenance.push(short_sha(&commit.sha));
                continue;
            }
        };
        with_provenance += 1;

        for file in &parsed.files {
            for entry in &file.entries {
                let lines: u64 = entry
                    .ranges
                    .iter()
                    .map(|(start, end)| u64::from(end.saturating_sub(*start) + 1))
                    .sum();
                if lines == 0 {
                    continue;
                }

                if let Some((session_key, _)) = entry.key.split_once("::") {
                    // AI-assisted: resolve tool/model/author from the session map.
                    let session = parsed.sessions.get(session_key);
                    let tool = session
                        .map(|s| s.agent_id.tool.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    let model = session
                        .map(|s| s.agent_id.model.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    let author = session
                        .and_then(|s| s.human_author.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    ai_lines += lines;
                    *by_tool.entry(tool).or_default() += lines;
                    *by_model.entry(model).or_default() += lines;
                    by_author.entry(author).or_default().ai_lines += lines;
                } else if let Some(human) = parsed.humans.get(&entry.key) {
                    human_lines += lines;
                    by_author
                        .entry(human.author.clone())
                        .or_default()
                        .human_lines += lines;
                } else {
                    unknown_lines += lines;
                }
            }
        }
    }

    let total = ai_lines + human_lines + unknown_lines;
    let ai_percentage = if total > 0 {
        (ai_lines as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    TeamReport {
        schema: "tellur.team-report.v1".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        base_ref: base_ref.to_string(),
        head_ref: head_ref.to_string(),
        commits_total: commits.len(),
        commits_with_provenance: with_provenance,
        commits_without_provenance: without_provenance,
        ai_lines,
        human_lines,
        unknown_lines,
        ai_percentage,
        by_tool,
        by_model,
        by_author,
    }
}

/// Render a team report as Markdown (for PR comments / CI output).
pub fn to_markdown(report: &TeamReport) -> String {
    let mut out = String::new();
    out.push_str("# Tellur Team AI-Involvement Report\n\n");
    out.push_str(&format!(
        "Range: `{}..{}`\n\n",
        report.base_ref, report.head_ref
    ));

    out.push_str("## Coverage\n\n");
    out.push_str(&format!("- Commits in range: {}\n", report.commits_total));
    out.push_str(&format!(
        "- With provenance: {}\n",
        report.commits_with_provenance
    ));
    if report.commits_without_provenance.is_empty() {
        out.push_str("- Without provenance: 0\n\n");
    } else {
        out.push_str(&format!(
            "- Without provenance: {} ({})\n\n",
            report.commits_without_provenance.len(),
            report.commits_without_provenance.join(", ")
        ));
    }

    out.push_str("## AI involvement\n\n");
    out.push_str(&format!(
        "- AI-assisted lines: {} ({:.1}%)\n",
        report.ai_lines, report.ai_percentage
    ));
    out.push_str(&format!("- Human lines: {}\n", report.human_lines));
    out.push_str(&format!("- Unknown lines: {}\n\n", report.unknown_lines));

    if !report.by_tool.is_empty() {
        out.push_str("## AI lines by tool\n\n");
        for (tool, lines) in sorted_desc(&report.by_tool) {
            out.push_str(&format!("- {}: {}\n", tool, lines));
        }
        out.push('\n');
    }

    if !report.by_model.is_empty() {
        out.push_str("## AI lines by model\n\n");
        for (model, lines) in sorted_desc(&report.by_model) {
            out.push_str(&format!("- {}: {}\n", model, lines));
        }
        out.push('\n');
    }

    if !report.by_author.is_empty() {
        out.push_str("## By author\n\n");
        out.push_str("| Author | AI lines | Human lines |\n");
        out.push_str("| --- | --- | --- |\n");
        for (author, stats) in &report.by_author {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                author, stats.ai_lines, stats.human_lines
            ));
        }
        out.push('\n');
    }

    out
}

/// Sort a `name -> count` map by descending count, then name, for stable output.
fn sorted_desc(map: &BTreeMap<String, u64>) -> Vec<(String, u64)> {
    let mut items: Vec<(String, u64)> = map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{IndexedAttribution, render_git_ai_note};
    use crate::schema::types::{AttributionRange, AttributionState, EvidenceStrength, Origin};

    fn ai_attr(
        file: &str,
        session: &str,
        range_id: &str,
        start: u32,
        end: u32,
    ) -> IndexedAttribution {
        IndexedAttribution {
            file_path: file.to_string(),
            git_blob_sha: "blob".to_string(),
            range: AttributionRange {
                range_id: range_id.to_string(),
                start_line: start,
                end_line: end,
                origin: Origin::Ai,
                evidence_strength: EvidenceStrength::Recorded,
                confidence: 0.95,
                state: AttributionState::Exact,
                session_id: session.to_string(),
                event_ids: vec![],
                agent_id: "claude-code".to_string(),
                model_id: Some("claude-opus-4.7".to_string()),
                prompt_hash: None,
                context_set_id: None,
                policy_tags: vec![],
                risk_tags: vec![],
                risk_level: None,
                tests_run: vec![],
                tests_passed: false,
                reviewer: Some("alice".to_string()),
                reviewed_at: None,
            },
        }
    }

    #[test]
    fn aggregates_ai_lines_by_tool_model_author() {
        // 5 AI lines (3..7 inclusive) from claude-code / opus, author alice.
        let note = render_git_ai_note(
            &[ai_attr("src/a.rs", "sess1", "rng1", 3, 7)],
            "base",
            "test",
        )
        .unwrap();
        let commits = vec![
            TeamCommitNote {
                sha: "aaaaaaaaaaaa".to_string(),
                note: Some(note),
            },
            TeamCommitNote {
                sha: "bbbbbbbbbbbb".to_string(),
                note: None,
            },
        ];

        let report = aggregate_team_report("main", "HEAD", &commits);
        assert_eq!(report.commits_total, 2);
        assert_eq!(report.commits_with_provenance, 1);
        assert_eq!(report.commits_without_provenance, vec!["bbbbbbbb"]);
        assert_eq!(report.ai_lines, 5);
        assert_eq!(report.human_lines, 0);
        assert_eq!(report.ai_percentage, 100.0);
        assert_eq!(report.by_tool.get("claude-code"), Some(&5));
        assert_eq!(report.by_model.get("claude-opus-4.7"), Some(&5));
        assert_eq!(report.by_author.get("alice").map(|s| s.ai_lines), Some(5));
    }

    #[test]
    fn unparseable_and_missing_notes_count_as_no_provenance() {
        let commits = vec![
            TeamCommitNote {
                sha: "deadbeef0000".to_string(),
                note: Some("not a valid note".to_string()),
            },
            TeamCommitNote {
                sha: "cafebabe0000".to_string(),
                note: None,
            },
        ];
        let report = aggregate_team_report("main", "HEAD", &commits);
        assert_eq!(report.commits_with_provenance, 0);
        assert_eq!(report.commits_without_provenance.len(), 2);
        assert_eq!(report.ai_lines, 0);
        assert!(to_markdown(&report).contains("Without provenance: 2"));
    }

    #[test]
    fn empty_range_is_zeroed() {
        let report = aggregate_team_report("main", "HEAD", &[]);
        assert_eq!(report.commits_total, 0);
        assert_eq!(report.ai_percentage, 0.0);
        assert!(to_markdown(&report).contains("Commits in range: 0"));
    }
}
