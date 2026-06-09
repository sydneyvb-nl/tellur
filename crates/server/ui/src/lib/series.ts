// Pure helpers to shape the activity time-series for charts (unit-tested).

import type { ActivityBucket } from "./api";

export interface DayTotal {
  day: string;
  count: number;
}

/** Sum activity buckets per day, sorted ascending by day. */
export function dailyTotals(buckets: ActivityBucket[]): DayTotal[] {
  const byDay = new Map<string, number>();
  for (const b of buckets) {
    byDay.set(b.day, (byDay.get(b.day) ?? 0) + b.count);
  }
  return [...byDay.entries()]
    .map(([day, count]) => ({ day, count }))
    .sort((a, b) => (a.day < b.day ? -1 : a.day > b.day ? 1 : 0));
}

/** Largest single-day total (for chart scaling); 0 when empty. */
export function maxCount(totals: DayTotal[]): number {
  return totals.reduce((m, t) => Math.max(m, t.count), 0);
}
