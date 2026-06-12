import { describe, it, expect } from "vitest";
import { sourceLink, rawUrl, sliceLines, buildTemplates } from "./source";

describe("buildTemplates", () => {
  it("builds public GitHub link + raw templates", () => {
    const t = buildTemplates("github", "acme/app", "main", false);
    expect(t.link).toBe("https://github.com/acme/app/blob/main/{path}#L{start}-L{end}");
    expect(t.raw).toBe("https://raw.githubusercontent.com/acme/app/main/{path}");
  });
  it("uses the authenticated contents API for a private GitHub repo", () => {
    const t = buildTemplates("github", "acme/app", "dev", true);
    expect(t.raw).toBe("https://api.github.com/repos/acme/app/contents/{path}?ref=dev");
  });
  it("defaults the branch to main and trims slashes from the slug", () => {
    const t = buildTemplates("github", "/acme/app/", "", false);
    expect(t.link).toBe("https://github.com/acme/app/blob/main/{path}#L{start}-L{end}");
  });
  it("builds GitLab and Bitbucket templates", () => {
    expect(buildTemplates("gitlab", "g/p", "main", false).raw).toBe(
      "https://gitlab.com/g/p/-/raw/main/{path}",
    );
    expect(buildTemplates("bitbucket", "b/r", "main", false).link).toBe(
      "https://bitbucket.org/b/r/src/main/{path}#lines-{start}:{end}",
    );
  });
});

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

describe("rawUrl", () => {
  it("substitutes and encodes the path", () => {
    expect(rawUrl("https://raw/{path}", "docs/a#b.md")).toBe("https://raw/docs/a%23b.md");
  });
  it("rejects non-https / empty templates", () => {
    expect(rawUrl("http://raw/{path}", "a")).toBeNull();
    expect(rawUrl(null, "a")).toBeNull();
  });
});

describe("sliceLines", () => {
  const text = "one\ntwo\nthree\nfour";
  it("returns the inclusive 1-based window", () => {
    expect(sliceLines(text, 2, 3)).toEqual([
      { n: 2, text: "two" },
      { n: 3, text: "three" },
    ]);
  });
  it("clamps past end of file and rejects bad ranges", () => {
    expect(sliceLines(text, 4, 99)).toEqual([{ n: 4, text: "four" }]);
    expect(sliceLines(text, 0, 2)).toEqual([]);
    expect(sliceLines(text, 3, 2)).toEqual([]);
  });
});
