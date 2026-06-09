// Pure formatting helpers (unit-tested). No DOM, no framework.

/** Format a 0..1 ratio as a whole-number percentage, e.g. 0.624 → "62%". */
export function pct(ratio: number): string {
  if (!Number.isFinite(ratio)) return "—";
  const clamped = Math.min(1, Math.max(0, ratio));
  return `${Math.round(clamped * 100)}%`;
}

/** Compact integer formatting, e.g. 1234 → "1,234". */
export function count(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return Math.round(n).toLocaleString("en-US");
}

/** Relative-time label for an RFC3339 timestamp, e.g. "3m ago", "2d ago". */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const secs = Math.max(0, Math.round((now - t) / 1000));
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}
