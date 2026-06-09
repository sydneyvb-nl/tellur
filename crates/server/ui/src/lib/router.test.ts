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
});
