import { describe, it, expect } from "vitest";
import { dailyTotals, maxCount } from "./series";

describe("dailyTotals", () => {
  it("sums per day and sorts ascending", () => {
    const totals = dailyTotals([
      { day: "2026-06-02", key: "a", count: 2 },
      { day: "2026-06-01", key: "a", count: 1 },
      { day: "2026-06-02", key: "b", count: 3 },
    ]);
    expect(totals).toEqual([
      { day: "2026-06-01", count: 1 },
      { day: "2026-06-02", count: 5 },
    ]);
  });
  it("handles empty", () => {
    expect(dailyTotals([])).toEqual([]);
    expect(maxCount([])).toBe(0);
  });
  it("finds the peak day", () => {
    expect(maxCount([{ day: "x", count: 4 }, { day: "y", count: 7 }])).toBe(7);
  });
});
