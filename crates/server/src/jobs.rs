//! Durable background-job worker.
//!
//! Heavy org exports are enqueued as persistent jobs (survive restarts) and run
//! by a worker that claims `queued` jobs, executes them, and stores the JSON
//! result. [`process_one`] runs a single job and is used both by the background
//! loop and by tests (deterministic, no timing).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::storage::{ComplianceSnapshot, Job, Store};
use tellur_core::policy::PolicyEngine;
use tellur_core::schema::types::{Origin, RiskLevel};

/// Job kinds the worker knows how to run.
pub const KIND_EXPORT_EVENTS: &str = "export.events";
pub const KIND_EXPORT_AUDIT: &str = "export.audit";
pub const KIND_COMPLIANCE: &str = "policy.compliance";
/// Per-repo SLSA / SPDX exports run as durable jobs for large repos (A13).
pub const KIND_EXPORT_SLSA: &str = "export.slsa";
pub const KIND_EXPORT_SPDX: &str = "export.spdx";
/// Org-wide evidence pack: every repo's SLSA + latest compliance + audit head.
pub const KIND_EXPORT_EVIDENCE: &str = "export.evidence";

/// Builder id stamped into generated SLSA provenance.
const BUILDER_ID: &str = "https://tellur.dev/hub";

/// Arguments for a per-repo export job (JSON in `job.params`).
#[derive(serde::Deserialize)]
pub struct RepoExportParams {
    pub repo_id: String,
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
}

/// Policy name evaluated for compliance snapshots (A8). The org's `default`
/// policy is the convention; if absent, the compliance run is a no-op.
pub const COMPLIANCE_POLICY: &str = "default";

/// Claim and run a single queued job. Returns `true` if a job was processed,
/// `false` if the queue was empty.
pub fn process_one(store: &Arc<dyn Store>) -> Result<bool> {
    let Some(job) = store.claim_next_job()? else {
        return Ok(false);
    };
    match run_job(store, &job) {
        Ok(value) => {
            let text = serde_json::to_string(&value)?;
            store.complete_job(&job.id, &text)?;
        }
        Err(e) => {
            store.fail_job(&job.id, &e.to_string())?;
        }
    }
    Ok(true)
}

/// Execute a job's work and return its JSON result.
fn run_job(store: &Arc<dyn Store>, job: &Job) -> Result<Value> {
    match job.kind.as_str() {
        KIND_EXPORT_EVENTS => {
            let events = store.export_events(&job.org_id)?;
            Ok(json!({
                "schema": "tellur.server.export.events.v1",
                "org_id": job.org_id,
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "count": events.len(),
                "events": events,
            }))
        }
        KIND_EXPORT_AUDIT => {
            let entries = store.export_audit(&job.org_id)?;
            let chain_intact = store.verify_audit_chain()?;
            Ok(json!({
                "schema": "tellur.server.export.audit.v1",
                "org_id": job.org_id,
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "chain_intact": chain_intact,
                "count": entries.len(),
                "entries": entries,
            }))
        }
        KIND_COMPLIANCE => run_compliance(store, &job.org_id),
        KIND_EXPORT_SLSA | KIND_EXPORT_SPDX => run_repo_export(store, job),
        KIND_EXPORT_EVIDENCE => run_evidence(store, &job.org_id),
        other => bail!("unknown job kind: {other}"),
    }
}

