import { describe, it, expect } from "vitest";
import {
  eventCategory,
  eventDetail,
  sessionStats,
  formatDuration,
} from "./timeline";
import type { StoredEvent } from "./api";

function ev(partial: Partial<StoredEvent>): StoredEvent {
  return {
    seq: 1,
    id: "e1",
    repo_id: "r1",
    session_id: "s1",
    timestamp: "2026-06-12T10:00:00Z",
    type: "file.write",
    actor: "claude",
    payload: {},
    ...partial,
  };
}

describe("eventCategory", () => {
  it("classifies common event types", () => {
    expect(eventCategory("user.prompt")).toBe("prompt");
    expect(eventCategory("prompt.submitted")).toBe("prompt");
    expect(eventCategory("file.write")).toBe("file");
    expect(eventCategory("command.post_execute")).toBe("command");
    expect(eventCategory("tool.pre_call")).toBe("tool");
    expect(eventCategory("mcp.post_call")).toBe("tool");
    expect(eventCategory("git.commit")).toBe("git");
    expect(eventCategory("session.start")).toBe("session");
    expect(eventCategory("something.weird")).toBe("other");
  });
});

describe("eventDetail", () => {
  it("pulls a file path", () => {
    expect(eventDetail(ev({ payload: { file_path: "src/a.rs" } })).file).toBe("src/a.rs");
  });
  it("surfaces a captured prompt excerpt", () => {
    const d = eventDetail(ev({ type: "user.prompt", payload: { prompt_excerpt: "fix the bug" } }));
    expect(d.prompt).toBe("fix the bug");
    expect(d.promptHashed).toBe(false);
  });
  it("flags a hash-only prompt (excerpt disabled)", () => {
    const d = eventDetail(ev({ type: "user.prompt", payload: { prompt_hash: "sha256:x" } }));
    expect(d.prompt).toBeNull();
    expect(d.promptHashed).toBe(true);
  });
  it("reads a command and exit code", () => {
    const d = eventDetail(ev({ payload: { command: "cargo test", exit_code: 0 } }));
    expect(d.command).toBe("cargo test");
    expect(d.exitCode).toBe(0);
  });
});

describe("sessionStats", () => {
  it("summarizes actors, files, duration, and prompts", () => {
    const events = [
      ev({ id: "1", type: "user.prompt", actor: "alice", timestamp: "2026-06-12T10:00:00Z", payload: { prompt_excerpt: "hi" } }),
      ev({ id: "2", type: "file.write", actor: "claude", timestamp: "2026-06-12T10:01:00Z", payload: { file: "a.rs" } }),
      ev({ id: "3", type: "file.write", actor: "claude", timestamp: "2026-06-12T10:04:00Z", payload: { file: "a.rs" } }),
    ];
    const s = sessionStats(events);
    expect(s.count).toBe(3);
    expect(s.actors).toEqual(["alice", "claude"]);
    expect(s.files).toBe(1);
    expect(s.prompts).toBe(1);
    expect(s.durationMs).toBe(4 * 60 * 1000);
    expect(s.categories).toContain("prompt");
    expect(s.categories).toContain("file");
  });
  it("handles a single event (no duration)", () => {
    expect(sessionStats([ev({})]).durationMs).toBe(0);
  });
});

describe("formatDuration", () => {
  it("formats seconds, minutes, hours", () => {
    expect(formatDuration(0)).toBe("0s");
    expect(formatDuration(38_000)).toBe("38s");
    expect(formatDuration(252_000)).toBe("4m 12s");
    expect(formatDuration(7_500_000)).toBe("2h 5m");
  });
});
