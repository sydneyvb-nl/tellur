import { describe, it, expect } from "vitest";
import { sourceLink } from "./source";

const ref = { path: "src/auth/session.ts", start: 10, end: 24, sha: "abc123" };

describe("sourceLink", () => {
  it("substitutes all placeholders", () => {
    const t = "https://github.com/acme/app/blob/main/{path}#L{start}-L{end}";
    expect(sourceLink(t, ref)).toBe(
      "https://github.com/acme/app/blob/main/src/auth/session.ts#L10-L24",
    );
  });
  it("substitutes the blob sha", () => {
    expect(sourceLink("https://h/{sha}", ref)).toBe("https://h/abc123");
  });
  it("returns null for an empty/absent template", () => {
    expect(sourceLink(null, ref)).toBeNull();
    expect(sourceLink("", ref)).toBeNull();
    expect(sourceLink(undefined, ref)).toBeNull();
  });
  it("rejects non-https templates (defence-in-depth)", () => {
    expect(sourceLink("javascript:alert(1)", ref)).toBeNull();
    expect(sourceLink("http://insecure/{path}", ref)).toBeNull();
  });
});
