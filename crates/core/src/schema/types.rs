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

/// All possible event types in the TraceGit event stream.
///
/// Serialised as a flat wire string (e.g. `"file.write"`). Any unrecognised
/// string round-trips through [`EventType::Custom`] rather than being silently
/// coerced — see `as_wire`/`from_wire`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    // Session lifecycle
    SessionStart,
    SessionEnd,
    // Prompt
    PromptSubmitted,
    PromptContextAdded,
    // File operations
    FileRead,
    FileWrite,
    FilePatch,
    FileDelete,
    // Commands
    CommandPreExecute,
    CommandPostExecute,
    // Tool calls
    ToolPreCall,
    ToolPostCall,
    // MCP
    McpPreCall,
    McpPostCall,
    // Tests
    TestRun,
    TestResult,
    // Git
    GitDiff,
    GitCommit,
    GitBranch,
    GitMerge,
    GitRebase,
    // Policy
    PolicyViolation,
    PolicyOverride,
    // Review
    ReviewApproval,
    // Export
    ExportCreated,
    // Convenience aliases used by adapters
    CommandExecution,
    CodeSearch,
    UserPrompt,
    /// Any event type not in the known set. Preserves the original wire string.
    Custom(String),
}

impl EventType {
    /// The canonical wire string for this event type.
    pub fn as_wire(&self) -> String {
        match self {
            EventType::SessionStart => "session.start",
            EventType::SessionEnd => "session.end",
            EventType::PromptSubmitted => "prompt.submitted",
            EventType::PromptContextAdded => "prompt.context_added",
            EventType::FileRead => "file.read",
            EventType::FileWrite => "file.write",
            EventType::FilePatch => "file.patch",
            EventType::FileDelete => "file.delete",
            EventType::CommandPreExecute => "command.pre_execute",
            EventType::CommandPostExecute => "command.post_execute",
            EventType::ToolPreCall => "tool.pre_call",
            EventType::ToolPostCall => "tool.post_call",
            EventType::McpPreCall => "mcp.pre_call",
            EventType::McpPostCall => "mcp.post_call",
            EventType::TestRun => "test.run",
            EventType::TestResult => "test.result",
            EventType::GitDiff => "git.diff",
            EventType::GitCommit => "git.commit",
            EventType::GitBranch => "git.branch",
            EventType::GitMerge => "git.merge",
            EventType::GitRebase => "git.rebase",
            EventType::PolicyViolation => "policy.violation",
            EventType::PolicyOverride => "policy.override",
            EventType::ReviewApproval => "review.approval",
            EventType::ExportCreated => "export.created",
            EventType::CommandExecution => "command.exec",
            EventType::CodeSearch => "code.search",
            EventType::UserPrompt => "user.prompt",
            EventType::Custom(s) => s.as_str(),
        }
        .to_string()
    }

    /// Parse a wire string into an [`EventType`]. Unknown strings become
    /// [`EventType::Custom`] so no information is lost.
    pub fn from_wire(s: &str) -> EventType {
        match s {
            "session.start" => EventType::SessionStart,
            "session.end" => EventType::SessionEnd,
            "prompt.submitted" => EventType::PromptSubmitted,
            "prompt.context_added" => EventType::PromptContextAdded,
            "file.read" => EventType::FileRead,
            "file.write" => EventType::FileWrite,
            "file.patch" => EventType::FilePatch,
            "file.delete" => EventType::FileDelete,
            "command.pre_execute" => EventType::CommandPreExecute,
            "command.post_execute" => EventType::CommandPostExecute,
            "tool.pre_call" => EventType::ToolPreCall,
            "tool.post_call" => EventType::ToolPostCall,
            "mcp.pre_call" => EventType::McpPreCall,
            "mcp.post_call" => EventType::McpPostCall,
            "test.run" => EventType::TestRun,
            "test.result" => EventType::TestResult,
            "git.diff" => EventType::GitDiff,
            "git.commit" => EventType::GitCommit,
            "git.branch" => EventType::GitBranch,
            "git.merge" => EventType::GitMerge,
            "git.rebase" => EventType::GitRebase,
            "policy.violation" => EventType::PolicyViolation,
            "policy.override" => EventType::PolicyOverride,
            "review.approval" => EventType::ReviewApproval,
            "export.created" => EventType::ExportCreated,
            "command.exec" => EventType::CommandExecution,
            "code.search" => EventType::CodeSearch,
            "user.prompt" => EventType::UserPrompt,
            other => EventType::Custom(other.to_string()),
        }
    }
}

impl Serialize for EventType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_wire())
    }
}

impl<'de> Deserialize<'de> for EventType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(EventType::from_wire(&s))
    }
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
    /// If true, AI agents must not read this path (e.g. secrets). Enforced by
    /// capture: matching files are skipped and never have their content stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_ai_read: Option<bool>,
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

#[cfg(test)]
mod event_type_tests {
    use super::EventType;

    #[test]
    fn test_known_round_trip() {
        for et in [
            EventType::FileWrite,
            EventType::CommandPostExecute,
            EventType::SessionStart,
            EventType::UserPrompt,
        ] {
            let json = serde_json::to_string(&et).unwrap();
            let back: EventType = serde_json::from_str(&json).unwrap();
            assert_eq!(et, back);
        }
        // Serialises as a flat string, not an object.
        assert_eq!(serde_json::to_string(&EventType::FileWrite).unwrap(), "\"file.write\"");
        assert_eq!(
            serde_json::to_string(&EventType::CommandPostExecute).unwrap(),
            "\"command.post_execute\""
        );
    }

    #[test]
    fn test_unknown_becomes_custom_and_round_trips() {
        // The bug this guards against: unknown types were silently coerced to
        // file.write and could not round-trip.
        let parsed = EventType::from_wire("vendor.special_event");
        assert_eq!(parsed, EventType::Custom("vendor.special_event".to_string()));
        let json = serde_json::to_string(&parsed).unwrap();
        assert_eq!(json, "\"vendor.special_event\"");
        let back: EventType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, back);
    }
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
