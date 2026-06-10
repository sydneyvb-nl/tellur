import { describe, it, expect } from "vitest";
import { normalizeDensity, toggleDensity } from "./density";

describe("normalizeDensity", () => {
  it("accepts compact, defaults everything else to comfortable", () => {
    expect(normalizeDensity("compact")).toBe("compact");
    expect(normalizeDensity("comfortable")).toBe("comfortable");
    expect(normalizeDensity("bogus")).toBe("comfortable");
    expect(normalizeDensity(null)).toBe("comfortable");
  });
});

describe("toggleDensity", () => {
  it("flips between the two densities", () => {
    expect(toggleDensity("comfortable")).toBe("compact");
    expect(toggleDensity("compact")).toBe("comfortable");
  });
});
