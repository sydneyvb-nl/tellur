import { describe, it, expect } from "vitest";
import { sourceLink } from "./source";

const ref = { path: "src/auth/session.ts", start: 10, end: 24 };

describe("sourceLink", () => {
  it("substitutes the path/start/end placeholders", () => {
    const t = "https://github.com/acme/app/blob/main/{path}#L{start}-L{end}";
    expect(sourceLink(t, ref)).toBe(
      "https://github.com/acme/app/blob/main/src/auth/session.ts#L10-L24",
    );
  });
  it("URL-encodes path segments but keeps slashes", () => {
    const t = "https://h/{path}#L{start}";
    expect(sourceLink(t, { path: "docs/a#b.md", start: 1, end: 2 })).toBe(
      "https://h/docs/a%23b.md#L1",
    );
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
