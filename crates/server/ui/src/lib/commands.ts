// Command-palette model. `buildCommands` produces the role-aware navigation
// targets for an org; `filterCommands` ranks them against a query. Both are pure
// and unit-tested; the palette component renders the result.

import {
  defaultPath,
  reposPath,
  sessionsPath,
  policiesPath,
  peoplePath,
  exportsPath,
  auditPath,
} from "./router";

export interface Command {
  id: string;
  label: string;
  /** Short context shown after the label (e.g. "admin"). */
  hint?: string;
  path: string;
  admin: boolean;
}

/** All navigable commands for an org, filtered to what the role may see. */
export function buildCommands(org: string, role: string): Command[] {
  const all: Command[] = [
    { id: "overview", label: "Overview", path: defaultPath(org), admin: false },
    { id: "repos", label: "Repositories", path: reposPath(org), admin: false },
    { id: "sessions", label: "Sessions", path: sessionsPath(org), admin: false },
    { id: "policies", label: "Policies", hint: "compliance", path: policiesPath(org), admin: true },
    { id: "people", label: "People & Access", hint: "admin", path: peoplePath(org), admin: true },
    { id: "exports", label: "Exports", hint: "admin", path: exportsPath(org), admin: true },
    { id: "audit", label: "Audit log", hint: "admin", path: auditPath(org), admin: true },
  ];
  return all.filter((c) => !c.admin || role === "admin");
}

/**
 * Rank commands by a subsequence match on the label (case-insensitive). An
 * empty query keeps the original order; non-matches are dropped. Earlier and
 * more contiguous matches rank higher.
 */
export function filterCommands(commands: Command[], query: string): Command[] {
  const q = query.trim().toLowerCase();
  if (!q) return commands;
  const scored: { c: Command; score: number }[] = [];
  for (const c of commands) {
    const score = subsequenceScore(c.label.toLowerCase(), q);
    if (score !== null) scored.push({ c, score });
  }
  // Lower score = better (earlier/tighter match); stable on ties.
  scored.sort((a, b) => a.score - b.score);
  return scored.map((s) => s.c);
}

/** Returns a penalty score for matching all of `q` as a subsequence of `text`, or null. */
function subsequenceScore(text: string, q: string): number | null {
  // Fast path: contiguous substring is the best kind of match.
  const idx = text.indexOf(q);
  if (idx !== -1) return idx;

  let ti = 0;
  let firstHit = -1;
  let gaps = 0;
  for (let qi = 0; qi < q.length; qi++) {
    const found = text.indexOf(q[qi]!, ti);
    if (found === -1) return null;
    if (firstHit === -1) firstHit = found;
    if (qi > 0 && found > ti) gaps += found - ti;
    ti = found + 1;
  }
  // Offset past contiguous matches; weight by start position and spread.
  return 100 + firstHit + gaps;
}
