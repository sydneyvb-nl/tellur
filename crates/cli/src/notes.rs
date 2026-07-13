//! Git authorship-notes commands (`notes export|show|import|fetch|push|
//! install-config`) and the no-server `team report` aggregation.

use anyhow::{Context, Result, bail};

use tellur_core::storage::{RepoStorage, TraceIndex};

use crate::git::{
    git_config_get_all, git_output, read_git_note, resolve_commit, run_git, short_sha,
    write_git_note,
};
use crate::util::sanitize_id;

pub(crate) fn cmd_notes_export(commit: &str, notes_ref: &str, print: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let commit_sha = resolve_commit(&storage.root, commit)?;
    let attributions =
        commit_scoped_attributions(&storage.root, &commit_sha, index.list_attributions()?)?;
    if attributions.is_empty() {
        println!("No exact attribution data for this commit to export.");
        return Ok(());
    }

    let note = tellur_core::notes::render_git_ai_note(
        &attributions,
        &commit_sha,
        env!("CARGO_PKG_VERSION"),
    )?;

    if print {
        print!("{}", note);
        return Ok(());
    }

    write_git_note(&storage.root, notes_ref, &commit_sha, &note)?;
    println!(
        "Exported {} attribution range(s) to {} on {}",
        attributions.len(),
        notes_ref,
        short_sha(&commit_sha)
    );
    println!("Push with: tellur notes push");
    Ok(())
}

pub(crate) fn cmd_notes_attest_ai(
    commit: &str,
    notes_ref: &str,
    session_id: &str,
    agent_id: &str,
    model_id: &str,
    force: bool,
) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let commit_sha = resolve_commit(&storage.root, commit)?;
    if !force && read_git_note(&storage.root, notes_ref, &commit_sha).is_ok() {
        bail!(
            "{} already has an authorship note in {}; pass --force to replace it",
            short_sha(&commit_sha),
            notes_ref
        );
    }
    let patch = git_output(
        &storage.root,
        &[
            "-c",
            "core.quotePath=false",
            "show",
            "--first-parent",
            "--format=",
            "--unified=0",
            "--no-ext-diff",
            &commit_sha,
        ],
    )?;
    let (added_ranges, _) = tellur_core::report::team_report::parse_commit_patch(&patch);
    if added_ranges.is_empty() {
        println!("Commit has no added lines to attest.");
        return Ok(());
    }
    let attestor = git_output(&storage.root, &["config", "user.name"])
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    let mut attributions = Vec::new();
    for (file_path, ranges) in added_ranges {
        let blob_sha = git_output(
            &storage.root,
            &["rev-parse", &format!("{}:{}", commit_sha, file_path)],
        )?;
        for (start, end) in ranges {
            attributions.push(tellur_core::notes::IndexedAttribution {
                file_path: file_path.clone(),
                git_blob_sha: blob_sha.trim().to_string(),
                range: tellur_core::schema::types::AttributionRange {
                    range_id: format!(
                        "claim_{}_{}_{}_{}",
                        short_sha(&commit_sha),
                        sanitize_id(&file_path),
                        start,
                        end
                    ),
                    start_line: start,
                    end_line: end,
                    origin: tellur_core::schema::types::Origin::Ai,
                    evidence_strength: tellur_core::schema::types::EvidenceStrength::Claimed,
                    confidence: 1.0,
                    state: tellur_core::schema::types::AttributionState::Exact,
                    session_id: session_id.to_string(),
                    event_ids: vec![],
                    agent_id: agent_id.to_string(),
                    model_id: Some(model_id.to_string()),
                    prompt_hash: None,
                    context_set_id: None,
                    policy_tags: vec![],
                    risk_tags: vec![],
                    risk_level: None,
                    tests_run: vec![],
                    tests_passed: false,
                    reviewer: attestor.clone(),
                    reviewed_at: Some(chrono::Utc::now().to_rfc3339()),
                },
            });
        }
    }
    let note = tellur_core::notes::render_git_ai_note(
        &attributions,
        &commit_sha,
        env!("CARGO_PKG_VERSION"),
    )?;
    write_git_note(&storage.root, notes_ref, &commit_sha, &note)?;
    println!(
        "Claimed AI authorship for {} added range(s) on {} (evidence: claimed)",
        attributions.len(),
        short_sha(&commit_sha)
    );
    Ok(())
}

