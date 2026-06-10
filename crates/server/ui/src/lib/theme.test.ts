import { describe, it, expect } from "vitest";
import { resolveTheme, normalizePref, nextPref } from "./theme";

describe("resolveTheme", () => {
  it("follows the OS signal when preference is system", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });
  it("honours an explicit preference regardless of OS", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});

describe("normalizePref", () => {
  it("accepts known values and defaults the rest to system", () => {
    expect(normalizePref("light")).toBe("light");
    expect(normalizePref("dark")).toBe("dark");
    expect(normalizePref("system")).toBe("system");
    expect(normalizePref("bogus")).toBe("system");
    expect(normalizePref(null)).toBe("system");
  });
});

describe("nextPref", () => {
  it("cycles system → light → dark → system", () => {
    expect(nextPref("system")).toBe("light");
    expect(nextPref("light")).toBe("dark");
    expect(nextPref("dark")).toBe("system");
  });
});
