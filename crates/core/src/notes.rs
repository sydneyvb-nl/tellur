//! Git notes interop for AI authorship metadata.
//!
//! Tellur keeps its rich provenance in `.tellur/`. This module provides a
//! compact Git AI-compatible authorship note for `refs/notes/ai` so commit-level
//! attribution can travel with Git history.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::schema::types::{AttributionRange, Origin};

pub const GIT_AI_NOTES_REF: &str = "refs/notes/ai";
pub const GIT_AI_SCHEMA_VERSION: &str = "authorship/3.0.0";

#[derive(Debug, Clone)]
pub struct IndexedAttribution {
    pub file_path: String,
    pub git_blob_sha: String,
    pub range: AttributionRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAuthorshipLog {
    pub schema_version: String,
    pub base_commit_sha: String,
    pub files: Vec<ParsedFileAttestation>,
    pub sessions: BTreeMap<String, GitAiSessionRecord>,
    pub humans: BTreeMap<String, GitAiHumanRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFileAttestation {
    pub path: String,
    pub entries: Vec<ParsedAttestationEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAttestationEntry {
    pub key: String,
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitAiAgentId {
    pub tool: String,
    pub id: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitAiSessionRecord {
    pub agent_id: GitAiAgentId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_author: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub custom_attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitAiHumanRecord {
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitAiMetadata {
    schema_version: String,
    git_ai_version: String,
    base_commit_sha: String,
    prompts: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    humans: BTreeMap<String, GitAiHumanRecord>,
    #[serde(default)]
    sessions: BTreeMap<String, GitAiSessionRecord>,
}

pub fn render_git_ai_note(
    attributions: &[IndexedAttribution],
    base_commit_sha: &str,
    tellur_version: &str,
) -> Result<String> {
    let mut files: BTreeMap<String, BTreeMap<String, Vec<(u32, u32)>>> = BTreeMap::new();
    let mut sessions = BTreeMap::new();
    let mut humans = BTreeMap::new();

    for item in attributions {
        let Some(key) = attestation_key(item, &mut sessions, &mut humans) else {
            continue;
        };
        files
            .entry(item.file_path.clone())
            .or_default()
            .entry(key)
            .or_default()
            .push((item.range.start_line, item.range.end_line));
    }

    let mut out = String::new();
    for (file_path, entries) in files {
        out.push_str(&format_file_path(&file_path));
        out.push('\n');
        for (key, ranges) in entries {
            out.push_str("  ");
            out.push_str(&key);
            out.push(' ');
            out.push_str(&format_ranges(&ranges));
            out.push('\n');
        }
    }

    let metadata = GitAiMetadata {
        schema_version: GIT_AI_SCHEMA_VERSION.to_string(),
        git_ai_version: format!("tellur/{}", tellur_version),
        base_commit_sha: base_commit_sha.to_string(),
        prompts: BTreeMap::new(),
        humans,
        sessions,
    };

    out.push_str("---\n");
    out.push_str(&serde_json::to_string_pretty(&metadata)?);
    out.push('\n');
    Ok(out)
}

pub fn parse_git_ai_note(note: &str) -> Result<ParsedAuthorshipLog> {
    let (attestation, metadata) = note
        .split_once("\n---\n")
        .context("Git AI note missing `---` divider")?;
    let metadata: GitAiMetadata =
        serde_json::from_str(metadata.trim()).context("invalid Git AI note metadata JSON")?;

    let mut files = Vec::new();
    let mut current: Option<ParsedFileAttestation> = None;
    for line in attestation.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(entry_line) = line.strip_prefix("  ") {
            let Some(file) = current.as_mut() else {
                bail!("attestation entry appeared before a file path");
            };
            let (key, ranges) = entry_line
                .split_once(' ')
                .context("attestation entry missing range list")?;
            file.entries.push(ParsedAttestationEntry {
                key: key.to_string(),
                ranges: parse_ranges(ranges)?,
            });
            continue;
        }
        if let Some(file) = current.take() {
            files.push(file);
        }
        current = Some(ParsedFileAttestation {
            path: parse_file_path(line)?,
            entries: Vec::new(),
        });
    }
    if let Some(file) = current {
        files.push(file);
    }

    Ok(ParsedAuthorshipLog {
        schema_version: metadata.schema_version,
        base_commit_sha: metadata.base_commit_sha,
        files,
        sessions: metadata.sessions,
        humans: metadata.humans,
    })
}

fn attestation_key(
    item: &IndexedAttribution,
    sessions: &mut BTreeMap<String, GitAiSessionRecord>,
    humans: &mut BTreeMap<String, GitAiHumanRecord>,
) -> Option<String> {
    match item.range.origin {
        Origin::Ai | Origin::Mixed => {
            let session_key = stable_key("s", &item.range.session_id);
            let trace_key = stable_key("t", &item.range.range_id);
            let evidence = serde_json::to_value(&item.range.evidence_strength)
                .ok()
                .and_then(|value| value.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            sessions
                .entry(session_key.clone())
                .or_insert_with(|| GitAiSessionRecord {
                    agent_id: GitAiAgentId {
                        tool: item.range.agent_id.clone(),
                        id: item.range.session_id.clone(),
                        model: item
                            .range
                            .model_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    },
                    human_author: item.range.reviewer.clone(),
                    custom_attributes: BTreeMap::from([
                        ("tellur.evidence_strength".to_string(), evidence),
                        (
                            "tellur.confidence".to_string(),
                            format!("{:.2}", item.range.confidence),
                        ),
                    ]),
                });
            Some(format!("{}::{}", session_key, trace_key))
        }
        Origin::Human => {
            let author = item
                .range
                .reviewer
                .clone()
                .unwrap_or_else(|| item.range.agent_id.clone());
            let key = stable_key("h", &author);
            humans
                .entry(key.clone())
                .or_insert_with(|| GitAiHumanRecord { author });
            Some(key)
        }
        Origin::Unknown => None,
    }
}

fn stable_key(prefix: &str, value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("{}_{}", prefix, hex_prefix(&digest, 14))
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    let mut s = String::with_capacity(chars);
    for byte in bytes {
        s.push_str(&format!("{:02x}", byte));
        if s.len() >= chars {
            s.truncate(chars);
            break;
        }
    }
    s
}

fn format_ranges(ranges: &[(u32, u32)]) -> String {
    let mut ranges = ranges.to_vec();
    ranges.sort_unstable();
    ranges
        .into_iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{}-{}", start, end)
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_ranges(input: &str) -> Result<Vec<(u32, u32)>> {
    input
        .split(',')
        .map(|part| {
            if let Some((start, end)) = part.split_once('-') {
                Ok((start.parse()?, end.parse()?))
            } else {
                let line = part.parse()?;
                Ok((line, line))
            }
        })
        .collect()
}

fn format_file_path(path: &str) -> String {
    if path.contains([' ', '\t', '\n', '"']) {
        serde_json::to_string(path).unwrap_or_else(|_| path.to_string())
    } else {
        path.to_string()
    }
}

fn parse_file_path(path: &str) -> Result<String> {
    if path.starts_with('"') {
        Ok(serde_json::from_str(path)?)
    } else {
        Ok(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::notes::{IndexedAttribution, parse_git_ai_note, render_git_ai_note};
    use crate::schema::types::{AttributionRange, AttributionState, EvidenceStrength, Origin};

    fn sample_attr(file_path: &str, session_id: &str, range_id: &str) -> IndexedAttribution {
        IndexedAttribution {
            file_path: file_path.to_string(),
            git_blob_sha: "blob123".to_string(),
            range: AttributionRange {
                range_id: range_id.to_string(),
                start_line: 3,
                end_line: 7,
                origin: Origin::Ai,
                evidence_strength: EvidenceStrength::Recorded,
                confidence: 0.94,
                state: AttributionState::Exact,
                session_id: session_id.to_string(),
                event_ids: vec!["evt_1".to_string()],
                agent_id: "codex".to_string(),
                model_id: Some("gpt-5".to_string()),
                prompt_hash: Some("prompt123".to_string()),
                context_set_id: None,
                policy_tags: vec![],
                risk_tags: vec![],
                risk_level: None,
                tests_run: vec![],
                tests_passed: false,
                reviewer: None,
                reviewed_at: None,
            },
        }
    }

    #[test]
    fn renders_git_ai_authorship_note_with_attestation_and_metadata() {
        let note = render_git_ai_note(
            &[sample_attr("src/main.rs", "sess_codex_1", "rng_1")],
            "abc123",
            "0.1.0",
        )
        .unwrap();

        assert!(note.contains("src/main.rs\n"));
        assert!(note.contains("  s_"));
        assert!(note.contains(" 3-7\n"));
        assert!(note.contains("\n---\n"));
        assert!(note.contains("\"schema_version\": \"authorship/3.0.0\""));
        assert!(note.contains("\"base_commit_sha\": \"abc123\""));
        assert!(note.contains("\"tool\": \"codex\""));
        assert!(note.contains("\"model\": \"gpt-5\""));
    }

    #[test]
    fn parses_git_ai_authorship_note_back_to_files_and_sessions() {
        let note = render_git_ai_note(
            &[sample_attr("src/main.rs", "sess_codex_1", "rng_1")],
            "abc123",
            "0.1.0",
        )
        .unwrap();

        let parsed = parse_git_ai_note(&note).unwrap();

        assert_eq!(parsed.schema_version, "authorship/3.0.0");
        assert_eq!(parsed.base_commit_sha, "abc123");
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].path, "src/main.rs");
        assert_eq!(parsed.files[0].entries[0].ranges, vec![(3, 7)]);
        assert_eq!(parsed.sessions.len(), 1);
    }
}