fn commit_scoped_attributions(
    repo: &std::path::Path,
    commit_sha: &str,
    attributions: Vec<tellur_core::notes::IndexedAttribution>,
) -> Result<Vec<tellur_core::notes::IndexedAttribution>> {
    let patch = git_output(
        repo,
        &[
            "-c",
            "core.quotePath=false",
            "show",
            "--first-parent",
            "--format=",
            "--unified=0",
            "--no-ext-diff",
            commit_sha,
        ],
    )?;
    let (added_ranges, _) = tellur_core::report::team_report::parse_commit_patch(&patch);
    let mut scoped = Vec::new();

    for item in attributions {
        let Some(commit_ranges) = added_ranges.get(&item.file_path) else {
            continue;
        };
        let Ok(blob_sha) = git_output(
            repo,
            &["rev-parse", &format!("{}:{}", commit_sha, item.file_path)],
        ) else {
            continue;
        };
        if blob_sha.trim() != item.git_blob_sha {
            continue;
        }
        for (added_start, added_end) in commit_ranges {
            let start = item.range.start_line.max(*added_start);
            let end = item.range.end_line.min(*added_end);
            if start > end {
                continue;
            }
            let mut exact = item.clone();
            exact.range.start_line = start;
            exact.range.end_line = end;
            exact.range.range_id = format!("{}-{}-{}", exact.range.range_id, start, end);
            scoped.push(exact);
        }
    }
    Ok(scoped)
}

