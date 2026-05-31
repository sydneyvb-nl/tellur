/**
 * TraceGit Core Schemas
 *
 * These types define the data model for AI code provenance.
 * Every event, session, and attribution record conforms to these types.
 *
 * Schema version: tracegit.v1
 */

// ─── Origin ─────────────────────────────────────────────────────────────────

/** Who or what produced a code change */
export type Origin = "human" | "ai" | "mixed" | "unknown";

/** How strong is the evidence for an attribution */
export type EvidenceStrength = "recorded" | "imported" | "inferred" | "claimed" | "unknown";

/** Current state of an attribution range */
export type AttributionState =
  | "exact"
  | "moved"
  | "modified"
  | "split"
  | "merged"
  | "uncertain"
  | "lost";

/** Risk level for a change or file */
export type RiskLevel = "low" | "medium" | "high" | "critical";

/** Policy action */
export type PolicyAction = "allow" | "warn" | "block" | "require" | "fail";

/** Session status */
export type SessionStatus = "active" | "completed" | "failed" | "interrupted" | "unknown";

/** Event actor */
export type EventActor = "human" | "agent" | "system" | "unknown";

// ─── Event Types ────────────────────────────────────────────────────────────

export const EventTypes = {
  // Session lifecycle
  SESSION_START: "session.start",
  SESSION_END: "session.end",

  // Prompt
  PROMPT_SUBMITTED: "prompt.submitted",
  PROMPT_CONTEXT_ADDED: "prompt.context_added",

  // File operations
  FILE_READ: "file.read",
  FILE_WRITE: "file.write",
  FILE_PATCH: "file.patch",
  FILE_DELETE: "file.delete",

  // Commands
  COMMAND_PRE_EXECUTE: "command.pre_execute",
  COMMAND_POST_EXECUTE: "command.post_execute",

  // Tool calls
  TOOL_PRE_CALL: "tool.pre_call",
  TOOL_POST_CALL: "tool.post_call",

  // MCP
  MCP_PRE_CALL: "mcp.pre_call",
  MCP_POST_CALL: "mcp.post_call",

  // Tests
  TEST_RUN: "test.run",
  TEST_RESULT: "test.result",

  // Git
  GIT_DIFF: "git.diff",
  GIT_COMMIT: "git.commit",
  GIT_BRANCH: "git.branch",
  GIT_MERGE: "git.merge",
  GIT_REBASE: "git.rebase",

  // Policy
  POLICY_VIOLATION: "policy.violation",
  POLICY_OVERRIDE: "policy.override",

  // Review
  REVIEW_APPROVAL: "review.approval",

  // Export
  EXPORT_CREATED: "export.created",
} as const;

export type EventType = (typeof EventTypes)[keyof typeof EventTypes];

// ─── Core Entities ──────────────────────────────────────────────────────────

export interface Actor {
  name: string;
  email?: string;
  emailHash?: string;
  type: EventActor;
}

export interface AgentInfo {
  id: string;
  name: string;
  version?: string;
}

export interface ModelInfo {
  provider: string;
  name: string;
  version?: string | null;
}

export interface TaskInfo {
  title?: string;
  /** SHA-256 hash of the full prompt */
  promptHash: string;
  /** Redacted version safe for storage/export */
  promptRedacted?: string;
}

export interface RedactionInfo {
  applied: boolean;
  mode: "none" | "automatic" | "strict" | "hash-only" | "custom";
  rulesApplied?: string[];
}

// ─── Session ────────────────────────────────────────────────────────────────

export interface Session {
  schema: "tracegit.session.v1";
  id: string;
  repoId: string;
  workspaceId: string;
  startedAt: string;
  endedAt?: string;
  humanActor: Actor;
  agent: AgentInfo;
  model?: ModelInfo;
  task?: TaskInfo;
  environment?: Record<string, string>;
  status: SessionStatus;
  relatedCommits: string[];
  relatedBranches: string[];
  relatedPRs: string[];
}

// ─── Event ──────────────────────────────────────────────────────────────────

