import { describe, it } from "node:test";
import assert from "node:assert/strict";

import {
  isValidSession,
  isValidEvent,
  isValidAttribution,
  generateSessionId,
  generateEventId,
  hashContent,
} from "../src/schemas/index.js";
import type { Session, TraceEvent } from "../src/schemas/types.js";

describe("Schema Validation", () => {
  describe("isValidSession", () => {
    it("accepts a valid session", () => {
      const session: Session = {
        schema: "tracegit.session.v1",
        id: "sess_abc123",
        repoId: "repo_xyz",
        workspaceId: "ws_001",
        startedAt: new Date().toISOString(),
        humanActor: { name: "Test", type: "human" },
        agent: { id: "claude-code", name: "Claude Code" },
        status: "completed",
        relatedCommits: [],
        relatedBranches: [],
        relatedPRs: [],
      };
      assert.equal(isValidSession(session), true);
    });

    it("rejects object with wrong schema", () => {
      assert.equal(isValidSession({ schema: "wrong" }), false);
    });

    it("rejects null", () => {
      assert.equal(isValidSession(null), false);
    });

    it("rejects non-object", () => {
      assert.equal(isValidSession("not an object"), false);
    });
  });

  describe("isValidEvent", () => {
    it("accepts a valid event", () => {
      const event: TraceEvent = {
        schema: "tracegit.event.v1",
        id: "evt_001",
        sessionId: "sess_abc",
        timestamp: new Date().toISOString(),
        type: "file.write",
        actor: "agent",
        payload: { file: "test.ts" },
      };
      assert.equal(isValidEvent(event), true);
    });

    it("rejects object missing required fields", () => {
      assert.equal(isValidEvent({ schema: "tracegit.event.v1" }), false);
    });
  });

  describe("isValidAttribution", () => {
    it("accepts a valid attribution", () => {
      const attr = {
        schema: "tracegit.attribution.v1",
        filePath: "src/test.ts",
        gitBlobSha: "abc123",
        ranges: [],
        updatedAt: new Date().toISOString(),
      };
      assert.equal(isValidAttribution(attr), true);
    });

    it("rejects attribution without ranges", () => {
      assert.equal(
        isValidAttribution({
          schema: "tracegit.attribution.v1",
          filePath: "test.ts",
          gitBlobSha: "abc",
        }),
        false
      );
    });
  });
});

describe("ID Generation", () => {
  it("generates unique session IDs", () => {
    const id1 = generateSessionId();
    const id2 = generateSessionId();
    assert.match(id1, /^sess_/);
    assert.match(id2, /^sess_/);
    assert.notEqual(id1, id2);
  });

  it("generates unique event IDs", () => {
    const id1 = generateEventId();
    const id2 = generateEventId();
    assert.match(id1, /^evt_/);
    assert.match(id2, /^evt_/);
    assert.notEqual(id1, id2);
  });
});

describe("Hashing", () => {
  it("produces a consistent SHA-256 hash", async () => {
    const content = "hello world";
    const hash = await hashContent(content);
    assert.match(hash, /^[a-f0-9]{64}$/);

    // Same input → same hash
    const hash2 = await hashContent(content);
    assert.equal(hash, hash2);
  });

  it("produces different hashes for different content", async () => {
    const hash1 = await hashContent("content a");
    const hash2 = await hashContent("content b");
    assert.notEqual(hash1, hash2);
  });
});
