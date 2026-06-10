//! Durable background-job worker.
//!
//! Heavy org exports are enqueued as persistent jobs (survive restarts) and run
//! by a worker that claims `queued` jobs, executes them, and stores the JSON
//! result. [`process_one`] runs a single job and is used both by the background
//! loop and by tests (deterministic, no timing).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::storage::{ComplianceSnapshot, Job, Store};
use tellur_core::policy::PolicyEngine;
use tellur_core::schema::types::{Origin, RiskLevel};

/// Job kinds the worker knows how to run.
pub const KIND_EXPORT_EVENTS: &str = "export.events";
pub const KIND_EXPORT_AUDIT: &str = "export.audit";
pub const KIND_COMPLIANCE: &str = "policy.compliance";

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
        other => bail!("unknown job kind: {other}"),
    }
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
