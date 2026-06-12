// Pure helpers for the session timeline: classify events into categories, pull
// human-readable context out of their payloads, and roll a session up into
// summary stats. Kept framework-free so they're unit-tested directly.

import type { StoredEvent } from "./api";

export type Category =
  | "prompt"
  | "file"
  | "command"
  | "tool"
  | "test"
  | "git"
  | "session"
  | "policy"
  | "other";

/** Map a dotted event type (e.g. `file.write`) to a display category. */
export function eventCategory(type: string): Category {
  const head = type.split(".")[0] ?? "";
  switch (head) {
    case "prompt":
    case "user":
      return "prompt";
    case "file":
    case "code":
      return "file";
    case "command":
      return "command";
    case "tool":
    case "mcp":
      return "tool";
    case "test":
      return "test";
    case "git":
      return "git";
    case "session":
      return "session";
    case "policy":
    case "review":
      return "policy";
    default:
      return "other";
  }
}

function asRecord(payload: unknown): Record<string, unknown> {
  return payload && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
}

function str(v: unknown): string | null {
  return typeof v === "string" && v.length > 0 ? v : null;
}

/** Context extracted from an event payload for the timeline row. */
export interface EventDetail {
  /** A file path touched by the event, if any. */
  file: string | null;
  /** A shell command, if any. */
  command: string | null;
  /** A tool / MCP name, if any. */
  tool: string | null;
  /** The stored prompt excerpt (opt-in capture), if any. */
  prompt: string | null;
  /** True when a prompt was captured as a hash only (excerpt disabled). */
  promptHashed: boolean;
  /** A command/test exit code, if present. */
  exitCode: number | null;
}

export function eventDetail(e: StoredEvent): EventDetail {
  const p = asRecord(e.payload);
  const exit = p["exit_code"];
  return {
    file: str(p["file"]) ?? str(p["file_path"]) ?? str(p["path"]),
    command: str(p["command"]) ?? str(p["cmd"]),
    tool: str(p["tool_name"]) ?? str(p["tool"]),
    prompt: str(p["prompt_excerpt"]),
    promptHashed: !str(p["prompt_excerpt"]) && !!str(p["prompt_hash"]),
    exitCode: typeof exit === "number" ? exit : null,
  };
}

export interface SessionStats {
  count: number;
  /** Span between the first and last event, in ms (0 for <2 events). */
  durationMs: number;
  /** Distinct actors, in first-seen order. */
  actors: string[];
  /** Distinct files touched. */
  files: number;
  /** Distinct event categories present. */
  categories: Category[];
  /** Number of captured prompts. */
  prompts: number;
}

export function sessionStats(events: StoredEvent[]): SessionStats {
  const actors: string[] = [];
  const files = new Set<string>();
  const categories = new Set<Category>();
  let prompts = 0;
  let min = Infinity;
  let max = -Infinity;
  for (const e of events) {
    if (!actors.includes(e.actor)) actors.push(e.actor);
    const cat = eventCategory(e.type);
    categories.add(cat);
    if (cat === "prompt") prompts++;
    const d = eventDetail(e);
    if (d.file) files.add(d.file);
    const t = Date.parse(e.timestamp);
    if (!Number.isNaN(t)) {
      min = Math.min(min, t);
      max = Math.max(max, t);
    }
  }
  return {
    count: events.length,
    durationMs: events.length > 1 && max > min ? max - min : 0,
    actors,
    files: files.size,
    categories: [...categories],
    prompts,
  };
}

/** Format a duration (ms) compactly, e.g. `4m 12s`, `2h 5m`, `38s`. */
export function formatDuration(ms: number): string {
  if (ms <= 0) return "0s";
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}
