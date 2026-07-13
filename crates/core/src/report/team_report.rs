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
    /// Added line ranges in the commit's resulting files.
    pub added_ranges: BTreeMap<String, Vec<(u32, u32)>>,
    /// Deleted lines are reported for scope, but cannot carry resulting-line attribution.
    pub deleted_lines: u64,
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
    pub coverage_status: String,
    pub added_lines: u64,
    pub deleted_lines: u64,
    pub attributed_lines: u64,
    pub coverage_percentage: f64,
    pub ai_lines: u64,
    pub human_lines: u64,
    pub unknown_lines: u64,
    pub ai_percentage: f64,
    /// AI-assisted lines per tool (e.g. `claude-code`).
    pub by_tool: BTreeMap<String, u64>,
    /// AI-assisted lines per model.
    pub by_model: BTreeMap<String, u64>,
    /// AI-assisted lines grouped by evidence strength (recorded/imported/inferred/claimed).
    pub by_evidence: BTreeMap<String, u64>,
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
    let mut by_evidence: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_author: BTreeMap<String, TeamAuthorStats> = BTreeMap::new();
    let mut with_provenance = 0usize;
    let mut without_provenance: Vec<String> = Vec::new();

    let mut added_lines = 0u64;
    let mut deleted_lines = 0u64;

    for commit in commits {
        added_lines += count_ranges(commit.added_ranges.values().flatten().copied());
        deleted_lines += commit.deleted_lines;
        let parsed = match commit.note.as_deref() {
            Some(note) => match parse_git_ai_note(note) {
                Ok(parsed) => parsed,
                Err(_) => {
                    without_provenance.push(short_sha(&commit.sha));
                    unknown_lines += count_ranges(commit.added_ranges.values().flatten().copied());
                    continue;
                }
            },
            None => {
                without_provenance.push(short_sha(&commit.sha));
                unknown_lines += count_ranges(commit.added_ranges.values().flatten().copied());
                continue;
            }
        };
        with_provenance += 1;

        for (file_path, ranges) in &commit.added_ranges {
            let attested_file = parsed.files.iter().find(|file| file.path == *file_path);
            for line in ranges.iter().flat_map(|(start, end)| *start..=*end) {
                let entry = attested_file.and_then(|file| {
                    file.entries.iter().find(|entry| {
                        entry
                            .ranges
                            .iter()
                            .any(|(start, end)| line >= *start && line <= *end)
                    })
                });
                let Some(entry) = entry else {
                    unknown_lines += 1;
                    continue;
                };

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
                    let evidence = session
                        .and_then(|s| s.custom_attributes.get("tellur.evidence_strength"))
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string());
                    ai_lines += 1;
                    *by_tool.entry(tool).or_default() += 1;
                    *by_model.entry(model).or_default() += 1;
                    *by_evidence.entry(evidence).or_default() += 1;
                    by_author.entry(author).or_default().ai_lines += 1;
                } else if let Some(human) = parsed.humans.get(&entry.key) {
                    human_lines += 1;
                    by_author
                        .entry(human.author.clone())
                        .or_default()
                        .human_lines += 1;
                } else {
                    unknown_lines += 1;
                }
            }
        }
    }

    let attributed_lines = ai_lines + human_lines;
    let ai_percentage = if added_lines > 0 {
        (ai_lines as f64 / added_lines as f64) * 100.0
    } else {
        0.0
    };
    let coverage_percentage = if added_lines > 0 {
        (attributed_lines as f64 / added_lines as f64) * 100.0
    } else {
        100.0
    };
    let coverage_status = match (added_lines, attributed_lines, unknown_lines) {
        (0, _, _) => "empty",
        (_, 0, _) => "missing",
        (_, _, 0) => "complete",
        _ => "partial",
    };

    TeamReport {
        schema: "tellur.team-report.v1".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        base_ref: base_ref.to_string(),
        head_ref: head_ref.to_string(),
        commits_total: commits.len(),
        commits_with_provenance: with_provenance,
        commits_without_provenance: without_provenance,
        coverage_status: coverage_status.to_string(),
        added_lines,
        deleted_lines,
        attributed_lines,
        coverage_percentage,
        ai_lines,
        human_lines,
        unknown_lines,
        ai_percentage,
        by_tool,
        by_model,
        by_evidence,
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

    match report.coverage_status.as_str() {
        "missing" => out.push_str("> ⚠️ **Provenance unavailable.** Tellur cannot determine whether this PR's added lines are AI- or human-authored.\n\n"),
        "partial" => out.push_str("> ⚠️ **Provenance is partial.** The AI share below is confirmed evidence over all added lines; unattributed lines remain unknown.\n\n"),
        "complete" => out.push_str("> ✅ **Provenance coverage is complete for added lines.**\n\n"),
        _ => out.push_str("> ℹ️ **This PR contains no added lines to attribute.**\n\n"),
    }

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
    out.push_str(&format!("- Added lines in PR: {}\n", report.added_lines));
    out.push_str(&format!(
        "- Deleted lines in PR: {}\n",
        report.deleted_lines
    ));
    out.push_str(&format!(
        "- Attributed added lines: {} ({:.1}% coverage)\n",
        report.attributed_lines, report.coverage_percentage
    ));
    out.push_str(&format!(
        "- Unattributed added lines: {}\n\n",
        report.unknown_lines
    ));

    out.push_str("## AI involvement\n\n");
    if report.coverage_status == "missing" {
        out.push_str("- AI-assisted lines: **unknown** (no portable provenance)\n");
        out.push_str("- Human lines: **unknown** (no portable provenance)\n");
    } else if report.coverage_status == "empty" {
        out.push_str("- AI-assisted lines: n/a\n");
        out.push_str("- Human lines: n/a\n");
    } else {
        out.push_str(&format!(
            "- AI-assisted lines: {} ({:.1}% of all added lines)\n",
            report.ai_lines, report.ai_percentage
        ));
        out.push_str(&format!("- Human lines: {}\n", report.human_lines));
    }
    out.push_str(&format!(
        "- Unknown added lines: {}\n\n",
        report.unknown_lines
    ));

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

    if !report.by_evidence.is_empty() {
        out.push_str("## AI lines by evidence strength\n\n");
        for (evidence, lines) in sorted_desc(&report.by_evidence) {
            out.push_str(&format!("- {}: {}\n", evidence, lines));
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

fn count_ranges(ranges: impl Iterator<Item = (u32, u32)>) -> u64 {
    ranges
        .map(|(start, end)| u64::from(end.saturating_sub(start) + 1))
        .sum()
}

/// Extract resulting-file addition ranges and deletion counts from a zero-context Git patch.
pub fn parse_commit_patch(patch: &str) -> (BTreeMap<String, Vec<(u32, u32)>>, u64) {
    let mut files = BTreeMap::<String, Vec<(u32, u32)>>::new();
    let mut current_file: Option<String> = None;
    let mut deleted = 0u64;

    for line in patch.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            current_file =
                (path != "/dev/null").then(|| path.strip_prefix("b/").unwrap_or(path).to_string());
            continue;
        }
        let Some(hunk) = line.strip_prefix("@@") else {
            continue;
        };
        let Some(header) = hunk.split("@@").next() else {
            continue;
        };
        let mut parts = header.split_whitespace();
        let old = parts.find(|part| part.starts_with('-'));
        let new = parts.find(|part| part.starts_with('+'));
        if let Some(old) = old {
            deleted += u64::from(parse_hunk_part(old).1);
        }
        if let (Some(file), Some(new)) = (current_file.as_ref(), new) {
            let (start, count) = parse_hunk_part(new);
            if count > 0 {
                files
                    .entry(file.clone())
                    .or_default()
                    .push((start, start + count - 1));
            }
        }
    }
    (files, deleted)
}

fn parse_hunk_part(part: &str) -> (u32, u32) {
    let value = part.trim_start_matches(['-', '+']);
    let mut pieces = value.split(',');
    let start = pieces.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count = pieces.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (start, count)
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
                added_ranges: BTreeMap::from([("src/a.rs".to_string(), vec![(3, 7)])]),
                deleted_lines: 2,
            },
            TeamCommitNote {
                sha: "bbbbbbbbbbbb".to_string(),
                note: None,
                added_ranges: BTreeMap::new(),
                deleted_lines: 0,
            },
        ];

        let report = aggregate_team_report("main", "HEAD", &commits);
        assert_eq!(report.commits_total, 2);
        assert_eq!(report.commits_with_provenance, 1);
        assert_eq!(report.commits_without_provenance, vec!["bbbbbbbb"]);
        assert_eq!(report.ai_lines, 5);
        assert_eq!(report.human_lines, 0);
        assert_eq!(report.ai_percentage, 100.0);
        assert_eq!(report.coverage_status, "complete");
        assert_eq!(report.added_lines, 5);
        assert_eq!(report.deleted_lines, 2);
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
                added_ranges: BTreeMap::from([("src/a.rs".to_string(), vec![(1, 3)])]),
                deleted_lines: 0,
            },
            TeamCommitNote {
                sha: "cafebabe0000".to_string(),
                note: None,
                added_ranges: BTreeMap::from([("src/b.rs".to_string(), vec![(8, 9)])]),
                deleted_lines: 1,
            },
        ];
        let report = aggregate_team_report("main", "HEAD", &commits);
        assert_eq!(report.commits_with_provenance, 0);
        assert_eq!(report.commits_without_provenance.len(), 2);
        assert_eq!(report.ai_lines, 0);
        assert_eq!(report.unknown_lines, 5);
        assert_eq!(report.coverage_status, "missing");
        let markdown = to_markdown(&report);
        assert!(markdown.contains("Provenance unavailable"));
        assert!(markdown.contains("AI-assisted lines: **unknown**"));
        assert!(!markdown.contains("AI-assisted lines: 0"));
        assert!(to_markdown(&report).contains("Without provenance: 2"));
    }

    #[test]
    fn empty_range_is_zeroed() {
        let report = aggregate_team_report("main", "HEAD", &[]);
        assert_eq!(report.commits_total, 0);
        assert_eq!(report.ai_percentage, 0.0);
        assert!(to_markdown(&report).contains("Commits in range: 0"));
    }

    #[test]
    fn parses_added_ranges_and_deletions_from_zero_context_patch() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -2,2 +2,3 @@\n-old\n+new\n@@ -10,0 +12,2 @@\n+one\n+two\n";
        let (files, deleted) = parse_commit_patch(patch);
        assert_eq!(files["src/a.rs"], vec![(2, 4), (12, 13)]);
        assert_eq!(deleted, 2);
    }
}