/// Generate a single repo's SLSA provenance or SPDX SBOM (A13). The repo and
/// optional build context come from the job's `params`.
fn run_repo_export(store: &Arc<dyn Store>, job: &Job) -> Result<Value> {
    let raw = job
        .params
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("missing export params"))?;
    let p: RepoExportParams = serde_json::from_str(raw).context("invalid export params")?;
    let repo = store
        .find_repo(&job.org_id, &p.repo_id)?
        .ok_or_else(|| anyhow::anyhow!("repo not found: {}", p.repo_id))?;
    let attrs = store.list_attributions(&job.org_id, &repo.id)?;
    let repo_url = p
        .repo_url
        .unwrap_or_else(|| format!("tellur:repo/{}", repo.id));
    let commit = p.commit.unwrap_or_else(|| "unknown".to_string());

    let doc = if job.kind == KIND_EXPORT_SLSA {
        serde_json::to_value(tellur_core::export::generate_slsa_provenance(
            &repo_url, &commit, &attrs, BUILDER_ID,
        ))?
    } else {
        serde_json::to_value(tellur_core::export::generate_spdx_sbom(
            &repo.name, &repo_url, &commit, &attrs,
        ))?
    };
    Ok(json!({
        "schema": format!("tellur.server.{}.v1", job.kind),
        "org_id": job.org_id,
        "repo_id": repo.id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "document": doc,
    }))
}

/// Build an org-wide evidence pack: every repo's SLSA provenance, the latest
/// per-repo compliance snapshot, and the audit chain's verification state. One
/// downloadable bundle for an auditor.
fn run_evidence(store: &Arc<dyn Store>, org_id: &str) -> Result<Value> {
    let repos = store.list_repos(org_id)?;
    let mut provenance = Vec::with_capacity(repos.len());
    for repo in &repos {
        let attrs = store.list_attributions(org_id, &repo.id)?;
        let repo_url = format!("tellur:repo/{}", repo.id);
        let slsa =
            tellur_core::export::generate_slsa_provenance(&repo_url, "unknown", &attrs, BUILDER_ID);
        provenance.push(json!({
            "repo_id": repo.id,
            "repo_name": repo.name,
            "slsa": serde_json::to_value(slsa)?,
        }));
    }
    let compliance = store.latest_compliance(org_id)?;
    let audit_chain_intact = store.verify_audit_chain()?;
    let audit_entries = store.audit_len()?;
    Ok(json!({
        "schema": "tellur.server.evidence.v1",
        "org_id": org_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "repos_evaluated": repos.len(),
        "provenance": provenance,
        "compliance": compliance,
        "audit": { "chain_intact": audit_chain_intact, "entries": audit_entries },
    }))
}

/// Evaluate the org's `default` policy against every repo's stored attribution
/// and persist a timestamped snapshot per repo (A8). Returns a summary; the
/// dashboard reads the persisted snapshots via `latest_compliance`.
fn run_compliance(store: &Arc<dyn Store>, org_id: &str) -> Result<Value> {
    let evaluated_at = chrono::Utc::now().to_rfc3339();
    let Some(policy_doc) = store.get_policy(org_id, COMPLIANCE_POLICY)? else {
        // No policy configured: nothing to evaluate, and we record nothing so the
        // dashboard can distinguish "no policy" from "evaluated, zero violations".
        return Ok(json!({
            "schema": "tellur.server.compliance.v1",
            "org_id": org_id,
            "generated_at": evaluated_at,
            "policy": Value::Null,
            "repos_evaluated": 0,
        }));
    };
    let engine = PolicyEngine::from_yaml_str(&policy_doc.content)?;

    let repos = store.list_repos(org_id)?;
    let mut total_violations: i64 = 0;
    // Buffer every repo's snapshot and persist them in one transaction only after
    // the whole run succeeds, so a mid-run failure (e.g. unreadable attribution)
    // never leaves a partial run as the "latest" results (see put_compliance_snapshots).
    let mut snapshots = Vec::with_capacity(repos.len());
    for repo in &repos {
        let files = store.list_attributions(org_id, &repo.id)?;
        let mut ai_ranges = 0i64;
        let (mut violations, mut high, mut medium, mut low) = (0i64, 0i64, 0i64, 0i64);
        for file in &files {
            for range in &file.ranges {
                if range.origin == Origin::Ai {
                    ai_ranges += 1;
                }
                for result in engine.evaluate_attribution(range, &file.file_path) {
                    if result.passed {
                        continue;
                    }
                    violations += 1;
                    match result.severity {
                        // The snapshot carries three buckets; Critical folds into
                        // High (the most severe bucket) so it is never understated.
                        RiskLevel::Critical | RiskLevel::High => high += 1,
                        RiskLevel::Medium => medium += 1,
                        RiskLevel::Low => low += 1,
                    }
                }
            }
        }
        total_violations += violations;
        snapshots.push(ComplianceSnapshot {
            repo_id: repo.id.clone(),
            repo_name: repo.name.clone(),
            policy_name: policy_doc.name.clone(),
            policy_version: policy_doc.version,
            evaluated_at: evaluated_at.clone(),
            ai_ranges,
            violations,
            high,
            medium,
            low,
        });
    }
    store.put_compliance_snapshots(org_id, &snapshots)?;

    Ok(json!({
        "schema": "tellur.server.compliance.v1",
        "org_id": org_id,
        "generated_at": evaluated_at,
        "policy": { "name": policy_doc.name, "version": policy_doc.version },
        "repos_evaluated": repos.len(),
        "total_violations": total_violations,
    }))
}

