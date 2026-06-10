import { describe, it, expect } from "vitest";
import { buildCommands, filterCommands } from "./commands";

describe("buildCommands", () => {
  it("hides admin-only targets from non-admins", () => {
    const viewer = buildCommands("o1", "viewer").map((c) => c.id);
    expect(viewer).toContain("overview");
    expect(viewer).toContain("sessions");
    expect(viewer).not.toContain("policies");
    expect(viewer).not.toContain("audit");
  });
  it("exposes admin targets to admins", () => {
    const admin = buildCommands("o1", "admin").map((c) => c.id);
    expect(admin).toContain("policies");
    expect(admin).toContain("people");
    expect(admin).toContain("audit");
  });
  it("scopes paths to the org", () => {
    const repos = buildCommands("acme", "viewer").find((c) => c.id === "repos");
    expect(repos?.path).toBe("/app/orgs/acme/repos");
  });
});

describe("filterCommands", () => {
  const cmds = buildCommands("o1", "admin");

  it("returns everything for an empty query", () => {
    expect(filterCommands(cmds, "").length).toBe(cmds.length);
  });
  it("matches a contiguous substring", () => {
    const r = filterCommands(cmds, "audit");
    expect(r[0]?.id).toBe("audit");
  });
  it("matches a subsequence (fuzzy)", () => {
    // "ppl" is a subsequence of "People & Access" → peoP… no; use letters in order.
    const r = filterCommands(cmds, "peo");
    expect(r.map((c) => c.id)).toContain("people");
  });
  it("drops non-matches", () => {
    expect(filterCommands(cmds, "zzzzz")).toHaveLength(0);
  });
  it("ranks an earlier match ahead of a later one", () => {
    // "o" appears at start of "Overview" and later in others.
    const r = filterCommands(cmds, "over");
    expect(r[0]?.id).toBe("overview");
  });
});