export interface TraceEvent {
  schema: "tracegit.event.v1";
  id: string;
  sessionId: string;
  timestamp: string;
  type: EventType;
  actor: EventActor;
  payload: Record<string, unknown>;
  redaction?: RedactionInfo;
  /** SHA-256 hash of the previous event — creates tamper-evident chain */
  prevHash?: string;
  /** SHA-256 hash of this event */
  eventHash?: string;
}

// ─── Attribution ────────────────────────────────────────────────────────────

export interface AttributionRange {
  rangeId: string;
  startLine: number;
  endLine: number;
  origin: Origin;
  evidenceStrength: EvidenceStrength;
  confidence: number;
  state: AttributionState;
  sessionId: string;
  eventIds: string[];
  agentId: string;
  modelId?: string;
  promptHash?: string;
  contextSetId?: string;
  policyTags: string[];
  riskTags: string[];
  riskLevel?: RiskLevel;
  testsRun: string[];
  testsPassed: boolean;
  reviewer?: string;
  reviewedAt?: string;
}

export interface FileAttribution {
  schema: "tracegit.attribution.v1";
  filePath: string;
  gitBlobSha: string;
  ranges: AttributionRange[];
  updatedAt: string;
}

// ─── Context Set ────────────────────────────────────────────────────────────

export interface ContextFile {
  path: string;
  /** SHA-256 of file contents at time of read */
  contentHash: string;
  readAt: string;
}

export interface ContextSet {
  id: string;
  sessionId: string;
  files: ContextFile[];
  externalUrls?: string[];
  /** Whether any untrusted external content was included */
  untrustedContent: boolean;
}

// ─── Command Execution ──────────────────────────────────────────────────────

export interface CommandExecution {
  command: string;
  exitCode: number | null;
  durationMs?: number;
  /** Hash of stdout — never store raw output by default */
  stdoutHash?: string;
  /** Hash of stderr */
  stderrHash?: string;
  blocked?: boolean;
  blockReason?: string;
}

// ─── Test Execution ─────────────────────────────────────────────────────────

export interface TestExecution {
  testRunner: string;
  command: string;
  exitCode: number;
  durationMs: number;
  passed: number;
  failed: number;
  skipped: number;
  coverage?: number;
}

// ─── Policy ─────────────────────────────────────────────────────────────────

export interface PolicyRule {
  id: string;
  description: string;
  /** Human-readable explanation of why this rule exists */
  rationale?: string;
  when: Record<string, unknown>;
  action: PolicyAction;
  require?: Record<string, unknown>;
}

export interface SensitivePath {
  path: string;
  tags: string[];
  requireHumanReview?: boolean;
  requireTests?: boolean;
  blockAiAutomerge?: boolean;
}

export interface PolicyFile {
  version: number;
  sensitivePaths?: SensitivePath[];
  rules?: PolicyRule[];
}

// ─── Provenance Bundle ──────────────────────────────────────────────────────

export interface ProvenanceBundle {
  schema: "tracegit.provenance.v1";
  id: string;
  createdAt: string;
  repoId: string;
  gitRef: string;
  gitCommitSha: string;
  sessions: Session[];
  events: TraceEvent[];
  attributions: FileAttribution[];
  contextSets: ContextSet[];
  policyResults: PolicyResult[];
  exportProfile: string;
  /** SHA-256 hash of the entire bundle */
  bundleHash: string;
}

export interface PolicyResult {
  ruleId: string;
  passed: boolean;
  severity: RiskLevel;
  message: string;
  evidence: string[];
}

// ─── PR Report ──────────────────────────────────────────────────────────────

export interface PRReport {
  schema: "tracegit.pr-report.v1";
  generatedAt: string;
  baseRef: string;
  headRef: string;
  overallRisk: RiskLevel;
  summary: string;
  aiInvolvement: {
    aiLines: number;
    humanLines: number;
    unknownLines: number;
    aiPercentage: number;
  };
  sensitiveFiles: string[];
  commandsExecuted: CommandExecution[];
  testsRun: TestExecution[];
  testsMissing: string[];
  policyViolations: PolicyResult[];
  unattributedChanges: string[];
  reviewerChecklist: string[];
}