pub(crate) fn cmd_notes_show(commit: &str, notes_ref: &str, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let commit_sha = resolve_commit(&storage.root, commit)?;
    let note = read_git_note(&storage.root, notes_ref, &commit_sha)?;
    let parsed = tellur_core::notes::parse_git_ai_note(&note)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": parsed.schema_version,
                "base_commit_sha": parsed.base_commit_sha,
                "files": parsed.files.iter().map(|f| &f.path).collect::<Vec<_>>(),
                "session_count": parsed.sessions.len(),
                "human_count": parsed.humans.len(),
            }))?
        );
        return Ok(());
    }

    println!("Git AI authorship note ({})", notes_ref);
    println!("Commit: {}", short_sha(&commit_sha));
    println!("Schema: {}", parsed.schema_version);
    println!("Base: {}", short_sha(&parsed.base_commit_sha));
    println!("Files: {}", parsed.files.len());
    println!("Sessions: {}", parsed.sessions.len());
    println!("Humans: {}", parsed.humans.len());
    for file in parsed.files {
        println!(
            "  {} ({} entr{})",
            file.path,
            file.entries.len(),
            if file.entries.len() == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

pub(crate) fn cmd_notes_import(commit: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let commit_sha = resolve_commit(&storage.root, commit)?;
    let note = read_git_note(&storage.root, notes_ref, &commit_sha)?;
    let parsed = tellur_core::notes::parse_git_ai_note(&note)?;
    let index = TraceIndex::open(&storage.index_path)?;

    let mut imported = 0u32;
    for file in &parsed.files {
        let blob_sha = git_output(
            &storage.root,
            &["rev-parse", &format!("{}:{}", commit_sha, file.path)],
        )
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| commit_sha.clone());
        for entry in &file.entries {
            for (start, end) in &entry.ranges {
                let (origin, session_id, agent_id, model_id, reviewer) =
                    if let Some(session_key) = entry.key.split_once("::").map(|(s, _)| s) {
                        let session = parsed.sessions.get(session_key);
                        (
                            tellur_core::schema::types::Origin::Ai,
                            session
                                .map(|s| s.agent_id.id.clone())
                                .unwrap_or_else(|| session_key.to_string()),
                            session
                                .map(|s| s.agent_id.tool.clone())
                                .unwrap_or_else(|| "unknown".to_string()),
                            session.map(|s| s.agent_id.model.clone()),
                            session.and_then(|s| s.human_author.clone()),
                        )
                    } else if let Some(human) = parsed.humans.get(&entry.key) {
                        (
                            tellur_core::schema::types::Origin::Human,
                            entry.key.clone(),
                            "human".to_string(),
                            None,
                            Some(human.author.clone()),
                        )
                    } else {
                        (
                            tellur_core::schema::types::Origin::Ai,
                            entry.key.clone(),
                            "unknown".to_string(),
                            None,
                            None,
                        )
                    };

                let range = tellur_core::schema::types::AttributionRange {
                    range_id: format!(
                        "gitai_{}_{}_{}_{}_{}",
                        short_sha(&commit_sha),
                        sanitize_id(&file.path),
                        sanitize_id(&entry.key),
                        start,
                        end
                    ),
                    start_line: *start,
                    end_line: *end,
                    origin,
                    evidence_strength: tellur_core::schema::types::EvidenceStrength::Imported,
                    confidence: 1.0,
                    state: tellur_core::schema::types::AttributionState::Exact,
                    session_id,
                    event_ids: vec![],
                    agent_id,
                    model_id,
                    prompt_hash: None,
                    context_set_id: None,
                    policy_tags: vec![],
                    risk_tags: vec![],
                    risk_level: None,
                    tests_run: vec![],
                    tests_passed: false,
                    reviewer,
                    reviewed_at: None,
                };
                index.index_attribution(
                    &range,
                    &file.path,
                    &blob_sha,
                    &chrono::Utc::now().to_rfc3339(),
                )?;
                imported += 1;
            }
        }
    }

    println!(
        "Imported {} attribution range(s) from {} on {}",
        imported,
        notes_ref,
        short_sha(&commit_sha)
    );
    Ok(())
}

pub(crate) fn cmd_notes_fetch(remote: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    run_git(
        &storage.root,
        &["fetch", remote, &format!("{}:{}", notes_ref, notes_ref)],
    )?;
    println!("Fetched {} from {}", notes_ref, remote);
    Ok(())
}

pub(crate) fn cmd_notes_push(remote: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    run_git(&storage.root, &["push", remote, notes_ref])?;
    println!("Pushed {} to {}", notes_ref, remote);
    Ok(())
}

pub(crate) fn cmd_notes_install_config(remote: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let fetch_key = format!("remote.{remote}.fetch");
    let fetch_value = format!("+{notes_ref}:{notes_ref}");
    if !git_config_get_all(&storage.root, &fetch_key).contains(&fetch_value) {
        run_git(
            &storage.root,
            &["config", "--add", &fetch_key, &fetch_value],
        )?;
    }
    if !git_config_get_all(&storage.root, "notes.rewriteRef").contains(&notes_ref.to_string()) {
        run_git(
            &storage.root,
            &["config", "--add", "notes.rewriteRef", notes_ref],
        )?;
    }
    run_git(
        &storage.root,
        &["config", "notes.rewriteMode", "concatenate"],
    )?;
    println!(
        "Configured {} fetch and rewrite support for {}",
        remote, notes_ref
    );
    Ok(())
}

pub(crate) fn cmd_team_report(base: &str, head: &str, notes_ref: &str, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let range = format!("{base}..{head}");
    // A merge of the base branch is history topology, not new PR authorship.
    // Its first-parent patch can contain every base-branch change since the
    // branch point, so including merge commits inflates PR line accounting.
    let revs = git_output(&storage.root, &["rev-list", "--no-merges", &range])
        .with_context(|| format!("failed to list commits in range {range}"))?;
    let commits: Vec<tellur_core::report::TeamCommitNote> = revs
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|sha| {
            let patch = git_output(
                &storage.root,
                &[
                    "-c",
                    "core.quotePath=false",
                    "show",
                    "--first-parent",
                    "--format=",
                    "--unified=0",
                    "--no-ext-diff",
                    sha,
                ],
            )?;
            let (added_ranges, deleted_lines) =
                tellur_core::report::team_report::parse_commit_patch(&patch);
            Ok(tellur_core::report::TeamCommitNote {
                note: read_git_note(&storage.root, notes_ref, sha).ok(),
                sha: sha.to_string(),
                added_ranges,
                deleted_lines,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let report = tellur_core::report::aggregate_team_report(base, head, &commits);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", tellur_core::report::team_report::to_markdown(&report));
    }
    Ok(())
}