/// Spawn the background worker loop. Drains the queue, then idles between polls.
pub fn spawn_worker(store: Arc<dyn Store>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Reclaim jobs left `running` by a previous process (crash/restart).
        match store.requeue_running_jobs() {
            Ok(n) if n > 0 => tracing::info!(requeued = n, "reclaimed in-flight jobs on startup"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "failed to requeue running jobs on startup"),
        }
        loop {
            // Drain all ready jobs, each on a blocking thread.
            loop {
                let store = store.clone();
                match tokio::task::spawn_blocking(move || process_one(&store)).await {
                    Ok(Ok(true)) => continue,
                    Ok(Ok(false)) => break,
                    Ok(Err(e)) => {
                        tracing::error!(error = %e, "job processing failed");
                        break;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "job worker task panicked");
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

/// How often the maintenance loop runs.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(3600);

/// Upper bound on the retention window (~100 years) — effectively "keep forever"
/// while keeping the day count well within i64 and chrono's valid date range.
const MAX_RETENTION_DAYS: u64 = 36_500;

/// Counts removed by one maintenance pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PruneCounts {
    pub sessions: u64,
    pub logins: u64,
    pub jobs: u64,
}

/// Run one retention pass: drop expired sessions and stale login transactions
/// always (safe data-minimisation), and finished jobs older than
/// `retention_days` when that is non-zero. Never touches the tamper-evident
/// event or audit chains. Returns the counts removed.
pub fn run_maintenance_once(store: &Arc<dyn Store>, retention_days: u64) -> Result<PruneCounts> {
    let sessions = store.prune_expired_sessions()?;
    let logins = store.prune_expired_logins(crate::oidc::LOGIN_TTL_SECS)?;
    let jobs = if retention_days > 0 {
        // Clamp to a sane ceiling (~100y) so an absurd env value can't wrap the
        // i64 cast negative (which would put the cutoff in the future and delete
        // every finished job) or overflow chrono's date arithmetic.
        let days = retention_days.min(MAX_RETENTION_DAYS) as i64;
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        store.prune_finished_jobs(&cutoff)?
    } else {
        0
    };
    Ok(PruneCounts {
        sessions,
        logins,
        jobs,
    })
}

/// Spawn the background retention loop. `retention_days = 0` disables job
/// pruning but expired sessions/logins are always cleaned up.
pub fn spawn_maintenance(
    store: Arc<dyn Store>,
    retention_days: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let store = store.clone();
            match tokio::task::spawn_blocking(move || run_maintenance_once(&store, retention_days))
                .await
            {
                Ok(Ok(c)) if c != PruneCounts::default() => {
                    tracing::info!(
                        sessions = c.sessions,
                        logins = c.logins,
                        jobs = c.jobs,
                        "retention pass pruned records"
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::error!(error = %e, "retention pass failed"),
                Err(e) => tracing::error!(error = %e, "retention task panicked"),
            }
            tokio::time::sleep(MAINTENANCE_INTERVAL).await;
        }
    })
}
