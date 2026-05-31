//! Core type definitions for TraceGit v1 schemas.
//!
//! Every event, session, attribution, and provenance record conforms to these types.
//! Schema version: tracegit.v1

use serde::{Deserialize, Serialize};

// ─── Enums ──────────────────────────────────────────────────────────────────

/// Who or what produced a code change
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Human,
    Ai,
    Mixed,
    Unknown,
}

/// How strong is the evidence for an attribution
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceStrength {
    Recorded,
    Imported,
    Inferred,
    Claimed,
    Unknown,
}

/// Current state of an attribution range
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttributionState {
    Exact,
    Moved,
    Modified,
    Split,
    Merged,
    Uncertain,
    Lost,
}

/// Risk level
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Policy action
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    Allow,
    Warn,
    Block,
    Require,
    Fail,
}

/// Session status
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Completed,
    Failed,
    Interrupted,
    Unknown,
}

/// Event actor
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventActor {
    Human,
    Agent,
    System,
    Unknown,
}

// ─── Event Types ────────────────────────────────────────────────────────────

/// All possible event types in the TraceGit event stream
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventType {
    // Session lifecycle
    #[serde(rename = "session.start")]
    SessionStart,
    #[serde(rename = "session.end")]
    SessionEnd,
    // Prompt
    #[serde(rename = "prompt.submitted")]
    PromptSubmitted,
    #[serde(rename = "prompt.context_added")]
    PromptContextAdded,
    // File operations
    #[serde(rename = "file.read")]
    FileRead,
    #[serde(rename = "file.write")]
    FileWrite,
    #[serde(rename = "file.patch")]
    FilePatch,
    #[serde(rename = "file.delete")]
    FileDelete,
    // Commands
    #[serde(rename = "command.pre_execute")]
    CommandPreExecute,
    #[serde(rename = "command.post_execute")]
    CommandPostExecute,
    // Tool calls
    #[serde(rename = "tool.pre_call")]
    ToolPreCall,
    #[serde(rename = "tool.post_call")]
    ToolPostCall,
    // MCP
    #[serde(rename = "mcp.pre_call")]
    McpPreCall,
    #[serde(rename = "mcp.post_call")]
    McpPostCall,
    // Tests
    #[serde(rename = "test.run")]
    TestRun,
    #[serde(rename = "test.result")]
    TestResult,
    // Git
    #[serde(rename = "git.diff")]
    GitDiff,
    #[serde(rename = "git.commit")]
    GitCommit,
    #[serde(rename = "git.branch")]
    GitBranch,
    #[serde(rename = "git.merge")]
    GitMerge,
    #[serde(rename = "git.rebase")]
    GitRebase,
    // Policy
    #[serde(rename = "policy.violation")]
    PolicyViolation,
    #[serde(rename = "policy.override")]
    PolicyOverride,
    // Review
    #[serde(rename = "review.approval")]
    ReviewApproval,
    // Export
    #[serde(rename = "export.created")]
    ExportCreated,
}

// ─── Core Entities ──────────────────────────────────────────────────────────

/// Actor — a human or agent that performs actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_hash: Option<String>,
    #[serde(rename = "type")]
    pub actor_type: EventActor,
}

/// Agent info — which AI tool produced the change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Model info — which model was used
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Task info — what was being worked on
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// SHA-256 hash of the full prompt
    pub prompt_hash: String,
    /// Redacted version safe for storage/export
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_redacted: Option<String>,
}

/// Redaction metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionInfo {
    pub applied: bool,
    pub mode: RedactionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules_applied: Option<Vec<String>>,
}

/// Redaction mode
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RedactionMode {
    None,
    Automatic,
    Strict,
    HashOnly,
    Custom,
}

// ─── Session ────────────────────────────────────────────────────────────────

/// A bounded AI-assisted development interaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub schema: String,
    pub id: String,
    pub repo_id: String,
    pub workspace_id: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub human_actor: Actor,
    pub agent: AgentInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<std::collections::HashMap<String, String>>,
    pub status: SessionStatus,
    pub related_commits: Vec<String>,
    pub related_branches: Vec<String>,
    pub related_prs: Vec<String>,
}

