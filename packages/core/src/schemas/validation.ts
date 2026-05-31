/**
 * Event type guards and validation helpers
 */

import type {
  TraceEvent,
  Session,
  FileAttribution,
  ProvenanceBundle,
  PRReport,
} from "./types.js";

// ─── Schema discriminators ──────────────────────────────────────────────────

const SCHEMA_VERSIONS = {
  session: "tracegit.session.v1",
  event: "tracegit.event.v1",
  attribution: "tracegit.attribution.v1",
  provenance: "tracegit.provenance.v1",
  prReport: "tracegit.pr-report.v1",
} as const;

// ─── Validation ─────────────────────────────────────────────────────────────

export function isValidSession(obj: unknown): obj is Session {
  if (typeof obj !== "object" || obj === null) return false;
  const s = obj as Record<string, unknown>;
  return (
    s.schema === SCHEMA_VERSIONS.session &&
    typeof s.id === "string" &&
    typeof s.repoId === "string" &&
    typeof s.startedAt === "string" &&
    typeof s.status === "string"
  );
}

export function isValidEvent(obj: unknown): obj is TraceEvent {
  if (typeof obj !== "object" || obj === null) return false;
  const e = obj as Record<string, unknown>;
  return (
    e.schema === SCHEMA_VERSIONS.event &&
    typeof e.id === "string" &&
    typeof e.sessionId === "string" &&
    typeof e.timestamp === "string" &&
    typeof e.type === "string"
  );
}

export function isValidAttribution(obj: unknown): obj is FileAttribution {
  if (typeof obj !== "object" || obj === null) return false;
  const a = obj as Record<string, unknown>;
  return (
    a.schema === SCHEMA_VERSIONS.attribution &&
    typeof a.filePath === "string" &&
    typeof a.gitBlobSha === "string" &&
    Array.isArray(a.ranges)
  );
}

export function isValidProvenanceBundle(obj: unknown): obj is ProvenanceBundle {
  if (typeof obj !== "object" || obj === null) return false;
  const b = obj as Record<string, unknown>;
  return (
    b.schema === SCHEMA_VERSIONS.provenance &&
    typeof b.id === "string" &&
    typeof b.bundleHash === "string"
  );
}

export function isValidPRReport(obj: unknown): obj is PRReport {
  if (typeof obj !== "object" || obj === null) return false;
  const r = obj as Record<string, unknown>;
  return (
    r.schema === SCHEMA_VERSIONS.prReport &&
    typeof r.overallRisk === "string" &&
    typeof r.summary === "string"
  );
}

// ─── ID Generation ──────────────────────────────────────────────────────────

export function generateId(prefix: string): string {
  const timestamp = Date.now().toString(36);
  const random = Math.random().toString(36).substring(2, 10);
  return `${prefix}_${timestamp}_${random}`;
}

export function generateSessionId(): string {
  return generateId("sess");
}

export function generateEventId(): string {
  return generateId("evt");
}

export function generateRangeId(): string {
  return generateId("rng");
}

// ─── Hashing ────────────────────────────────────────────────────────────────

export async function hashContent(content: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(content);
  const hashBuffer = await crypto.subtle.digest("SHA-256", data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
}

export async function hashEvent(event: Omit<TraceEvent, "eventHash">): Promise<string> {
  const canonical = JSON.stringify({
    id: event.id,
    sessionId: event.sessionId,
    timestamp: event.timestamp,
    type: event.type,
    actor: event.actor,
    payload: event.payload,
    prevHash: event.prevHash,
  });
  return hashContent(canonical);
}
