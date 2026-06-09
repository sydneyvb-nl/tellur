import { describe, it, expect } from "vitest";
import { pct, count, relativeTime } from "./format";

describe("pct", () => {
  it("rounds a ratio to a whole percent", () => {
    expect(pct(0.624)).toBe("62%");
    expect(pct(0)).toBe("0%");
    expect(pct(1)).toBe("100%");
  });
  it("clamps and guards non-finite", () => {
    expect(pct(1.5)).toBe("100%");
    expect(pct(-1)).toBe("0%");
    expect(pct(NaN)).toBe("—");
  });
});

describe("count", () => {
  it("groups thousands", () => {
    expect(count(1234)).toBe("1,234");
    expect(count(0)).toBe("0");
  });
});

describe("relativeTime", () => {
  const now = Date.parse("2026-06-08T12:00:00Z");
  it("buckets into human units", () => {
    expect(relativeTime("2026-06-08T11:59:30Z", now)).toBe("just now");
    expect(relativeTime("2026-06-08T11:40:00Z", now)).toBe("20m ago");
    expect(relativeTime("2026-06-08T09:00:00Z", now)).toBe("3h ago");
    expect(relativeTime("2026-06-06T12:00:00Z", now)).toBe("2d ago");
  });
  it("handles bad input", () => {
    expect(relativeTime("nope", now)).toBe("—");
  });
});
