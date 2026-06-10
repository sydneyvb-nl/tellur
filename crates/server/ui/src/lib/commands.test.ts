import { describe, it, expect } from "vitest";
import { buildCommands, filterCommands, type ResolvedCommand } from "./commands";
import { translate } from "./i18n";

/** Resolve labels (as the palette does) so filtering matches displayed text. */
function resolved(org: string, role: string): ResolvedCommand[] {
  return buildCommands(org, role).map((c) => ({
    ...c,
    label: translate("en", c.labelKey),
  }));
}

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
  const cmds = resolved("o1", "admin");

  it("returns everything for an empty query", () => {
    expect(filterCommands(cmds, "").length).toBe(cmds.length);
  });
  it("matches a contiguous substring", () => {
    const r = filterCommands(cmds, "audit");
    expect(r[0]?.id).toBe("audit");
  });
  it("matches a subsequence (fuzzy)", () => {
    const r = filterCommands(cmds, "peo");
    expect(r.map((c) => c.id)).toContain("people");
  });
  it("drops non-matches", () => {
    expect(filterCommands(cmds, "zzzzz")).toHaveLength(0);
  });
  it("ranks an earlier match ahead of a later one", () => {
    const r = filterCommands(cmds, "over");
    expect(r[0]?.id).toBe("overview");
  });
});
