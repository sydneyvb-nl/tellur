import { describe, it, expect } from "vitest";
import { parseRoute, routePath, defaultPath } from "./router";

describe("parseRoute", () => {
  it("returns null for the bare base (caller redirects to default org)", () => {
    expect(parseRoute("/app")).toBeNull();
    expect(parseRoute("/app/")).toBeNull();
  });
  it("parses an org-scoped overview route", () => {
    expect(parseRoute("/app/orgs/org_123/overview")).toEqual({
      name: "overview",
      org: "org_123",
    });
  });
  it("defaults the screen to overview", () => {
    expect(parseRoute("/app/orgs/org_123")).toEqual({
      name: "overview",
      org: "org_123",
    });
  });
  it("parses the repos list and a repo detail", () => {
    expect(parseRoute("/app/orgs/o1/repos")).toEqual({ name: "repos", org: "o1" });
    expect(parseRoute("/app/orgs/o1/repos/app")).toEqual({
      name: "repo",
      org: "o1",
      repo: "app",
    });
  });
  it("parses a file view with a nested path", () => {
    expect(parseRoute("/app/orgs/o1/repos/app/files/src/a.rs")).toEqual({
      name: "file",
      org: "o1",
      repo: "app",
      path: "src/a.rs",
    });
  });
  it("parses sessions list and detail", () => {
    expect(parseRoute("/app/orgs/o1/sessions")).toEqual({ name: "sessions", org: "o1" });
    expect(parseRoute("/app/orgs/o1/sessions/sess_1")).toEqual({
      name: "session",
      org: "o1",
      id: "sess_1",
    });
  });
  it("parses audit and exports routes", () => {
    expect(parseRoute("/app/orgs/o1/audit")).toEqual({ name: "audit", org: "o1" });
    expect(parseRoute("/app/orgs/o1/exports")).toEqual({ name: "exports", org: "o1" });
  });
  it("parses policies and people routes", () => {
    expect(parseRoute("/app/orgs/o1/policies")).toEqual({ name: "policies", org: "o1" });
    expect(parseRoute("/app/orgs/o1/people")).toEqual({ name: "people", org: "o1" });
  });
  it("flags unknown screens but keeps the org", () => {
    const r = parseRoute("/app/orgs/org_123/wat");
    expect(r).toMatchObject({ name: "unknown", org: "org_123" });
  });
  it("flags malformed paths", () => {
    expect(parseRoute("/app/nonsense")).toMatchObject({
      name: "unknown",
      org: null,
    });
  });
});

describe("routePath / defaultPath", () => {
  it("round-trips overview", () => {
    const r = parseRoute("/app/orgs/o1/overview")!;
    expect(routePath(r)).toBe("/app/orgs/o1/overview");
  });
  it("builds the default landing path", () => {
    expect(defaultPath("o1")).toBe("/app/orgs/o1/overview");
  });
  it("round-trips audit and exports", () => {
    expect(routePath(parseRoute("/app/orgs/o1/audit")!)).toBe("/app/orgs/o1/audit");
    expect(routePath(parseRoute("/app/orgs/o1/exports")!)).toBe("/app/orgs/o1/exports");
  });
  it("round-trips policies and people", () => {
    expect(routePath(parseRoute("/app/orgs/o1/policies")!)).toBe("/app/orgs/o1/policies");
    expect(routePath(parseRoute("/app/orgs/o1/people")!)).toBe("/app/orgs/o1/people");
  });
});
