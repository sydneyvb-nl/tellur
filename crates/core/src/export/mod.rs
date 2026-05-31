//! SLSA/SPDX export — supply chain provenance formats
//!
//! Generates SLSA provenance and SPDX SBOM documents from TraceGit data
//! for compliance and supply chain security.

use serde::{Deserialize, Serialize};

use crate::schema::types::*;

/// SLSA v1.0 Provenance document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaProvenance {
    #[serde(rename = "_type")]
    pub doc_type: String,
    pub subject: Vec<SlsaSubject>,
    pub predicate_type: String,
    pub predicate: SlsaPredicate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaSubject {
    pub name: String,
    pub digest: SlsaDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaDigest {
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaPredicate {
    pub builder: SlsaBuilder,
    pub build_type: String,
    pub invocation: SlsaInvocation,
    pub materials: Vec<SlsaMaterial>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaBuilder {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaInvocation {
    pub config_source: SlsaConfigSource,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaConfigSource {
    pub uri: String,
    pub digest: SlsaDigest,
    pub entry_point: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlsaMaterial {
    pub uri: String,
    pub digest: Option<SlsaDigest>,
    pub ai_origin: Option<String>,
    pub ai_model: Option<String>,
    pub ai_confidence: Option<f64>,
}

/// SPDX 2.3 SBOM document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxDocument {
    pub spdx_version: String,
    pub data_license: String,
    pub spdx_id: String,
    pub name: String,
    pub document_namespace: String,
    pub creation_info: SpdxCreationInfo,
    pub packages: Vec<SpdxPackage>,
    pub relationships: Vec<SpdxRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxCreationInfo {
    pub created: String,
    pub creators: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxPackage {
    pub spdx_id: String,
    pub name: String,
    pub version_info: Option<String>,
    pub download_location: String,
    pub files_analyzed: bool,
    pub attribution_texts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpdxRelationship {
    pub spdx_element_id: String,
    pub relationship_type: String,
    pub related_spdx_element: String,
}

/// Generate a SLSA provenance document from TraceGit data
pub fn generate_slsa_provenance(
    repo_url: &str,
    commit_sha: &str,
    attributions: &[FileAttribution],
    builder_id: &str,
) -> SlsaProvenance {
    let mut materials = Vec::new();

    for attr in attributions {
        let ai_info: Option<&AttributionRange> = attr.ranges.iter().find(|r|
            matches!(r.origin, Origin::Ai | Origin::Mixed)
        );

        materials.push(SlsaMaterial {
            uri: format!("git+{}#{}", repo_url, commit_sha),
            digest: Some(SlsaDigest {
                sha256: attr.git_blob_sha.clone(),
            }),
            ai_origin: ai_info.map(|r| format!("{:?}", r.origin)),
            ai_model: ai_info.and_then(|r| r.model_id.clone()),
            ai_confidence: ai_info.map(|r| r.confidence),
        });
    }

    SlsaProvenance {
        doc_type: "https://in-toto.io/Statement/v1".to_string(),
        subject: vec![SlsaSubject {
            name: repo_url.to_string(),
            digest: SlsaDigest {
                sha256: commit_sha.to_string(),
            },
        }],
        predicate_type: "https://slsa.dev/provenance/v1".to_string(),
        predicate: SlsaPredicate {
            builder: SlsaBuilder {
                id: builder_id.to_string(),
            },
            build_type: "https://tracegit.dev/build/v1".to_string(),
            invocation: SlsaInvocation {
                config_source: SlsaConfigSource {
                    uri: format!("git+{}", repo_url),
                    digest: SlsaDigest {
                        sha256: commit_sha.to_string(),
                    },
                    entry_point: "tracegit".to_string(),
                },
                parameters: serde_json::json!({}),
            },
            materials,
        },
    }
}

/// Generate an SPDX SBOM with AI attribution info
pub fn generate_spdx_sbom(
    repo_name: &str,
    repo_url: &str,
    commit_sha: &str,
    attributions: &[FileAttribution],
) -> SpdxDocument {
    let mut packages = Vec::new();
    let mut relationships = Vec::new();

    for (i, attr) in attributions.iter().enumerate() {
        let ai_lines: Vec<&AttributionRange> = attr.ranges.iter()
            .filter(|r| matches!(r.origin, Origin::Ai | Origin::Mixed))
            .collect();

        let human_lines: Vec<&AttributionRange> = attr.ranges.iter()
            .filter(|r| matches!(r.origin, Origin::Human))
            .collect();

        let mut attribution_texts = Vec::new();
        if !ai_lines.is_empty() {
            let total_ai: u64 = ai_lines.iter().map(|r| (r.end_line - r.start_line + 1) as u64).sum();
            attribution_texts.push(format!("AI-generated: {} lines", total_ai));
            for r in &ai_lines {
                if let Some(ref model) = r.model_id {
                    attribution_texts.push(format!("AI model: {}", model));
                }
                attribution_texts.push(format!("AI confidence: {:.0}%", r.confidence * 100.0));
            }
        }
        if !human_lines.is_empty() {
            let total_human: u64 = human_lines.iter().map(|r| (r.end_line - r.start_line + 1) as u64).sum();
            attribution_texts.push(format!("Human-written: {} lines", total_human));
        }

        let short_sha: String = commit_sha.chars().take(8).collect();
        let pkg_id = format!("SPDXRef-file-{}", i);
        packages.push(SpdxPackage {
            spdx_id: pkg_id.clone(),
            name: attr.file_path.clone(),
            version_info: Some(short_sha),
            download_location: format!("git+{}#{}", repo_url, commit_sha),
            files_analyzed: true,
            attribution_texts,
        });

        relationships.push(SpdxRelationship {
            spdx_element_id: "SPDXRef-DOCUMENT".to_string(),
            relationship_type: "CONTAINS".to_string(),
            related_spdx_element: pkg_id,
        });
    }

    SpdxDocument {
        spdx_version: "SPDX-2.3".to_string(),
        data_license: "CC0-1.0".to_string(),
        spdx_id: "SPDXRef-DOCUMENT".to_string(),
        name: format!("{}-tracegit-sbom", repo_name),
        document_namespace: format!("https://tracegit.dev/sbom/{}/{}", repo_name, commit_sha),
        creation_info: SpdxCreationInfo {
            created: chrono::Utc::now().to_rfc3339(),
            creators: vec![
                "Tool: TraceGit".to_string(),
                "Organization: TraceGit".to_string(),
            ],
        },
        packages,
        relationships,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attribution() -> FileAttribution {
        FileAttribution {
            schema: "tracegit.attribution.v1".to_string(),
            file_path: "src/auth.rs".to_string(),
            git_blob_sha: "abc123".to_string(),
            ranges: vec![
                AttributionRange {
                    range_id: "rng_1".to_string(),
                    start_line: 1,
                    end_line: 50,
                    origin: Origin::Ai,
                    evidence_strength: EvidenceStrength::Recorded,
                    confidence: 0.92,
                    state: AttributionState::Exact,
                    session_id: "sess_1".to_string(),
                    event_ids: vec![],
                    agent_id: "claude-code".to_string(),
                    model_id: Some("anthropic:claude-opus-4.7".to_string()),
                    prompt_hash: None,
                    context_set_id: None,
                    policy_tags: vec![],
                    risk_tags: vec![],
                    risk_level: None,
                    tests_run: vec![],
                    tests_passed: false,
                    reviewer: None,
                    reviewed_at: None,
                },
            ],
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_generate_slsa_provenance() {
        let attr = sample_attribution();
        let slsa = generate_slsa_provenance(
            "https://github.com/example/repo",
            "sha256abc",
            &[attr],
            "https://tracegit.dev/builder/v1",
        );

        assert_eq!(slsa.doc_type, "https://in-toto.io/Statement/v1");
        assert_eq!(slsa.predicate.materials.len(), 1);
        assert!(slsa.predicate.materials[0].ai_origin.is_some());
        assert_eq!(slsa.predicate.materials[0].ai_confidence.unwrap(), 0.92);

        // Verify it serializes to valid JSON
        let json = serde_json::to_string(&slsa).unwrap();
        assert!(json.contains("slsa.dev"));
    }

    #[test]
    fn test_generate_spdx_sbom() {
        let attr = sample_attribution();
        let spdx = generate_spdx_sbom(
            "my-repo",
            "https://github.com/example/repo",
            "sha256abc",
            &[attr],
        );

        assert_eq!(spdx.spdx_version, "SPDX-2.3");
        assert_eq!(spdx.packages.len(), 1);
        assert!(spdx.packages[0].attribution_texts.iter().any(|t| t.contains("AI-generated")));
        assert!(spdx.packages[0].attribution_texts.iter().any(|t| t.contains("claude-opus")));

        let json = serde_json::to_string(&spdx).unwrap();
        assert!(json.contains("SPDX-2.3"));
    }
}
