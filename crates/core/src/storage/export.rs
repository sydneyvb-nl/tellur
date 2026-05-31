//! Provenance export — portable bundles for audit, compliance, and sharing

use anyhow::Result;

use crate::schema::types::*;

/// Export profiles
#[derive(Debug, Clone, PartialEq)]
pub enum ExportProfile {
    /// Full developer data (local only)
    DeveloperFull,
    /// Safe for open source — no prompts, no command output, no secrets
    OpenSourcePublic,
    /// Corporate with redaction — encrypted prompts, full audit trail
    CorporateRedacted,
    /// Private audit — full event chain, signed hashes
    AuditPrivate,
    /// Release attestation — release-specific provenance
    ReleaseAttestation,
    /// CI summary — condensed report for CI output
    CISummary,
}

/// Export a provenance bundle
pub fn export_provenance_bundle(
    profile: ExportProfile,
    sessions: Vec<Session>,
    events: Vec<TraceEvent>,
    attributions: Vec<FileAttribution>,
    policy_results: Vec<PolicyResult>,
    git_ref: &str,
    git_commit_sha: &str,
) -> Result<ProvenanceBundle> {
    let repo_id = sessions
        .first()
        .map(|s| s.repo_id.clone())
        .unwrap_or_default();

    let bundle = ProvenanceBundle {
        schema: "tellur.provenance.v1".to_string(),
        id: crate::schema::ids::generate_bundle_id(),
        created_at: chrono::Utc::now().to_rfc3339(),
        repo_id,
        git_ref: git_ref.to_string(),
        git_commit_sha: git_commit_sha.to_string(),
        sessions: filter_sessions(&profile, sessions),
        events: filter_events(&profile, events),
        attributions: filter_attributions(&profile, attributions),
        context_sets: Vec::new(),
        policy_results,
        export_profile: format!("{:?}", profile).to_lowercase(),
        bundle_hash: String::new(), // Computed last
    };

    // Compute bundle hash
    let canonical = serde_json::to_string(&bundle)?;
    let hash = crate::schema::ids::hash_content(&canonical);

    Ok(ProvenanceBundle {
        bundle_hash: hash,
        ..bundle
    })
}

fn filter_sessions(profile: &ExportProfile, sessions: Vec<Session>) -> Vec<Session> {
    match profile {
        // Public OSS: drop environment + prompt excerpts entirely.
        ExportProfile::OpenSourcePublic => sessions
            .into_iter()
            .map(|mut s| {
                s.environment = None;
                if let Some(ref mut task) = s.task {
                    task.prompt_redacted = None;
                }
                s
            })
            .collect(),
        // Corporate: keep audit trail but never export prompt excerpts; keep
        // the prompt *hash* for integrity. Environment is dropped.
        ExportProfile::CorporateRedacted => sessions
            .into_iter()
            .map(|mut s| {
                s.environment = None;
                if let Some(ref mut task) = s.task {
                    task.prompt_redacted = None;
                }
                s
            })
            .collect(),
        // Audit and release keep full session detail; developer keeps everything.
        _ => sessions,
    }
}

fn filter_events(profile: &ExportProfile, events: Vec<TraceEvent>) -> Vec<TraceEvent> {
    match profile {
        // Public OSS: strip all payloads — only event types and timestamps.
        ExportProfile::OpenSourcePublic => events
            .into_iter()
            .map(|mut e| {
                e.payload = serde_json::json!({});
                e.redaction = None;
                e
            })
            .collect(),
        // Corporate: keep payload structure but drop free-text fields that may
        // carry secrets (command strings, prompt text, stdout), keep hashes.
        ExportProfile::CorporateRedacted => events
            .into_iter()
            .map(|mut e| {
                if let Some(obj) = e.payload.as_object_mut() {
                    for key in [
                        "command", "prompt", "input", "output", "content", "stdout", "stderr",
                    ] {
                        obj.remove(key);
                    }
                }
                e
            })
            .collect(),
        // CI / release: only the events relevant to a build/release decision.
        ExportProfile::CISummary | ExportProfile::ReleaseAttestation => events
            .into_iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    EventType::GitCommit
                        | EventType::TestResult
                        | EventType::PolicyViolation
                        | EventType::ReviewApproval
                        | EventType::SessionStart
                        | EventType::SessionEnd
                )
            })
            .collect(),
        // Developer (full) and audit (full chain) keep everything.
        ExportProfile::DeveloperFull | ExportProfile::AuditPrivate => events,
    }
}

fn filter_attributions(
    _profile: &ExportProfile,
    attributions: Vec<FileAttribution>,
) -> Vec<FileAttribution> {
    // Attributions are generally safe to export
    attributions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_developer_full() {
        let bundle = export_provenance_bundle(
            ExportProfile::DeveloperFull,
            vec![],
            vec![],
            vec![],
            vec![],
            "main",
            "abc123",
        )
        .unwrap();

        assert_eq!(bundle.schema, "tellur.provenance.v1");
        assert!(!bundle.bundle_hash.is_empty());
        assert_eq!(bundle.git_ref, "main");
    }

    #[test]
    fn test_export_opensource_strips_payloads() {
        let events = vec![TraceEvent {
            schema: "tellur.event.v1".to_string(),
            id: "evt_1".to_string(),
            session_id: "sess_1".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: EventType::FileWrite,
            actor: EventActor::Agent,
            payload: serde_json::json!({"secret": "sensitive_data"}),
            redaction: None,
            prev_hash: None,
            event_hash: None,
        }];

        let bundle = export_provenance_bundle(
            ExportProfile::OpenSourcePublic,
            vec![],
            events,
            vec![],
            vec![],
            "main",
            "abc",
        )
        .unwrap();

        // Payload should be stripped
        assert_eq!(bundle.events[0].payload, serde_json::json!({}));
    }
}
