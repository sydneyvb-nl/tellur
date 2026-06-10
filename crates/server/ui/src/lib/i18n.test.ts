import { describe, it, expect } from "vitest";
import { translate, normalizeLocale, nextLocale, localeKeys } from "./i18n";

describe("translate", () => {
  it("resolves a key in the requested locale", () => {
    expect(translate("en", "nav.overview")).toBe("Overview");
    expect(translate("nl", "nav.overview")).toBe("Overzicht");
  });
  it("interpolates {var} placeholders", () => {
    expect(translate("en", "common.by", { actor: "claude" })).toBe("by claude");
    expect(translate("nl", "repoDetail.events", { n: 3 })).toBe("3 events");
  });
  it("leaves unknown placeholders intact", () => {
    expect(translate("en", "common.by", {})).toBe("by {actor}");
  });
  it("returns the key itself when unknown everywhere", () => {
    expect(translate("nl", "does.not.exist")).toBe("does.not.exist");
    expect(translate("en", "does.not.exist")).toBe("does.not.exist");
  });
});

describe("normalizeLocale", () => {
  it("accepts known locales, defaults the rest to en", () => {
    expect(normalizeLocale("nl")).toBe("nl");
    expect(normalizeLocale("en")).toBe("en");
    expect(normalizeLocale("fr")).toBe("en");
    expect(normalizeLocale(null)).toBe("en");
  });
});

describe("nextLocale", () => {
  it("cycles en → nl → en", () => {
    expect(nextLocale("en")).toBe("nl");
    expect(nextLocale("nl")).toBe("en");
  });
});

describe("catalog parity", () => {
  it("en and nl define exactly the same keys (no gaps or extras)", () => {
    const en = new Set(localeKeys("en"));
    const nl = new Set(localeKeys("nl"));
    const missingInNl = [...en].filter((k) => !nl.has(k));
    const extraInNl = [...nl].filter((k) => !en.has(k));
    expect(missingInNl).toEqual([]);
    expect(extraInNl).toEqual([]);
  });
});
