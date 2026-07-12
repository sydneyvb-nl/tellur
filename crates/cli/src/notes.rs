//! Git authorship-notes commands (`notes export|show|import|fetch|push|
//! install-config`) and the no-server `team report` aggregation.

use anyhow::{Context, Result};

use tellur_core::storage::{RepoStorage, TraceIndex};

use crate::git::{git_output, read_git_note, resolve_commit, run_git, short_sha, write_git_note};
use crate::util::sanitize_id;

pub(crate) fn cmd_notes_export(commit: &str, notes_ref: &str, print: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let attributions = index.list_attributions()?;
    if attributions.is_empty() {
        println!("No attribution data to export.");
        return Ok(());
    }

    let commit_sha = resolve_commit(&storage.root, commit)?;
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
    run_git(
        &storage.root,
        &[
            "config",
            "--add",
            &format!("remote.{}.fetch", remote),
            &format!("+{}:{}", notes_ref, notes_ref),
        ],
    )?;
    run_git(
        &storage.root,
        &["config", "--add", "notes.rewriteRef", notes_ref],
    )?;
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
    let revs = git_output(&storage.root, &["rev-list", &range])
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