impl Session {
    pub fn new(repo_id: String, human_actor: Actor, agent: AgentInfo) -> Self {
        Self {
            schema: "tracegit.session.v1".to_string(),
            id: crate::schema::ids::generate_session_id(),
            repo_id,
            workspace_id: crate::schema::ids::generate_id("ws"),
            started_at: chrono::Utc::now().to_rfc3339(),
            ended_at: None,
            human_actor,
            agent,
            model: None,
            task: None,
            environment: None,
            status: SessionStatus::Active,
            related_commits: Vec::new(),
            related_branches: Vec::new(),
            related_prs: Vec::new(),
        }
    }
}

// ─── Event ──────────────────────────────────────────────────────────────────

/// A timestamped action within a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub schema: String,
    pub id: String,
    pub session_id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub actor: EventActor,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction: Option<RedactionInfo>,
    /// SHA-256 hash of the previous event — tamper-evident chain
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    /// SHA-256 hash of this event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_hash: Option<String>,
}

// ─── Attribution ────────────────────────────────────────────────────────────

/// A range of lines attributed to a specific origin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionRange {
    pub range_id: String,
    pub start_line: u32,
    pub end_line: u32,
    pub origin: Origin,
    pub evidence_strength: EvidenceStrength,
    pub confidence: f64,
    pub state: AttributionState,
    pub session_id: String,
    pub event_ids: Vec<String>,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_set_id: Option<String>,
    pub policy_tags: Vec<String>,
    pub risk_tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<RiskLevel>,
    pub tests_run: Vec<String>,
    pub tests_passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
}

/// File-level attribution map
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttribution {
    pub schema: String,
    pub file_path: String,
    pub git_blob_sha: String,
    pub ranges: Vec<AttributionRange>,
    pub updated_at: String,
}

// ─── Context Set ────────────────────────────────────────────────────────────

/// A file read as context during a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: String,
    /// SHA-256 of file contents at time of read
    pub content_hash: String,
    pub read_at: String,
}

/// Set of context used for a specific AI action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSet {
    pub id: String,
    pub session_id: String,
    pub files: Vec<ContextFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_urls: Option<Vec<String>>,
    /// Whether any untrusted external content was included
    pub untrusted_content: bool,
}

// ─── Command Execution ──────────────────────────────────────────────────────

/// Record of a shell command executed by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandExecution {
    pub command: String,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
}

// ─── Test Execution ─────────────────────────────────────────────────────────

/// Record of a test run during a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestExecution {
    pub test_runner: String,
    pub command: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<f64>,
}

// ─── Policy ─────────────────────────────────────────────────────────────────

/// A policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    pub when: serde_json::Value,
    pub action: PolicyAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require: Option<serde_json::Value>,
}

/// A sensitive path definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitivePath {
    pub path: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_human_review: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_tests: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_ai_automerge: Option<bool>,
}

/// A complete policy file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFile {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitive_paths: Option<Vec<SensitivePath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<PolicyRule>>,
}

// ─── Provenance Bundle ──────────────────────────────────────────────────────

/// Result of evaluating a policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResult {
    pub rule_id: String,
    pub passed: bool,
    pub severity: RiskLevel,
    pub message: String,
    pub evidence: Vec<String>,
}

/// Portable export of all provenance data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceBundle {
    pub schema: String,
    pub id: String,
    pub created_at: String,
    pub repo_id: String,
    pub git_ref: String,
    pub git_commit_sha: String,
    pub sessions: Vec<Session>,
    pub events: Vec<TraceEvent>,
    pub attributions: Vec<FileAttribution>,
    pub context_sets: Vec<ContextSet>,
    pub policy_results: Vec<PolicyResult>,
    pub export_profile: String,
    pub bundle_hash: String,
}

// ─── PR Report ──────────────────────────────────────────────────────────────

/// AI involvement statistics for a PR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiInvolvement {
    pub ai_lines: u64,
    pub human_lines: u64,
    pub unknown_lines: u64,
    pub ai_percentage: f64,
}

/// Complete PR risk report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PRReport {
    pub schema: String,
    pub generated_at: String,
    pub base_ref: String,
    pub head_ref: String,
    pub overall_risk: RiskLevel,
    pub summary: String,
    pub ai_involvement: AiInvolvement,
    pub sensitive_files: Vec<String>,
    pub commands_executed: Vec<CommandExecution>,
    pub tests_run: Vec<TestExecution>,
    pub tests_missing: Vec<String>,
    pub policy_violations: Vec<PolicyResult>,
    pub unattributed_changes: Vec<String>,
    pub reviewer_checklist: Vec<String>,
}
