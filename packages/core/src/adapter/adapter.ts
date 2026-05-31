/**
 * Adapter interface — every AI tool adapter implements this
 */

import type { TraceEvent, Session } from "../schemas/types.js";

export interface DetectionResult {
  detected: boolean;
  toolName: string;
  version?: string;
  configPath?: string;
}

export interface AdapterCapability {
  sessionLifecycle: boolean;
  promptCapture: boolean;
  fileReadCapture: boolean;
  fileWriteCapture: boolean;
  shellCommandCapture: boolean;
  toolCallCapture: boolean;
  mcpCapture: boolean;
  modelMetadata: boolean;
  costCapture: boolean;
  testResultCapture: boolean;
  externalContextCapture: boolean;
  branchCommitCapture: boolean;
  nativeAttributionImport: boolean;
}

export interface AgentAdapter {
  /** Unique identifier for this adapter */
  readonly id: string;

  /** Human-readable name */
  readonly name: string;

  /** Detect if this tool is present in the workspace */
  detect(workspacePath: string): Promise<DetectionResult>;

  /** Optional: install hooks/integration for this tool */
  install?(workspacePath: string, config?: Record<string, unknown>): Promise<void>;

  /** Optional: remove hooks/integration */
  uninstall?(workspacePath: string): Promise<void>;

  /** Import existing data from this tool */
  import?(source: string): AsyncGenerator<TraceEvent | Session>;

  /** List what this adapter can capture */
  capabilities(): AdapterCapability;
}
