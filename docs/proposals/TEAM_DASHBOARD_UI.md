# Proposal: Tellur Team Dashboard (Web UI)

**Status:** Design proposal — **not implemented**. Defines product scope, UX, a
visual design system, the API surface it requires, runtime architecture, and a
phased delivery plan for a full-fledged team dashboard served by the self-hosted
hub (`tellur-server`).
**Owner:** product + design (this doc) · **Roadmap:** PROJECT_STATUS.md #8 (Tier 1/2)
· **Builds on:** `docs/proposals/TEAM_SERVER_MODE.md`,
`docs/proposals/TEAM_SERVER_IMPLEMENTATION.md`
**Last updated:** 2026-06-08

> This is a plan, not shipped behavior. It is written so any contributor (or
> agent) can pick up a milestone and build it without re-deriving the product
> thinking. Nothing here is implemented yet. No pricing/commercial detail is in
> scope (public repo).

---

## 0. TL;DR

The hub already aggregates AI code provenance (events, attribution, policy,
audit, exports, RBAC, SSO, SCIM, durable jobs). Today the only browser surface is
a single static **session-replay** page (`web/index.html`) pointed at the local
daemon. This proposal designs the **team dashboard**: the multi-repo,
multi-contributor governance and insight UI that turns the hub's data into
answers for four named personas.

The north-star: **make "how AI participated" legible at a glance for a whole
org, and one click from glance to evidence.** Git shows *what* changed; Tellur's
dashboard shows *how AI participated* — and proves it.

Three hard design constraints shape everything below:

1. **Trust is the product.** The UI must feel like an audit-grade instrument, not
   a marketing dashboard. Every number links to verifiable evidence (the
   hash-chained event log). Nothing is asserted without a drill-down.
2. **Local-first dignity.** The dashboard is *optional* aggregation. It never
   implies the cloud is required, never hides the export, never locks data in.
3. **Not generic.** It must not read as a templated admin panel. A bespoke design
   system (typography, restraint, density, a single confident accent) earns
   credibility with senior engineers and security reviewers who distrust
   over-styled tools.

---

## 1. Research

### 1.1 Personas (primary)

Derived from `TEAM_SERVER_MODE.md §3` (segments) and `README.md` (audiences),
sharpened into people who will actually open the dashboard.

**P1 — "Maya", Engineering Manager / Tech Lead (SMB scale-up, team of 12).**
Primary buyer of the team view. Owns delivery and is newly accountable for "how
much of our code is AI-assisted and is that safe." Lives in dashboards 10 min/day.
Not a security specialist. Wants trend lines, hotspots, and "is anything on fire."
- *Success:* can answer her VP's "what % of our shipped code is AI-generated and
  is it reviewed?" in under 30 seconds, with a link she can forward.
- *Failure:* a wall of raw events; numbers she can't trust or explain.

**P2 — "Dev/Sofia", Senior Engineer / Reviewer (any segment).**
Opens the dashboard from a PR or a Slack link to understand a *specific* change:
who/what produced these lines, what model, was it tested, does policy flag it.
Power user; keyboard-first; allergic to fluff and to tools that slow her down.
- *Success:* from a repo or a file, reaches line-level attribution + the session
  that produced it in two clicks; verifies the chain.
- *Failure:* the dashboard duplicates Git/GitHub badly or can't deep-link.

**P3 — "Raj", Security / Compliance / Platform (corporate, on-prem/VPC).**
Cares about the audit trail, policy compliance across repos, SLSA/SPDX evidence,
RBAC correctness, and that **no prompts leak**. Will scrutinize the tool itself.
- *Success:* org-wide policy compliance status, a tamper-evident audit log he can
  filter and export, and one-click compliance bundles per repo/release.
- *Failure:* anything that looks like it phones home, or evidence he can't export.

**P4 — "Lena", Org Admin / IT (corporate).**
Sets up SSO/SCIM, manages members and roles, owns retention/backup. Visits rarely
but every visit is high-stakes.
- *Success:* sees who has access (and why — via SCIM group → role mapping),
  provisions/deprovisions confidently, confirms SSO is wired correctly.
- *Failure:* opaque role state; can't tell why someone is an admin.

**Anti-persona:** the casual viewer who wants vanity metrics. We do not optimize
for "AI usage leaderboards" framed as productivity scores — that invites misuse
(surveillance) and erodes trust with engineers. Activity is framed as
*provenance and risk*, never individual performance ranking.

### 1.2 Jobs To Be Done (job stories)

Format: *When [situation], I want to [motivation], so I can [outcome].*

Governance / insight (P1, P3)
- When I start my week, I want to see AI involvement and review-gap trends across
  all repos, so I can spot where un-reviewed AI code is accumulating.
- When my leadership asks "how much is AI-written and is it safe," I want a
  shareable, defensible summary, so I can answer without a data project.
- When a release goes out, I want org-wide SLSA/SPDX evidence in one place, so I
  can satisfy supply-chain/audit requirements.

Investigation (P2, P3)
- When I review a risky change, I want to trace specific lines to the agent,
  model, prompt hash, tests, and session, so I can judge whether to trust it.
- When something looks wrong, I want to verify the evidence is tamper-evident, so
  I can rely on it in an incident or audit.
- When I get a Slack link to a repo/file/session, I want it to open exactly there,
  so the dashboard fits how teams actually communicate.

Policy & access (P3, P4)
- When we change org policy, I want to see which repos comply and which don't, so
  enforcement isn't theoretical.
- When someone joins/leaves, I want their access to reflect their IdP groups
  automatically, and I want to *see* why they have a role, so access is auditable.

Cross-cutting
- When I land anywhere in the dashboard, I want it to be obviously fast and
  trustworthy, so I keep using it instead of exporting CSVs.

### 1.3 Use cases → screens (traceability)

| # | Use case | Persona | Primary screen |
|---|----------|---------|----------------|
| U1 | Org pulse: AI involvement + review gaps, trend | P1 | Overview |
| U2 | Per-repo health & hotspots | P1, P2 | Repos → Repo detail |
| U3 | Line-level attribution drill-down + verify | P2, P3 | Repo → File / Session |
| U4 | Session replay (timeline of an AI session) | P2 | Session detail |
| U5 | Policy compliance across repos | P3 | Policies |
| U6 | Audit log: filter, inspect, export, verify chain | P3 | Audit |
| U7 | Compliance evidence (SLSA/SPDX, event/audit bundles) | P3 | Exports |
| U8 | People & access (roles, SCIM groups, SSO state) | P4 | People / Access |
| U9 | Deep-link from PR/Slack to any of the above | P2 | (routing) |

### 1.4 Competitive / landscape notes (what to avoid and steal)

- **GitHub Insights / generic BI dashboards:** familiar but shallow; engineers
  distrust "productivity" charts. *Steal:* deep-linking, PR-native entry points.
  *Avoid:* per-person leaderboards as the headline.
- **Observability tools (Grafana/Datadog):** density and drill-down done well.
  *Steal:* time-range control, facet filters, "every panel is a query you can
  inspect." *Avoid:* configuration overload; we ship opinionated views.
- **Security/compliance consoles (Snyk, Semgrep, SLSA tooling):** evidence-first
  framing, severity language. *Steal:* "finding → evidence → action" flow,
  exportable attestations. *Avoid:* alert fatigue / noisy red.
- **Linear / Vercel / Stripe dashboards (craft bar):** restraint, typography,
  motion, keyboard. *Steal:* the *feel* — calm, fast, confident, monochrome with
  one accent. This is the visual bar we hold ourselves to.

### 1.5 Key insights (the spine of the design)

1. **Evidence, not vibes.** Every aggregate is a link to its underlying,
   verifiable rows. The tagline "Git shows what; Tellur shows how AI participated"
   becomes a literal interaction: drill from a % to the exact lines and session.
2. **Two speeds of user.** P1/P4 skim (cards, trends, status). P2/P3 hunt
   (filters, tables, drill-downs, keyboard). The IA must serve both without
   compromise — overview that *invites* drill-down rather than replacing it.
3. **Risk framing beats productivity framing.** Headline metrics are about
   *coverage and review*, e.g. "AI-attributed lines under review," not "lines per
   dev." This is both more useful and more ethical, and differentiates us.
4. **Provenance is temporal.** Activity, review gaps, and policy compliance only
   mean something *over time*. Time-range is a first-class, global control.
5. **The audit log is a feature, not a footer.** For P3 it may be the main reason
   to adopt; it deserves a real, filterable, verifiable surface.

---

## 2. Product scope

### 2.1 Goals
- Turn hub data into the nine use cases above with a fast, trustworthy,
  keyboard-friendly UI served same-origin by the hub.
- Make every aggregate drillable to verifiable evidence.
- Be deep-linkable everywhere (PR/Slack/email entry points).
- Be accessible (WCAG 2.2 AA), responsive (down to tablet; usable read-only on
  phone), themeable (dark default + light), and i18n-ready.

### 2.2 Non-goals (initial)
- No write-heavy provenance editing (the dashboard reads + governs; capture stays
  in CLI/editors).
- No mandatory cloud, no third-party analytics, no per-person productivity scores.
- No replacement for Git/PR review; we *augment* and link out.
- No mobile-first/native app (responsive web only).

### 2.3 Success metrics (privacy-respecting, see §10)
- **Activation:** % of orgs with ≥1 weekly-active non-admin viewer within 2 weeks
  of enabling SSO.
- **Time-to-answer (U1):** median time from Overview load to opening a repo/file
  drill-down (proxy for "the overview invited investigation").
- **Trust signal:** % of sessions that reach a `verify` / evidence view.
- **Compliance value:** # of export bundles generated per org per release cycle.
- **Performance:** see budgets in §8.6 (these are product metrics, not vanity).
- All measured via self-hosted, aggregate, opt-in telemetry only (§10).

---

## 3. Information architecture

### 3.1 Top-level navigation (left rail, collapsible)

```
Tellur ▸ <Org name>            [ time range ▾ ]   [ ⌘K ]   [ avatar ▾ ]
────────────────────────────────────────────────────────────────────────
◆ Overview            ← U1  org pulse + trends
▣ Repositories        ← U2/U3 list → repo detail → file/attribution
◷ Sessions            ← U4  cross-repo session list → replay
⚖ Policies            ← U5  org policies + per-repo compliance
⤓ Exports             ← U7  durable export jobs (SLSA/SPDX/event/audit)
≣ Audit log           ← U6  tamper-evident trail (admin)
☷ People & Access     ← U8  members, roles, SCIM groups, SSO state (admin)
────────────────────────────────────────────────────────────────────────
settings · docs · sign out
```

Nav items gate by role: Overview/Repos/Sessions/Policies are viewer+ (read);
**Exports, Audit, and People & Access are admin-only** (the export surface —
org event/audit jobs, job polling, and per-repo SLSA/SPDX — is admin-only in the
hub today, so the IA matches the API rather than 403-ing a viewer who follows
it). Items the caller can't access are hidden, not shown-disabled (avoid
teasing).

### 3.2 Routes (URL = state; everything deep-linkable)

All SPA routes are **served under `/app`** (the hub only falls back to
`index.html` for `GET /app/*`; root paths stay reserved for `/auth`, `/v1`,
`/healthz`, `/readyz`, `/metrics`) and are **org-scoped from day one** (decision
§12.5) so multi-org never requires a repaint:

```
/app                                              → redirect to /app/orgs/:defaultOrg/overview
/app/orgs/:org/overview?range=30d
/app/orgs/:org/repos?q=&sort=&range=30d
/app/orgs/:org/repos/:repo                        repo detail (tabs: activity | files | contributors | exports)
/app/orgs/:org/repos/:repo/files/:path*           file attribution (line-level)
/app/orgs/:org/sessions?repo=&actor=&type=&range=
/app/orgs/:org/sessions/:sessionId                session replay timeline
/app/orgs/:org/policies
/app/orgs/:org/policies/:name                      policy body + compliance snapshot
/app/orgs/:org/exports        (admin)              job list
/app/orgs/:org/exports/:jobId (admin)              job status + result viewer
/app/orgs/:org/audit          (admin)  ?actor=&action=&range=&cursor=
/app/orgs/:org/people         (admin)              members + roles
/app/orgs/:org/people/groups  (admin)              SCIM groups → role mapping
/app/orgs/:org/settings                            org/SSO/SCIM status (read), theme, prefs
```

Single-org installs redirect `/app` → the default org and may hide the org
switcher, but the org-scoped path shape is permanent. Global controls present on
every screen: **time range** (affects time-scoped views), **command palette
(⌘K)** for jump-to-repo/session/action, and the **org context** (switcher for
multi-org; hidden for single-org).

### 3.3 Entry points (deep-linking is a feature)
- From CLI/PR report: a `tellur:` URL or `--dashboard-url` prints a link to the
  exact repo/file/session.
- From Slack/email: links resolve post-auth (SSO redirect preserves the target).
- `⌘K` palette: fuzzy jump to any repo, recent session, or action.

---

## 4. Screen specs

Each screen lists: purpose, who, primary data, layout (ASCII), states
(loading/empty/error/no-access), and interactions. Wireframes are intent, not
pixels.

### 4.1 Overview (U1 — the room's first impression)

Purpose: org pulse in <10s for P1; an invitation to drill for P2/P3.
Data: `report` rollup + time-series activity + review-gap + top repos + recent
high-signal events. (Several of these need new endpoints — see §6.)

```
┌ Overview ──────────────────────────────────  range: Last 30 days ▾ ┐
│                                                                      │
│  AI-attributed        Under review        Policy findings   Sessions │
│  ▰▰▰▰▱ 62%             ▰▰▰▱▱ 71%           3 open            128      │
│  of changed lines      of AI lines         across 2 repos    this wk  │
│  ▲ 4pts vs prev        ▼ 6pts ⚠            ● 1 high                   │
│                                                                      │
│  Activity over time                                   [ by origin ▾ ]│
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │   ▁▂▃▅▆▇▆▅▃▂  AI   ·····  human   (stacked area, hover = day)    │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  Repos needing attention            Recent high-signal activity      │
│  ┌───────────────────────────┐      ┌──────────────────────────────┐│
│  │ payments-svc  87% AI  ⚠42% │      │ ⚠ policy: secrets touched … ││
│  │ auth-core     64% AI  ✓88% │      │ ● review-gap on payments-svc ││
│  │ web-app       …            │      │ ◷ session sess_… (claude)    ││
│  └───────────────────────────┘      └──────────────────────────────┘│
└──────────────────────────────────────────────────────────────────────┘
```

- "Needing attention" ranks by *review gap × AI share × recency* (a composed
  risk score, transparent on hover), not by raw volume.
- Every tile is a link (e.g. "Under review" → Sessions filtered to unreviewed).
- States: skeleton cards on load; empty = "No activity yet — connect a repo"
  with the exact CLI command; error = inline retry, never a blank page.

### 4.2 Repositories + Repo detail (U2/U3)

List: searchable/sortable table (name, AI share, review coverage, last activity,
policy status, contributors). Repo detail tabs:
- **Activity:** time-series + event stream (filter by type/actor/session).
- **Files:** tree/list with per-file AI coverage + review state → file view.
- **Contributors:** per-actor rollup *for this repo* (provenance, not ranking).
- **Exports:** per-repo SLSA/SPDX generation + history.

File view (U3 — the money shot): the file with a **provenance gutter** — each
line tinted by origin (AI/human/mixed) with confidence; click a range to open a
side panel showing agent, model, prompt hash, evidence strength, tests
run/passed, policy tags, and a link to the originating session + a **Verify
chain** action.

**Source-text contract (important).** The hub's persisted attribution stores
`file_path`, `git_blob_sha`, and ranges — **not** the source text. The gutter
therefore renders in one of two modes, in priority order:
1. **Metadata-first (default, always available):** render the range map without
   source — a compact list/heatmap of attributed line ranges with their
   provenance, and "open in Git" deep links built from the repo's remote +
   `git_blob_sha` (no source ever leaves the user's Git host). This honors the
   "prompts/source don't leave the machine" principle and needs no new content
   API.
2. **Full source overlay (opt-in):** if an org configures a content source, fetch
   text by `git_blob_sha` to render real lines under the gutter. This requires a
   new contract — either a Git-provider link/fetch integration or a content
   endpoint — and is gated behind explicit org configuration. Tracked as API gap
   **A12**; not required for D2.
The wireframe above shows mode 2; D2 ships mode 1 and treats mode 2 as a
follow-up so the gutter is useful without storing or proxying source code.

```
┌ payments-svc / src/charge.rs ─────────────── ⓘ provenance ▸ verify ─┐
│  ◧ AI   ◨ human   ▨ mixed        coverage: 87% AI · 42% reviewed     │
│ ───────────────────────────────────────────  ┌ range r_… ──────────┐│
│ 12 ◧  let amount = parse(req)?;               │ origin    AI (0.95) ││
│ 13 ◧  charge(gateway, amount)                 │ agent     claude    ││
│ 14 ◨  // manual guard added in review         │ model     opus-4.x  ││
│ 15 ▨  if amount > LIMIT { reject() }          │ tests     ✓ 3 passed││
│                                                │ policy    payments ⚠││
│                                                │ session   sess_… ▸  ││
│                                                │ [ verify chain ]    ││
└───────────────────────────────────────────────└─────────────────────┘
```

### 4.3 Sessions + Session detail (U4)

Cross-repo session list (filters: repo, actor, type, range, "unreviewed only").
Detail reuses and elevates the *existing* `web/index.html` timeline: a vertical
event timeline (file reads/writes, commands, prompts-as-hashes), an AI/human
split bar, diffs, and the model/prompt-hash metadata — now backed by the hub and
deep-linkable. This is the one screen we partly already have; it becomes a
component, not a separate page.

### 4.4 Policies (U5)

Left: org policies (name, version, updated, source). Right: selected policy body
(read-only, syntax-highlighted YAML) + a **compliance snapshot table**: each repo
× pass/fail/not-evaluated, with the failing rules expandable to the offending
events/lines. Compliance is **durable-job-backed and cached as timestamped
snapshots** per `(org, repo, policy version)` (decision §12.4): the UI reads the
latest snapshot (showing its "evaluated at" age) and an admin can trigger a
re-evaluation (which enqueues a job and re-polls). "Distribute" affordance
documents `tellur policy pull`. P3's core "is enforcement real" view.

### 4.5 Exports (U7)

Admin-only. Two export shapes, surfaced honestly because the hub treats them
differently today:
- **Org event / audit bundles → durable jobs.** "New export" enqueues, the job
  table shows kind/requested-by/status/age, and the detail **polls**
  (`GET /jobs/{id}`) with a calm progress state, then renders/downloads the
  result. This is the UI contract for the shipped durable-job backend.
- **Per-repo SLSA / SPDX → synchronous.** These are immediate
  `GET .../repos/{repo}/export/slsa|spdx` responses (no `job_id`); the UI streams
  the download directly and shows an inline result, **not** a job poller.
If we later want consistency (history, large repos), API gap **A13** would add
job-backed SLSA/SPDX variants; until then the UI must not assume a `job_id` for
them.

### 4.6 Audit log (U6 — P3's anchor)

Full-height virtualized table over the hash-chained audit trail: timestamp,
actor, action, detail, entry hash. Faceted filters (actor, action, range),
cursor pagination, row → detail drawer, a persistent **"chain verified ✓"**
indicator (re-checked server-side), and **Export** (reuses the audit export job).
Read-only by construction; communicates tamper-evidence visibly.

### 4.7 People & Access (U8 — P4)

Members table (name, email, org role, SSO bound?, last seen) + **why**: a role
provenance popover ("admin via SCIM group `tellur-admin`" vs "set directly").
SCIM groups → role-mapping view. SSO/SCIM **status** panel (issuer reachable,
last SCIM sync, token age) — read-only health, not secrets. Per-repo role grants
manageable here for admins. Makes access *auditable*, which is the JTBD.

### 4.8 Settings
Org info, theme (system/dark/light), density, SSO/SCIM read-only status, and a
clearly-labeled **Export everything** shortcut (reinforces "no lock-in").

---

## 5. UX & interaction principles

- **Drill, don't dead-end.** Every number/chart segment is a link to its rows.
  Breadcrumbs + back always restore filter state (URL-encoded).
- **States are designed, not afterthoughts.** Every data surface specifies four
  states: loading (skeleton, not spinner-on-blank), empty (with the exact next
  action/CLI command), error (inline, retry, human message — reuse the hub's
  RFC 9457 problem detail), no-access (hidden or a clean 403 explainer).
- **Time range is global and persistent** (URL + last-used). Comparisons ("vs
  previous period") are first-class.
- **Async is honest.** Exports are jobs; the UI shows queued→running→done with
  real status, never a fake spinner. Long lists virtualize; nothing blocks.
- **Keyboard-first.** `⌘K` palette; `j/k` row nav; `/` focus search; `g o / g r`
  go-to nav; `?` shortcuts cheatsheet. P2/P3 should rarely need the mouse.
- **Density toggle.** Comfortable (P1) vs compact (P2/P3 tables).
- **No dark patterns.** No nags, no "upgrade" interstitials, no hiding export.
- **Copy is precise and calm.** Security/PM voice: "62% of changed lines are
  AI-attributed," not "🚀 AI supercharged your team!" (see §7.5).

---

## 6. API gaps (the dashboard is API-first)

The UI must not invent a parallel data path. Today's hub gives us: `/v1/me`,
`/v1/orgs/{org}/dashboard` (report + recent events), `/repos`, per-repo
`/events` (cursor by seq), `/report`, `/policies[/{name}]`, `/jobs/{id}`, export
job creation, per-repo SLSA/SPDX, RBAC grant endpoints, SCIM (token-scoped, not
session). **Gaps that must be built first** (Phase 0), each small and testable,
all session-cookie/Bearer auth + tenant-scoped + audited where it mutates:

| # | New/changed endpoint | Feeds | Notes |
|---|----------------------|-------|-------|
| A1 | `GET /v1/orgs/{org}/activity?range=&bucket=day&group_by=origin\|actor\|type` | Overview/Repo trends | Time-bucketed counts; the single biggest gap (no time-series today). |
| A2 | `GET /v1/orgs/{org}/members` (admin) | People, Contributors | Session-auth member list (SCIM list needs a SCIM token). Include role + role-source + bound-SSO + last-seen. |
| A3 | `GET /v1/orgs/{org}/repos/{repo}` | Repo detail header | Single-repo summary: AI share, review coverage, contributors, last activity, policy status. |
| A4 | `GET /v1/orgs/{org}/repos/{repo}/attributions[?path=]` | Files / file view | Read endpoint over stored attribution (exists in `Store`, only exposed via SLSA/SPDX today). Powers the gutter. |
| A5 | `GET /v1/orgs/{org}/repos/{repo}/events?type=&actor=&session=&before=&after=` | Activity/Sessions filters | Add facets to the existing seq-paginated list. |
| A6 | `GET /v1/orgs/{org}/sessions[?repo=&actor=&range=]` + `/sessions/{id}` | Sessions list/replay | Cross-repo session index (group events by `session_id`). |
| A7 | `GET /v1/orgs/{org}/audit?actor=&action=&range=&cursor=` (admin) | Audit log | Paginated **read** (today audit is export-only via job). |
| A8 | `POST /v1/orgs/{org}/policies/compliance` (enqueue) + `GET .../policies/compliance` (latest snapshot, +per-repo) (admin) | Policies | Durable-job-backed; persists **timestamped compliance snapshots** per `(org, repo, policy version)`. UI reads the latest snapshot; admin can trigger re-eval (§12.4). |
| A9 | `GET /v1/orgs/{org}/overview` | Overview | Optional: one composed payload (review-gap, risk-ranked repos, headline deltas) to keep the landing screen one round-trip. Could extend the existing `/dashboard`. |
| A10 | `GET /v1/orgs/{org}/sso-status` (admin) | People/Settings | Read-only health (issuer reachable, last SCIM activity, token age). No secrets. |
| A11 | `GET /v1/orgs/{org}/groups` (admin, **session-auth**) | People & Access | Session-auth read of SCIM groups + members + derived role mapping. The existing `/scim/v2/Groups` needs a SCIM bearer token, which the SPA must not hold — so the People screen needs this browser-auth mirror. |
| A12 | File source contract (opt-in): Git-provider link/fetch **or** `GET .../repos/{repo}/blob/{git_blob_sha}` | File view (full-source mode) | Persisted attribution has no source text. Only needed for the opt-in full-source gutter (§4.2 mode 2); D2 ships metadata-first without it. |
| A13 | Job-backed SLSA/SPDX variants (optional) | Exports | Only if we want export history/large-repo handling; per-repo SLSA/SPDX are synchronous today and the UI treats them so (§4.5). |

Cross-cutting API requirements: consistent cursor pagination + `total` where
cheap; `ETag`/`Last-Modified` for cacheable reads; uniform error model (already
RFC 9457); per-endpoint role gating mirrored in the UI; **the dashboard adds no
new trust boundary** — it is a same-origin client of these endpoints.

**Review-gap definition (decision §12.1, drives A1/A3/A9).** An AI-attributed
range counts as **reviewed** only when *all* hold: it has an explicit human
review state; a human `reviewer` distinct from the producing agent/actor; a
`reviewed_at` timestamp **after the latest AI modification** of that range; and —
where policy requires tests — passing test evidence (`tests_passed`). **Review
coverage** = `reviewed_ai_lines / total_ai_attributed_lines`; **review gap** is
its inverse. This is computed server-side from `AttributionRange`
(`state`/`reviewer`/`reviewed_at`/`tests_passed`) and is the single source for
the headline metric — the UI never recomputes it.

---

## 7. Visual design system

The goal: an instrument that senior engineers trust on sight. Restraint,
typography, density, one accent. Concrete tokens below so it is buildable and
consistent — and so it does **not** look like a default component-library admin.

### 7.1 Brand alignment
Tellur's mark uses a calm teal-green (`tellur.dev` accent ≈ `#69d3a5`). The
current `web/index.html` uses a GitHub-dark palette with a blue accent — we
**re-base on the brand green** as the single accent so the product feels like
Tellur, not GitHub. Monochrome surfaces + one green accent + semantic status
colors only.

### 7.2 Color tokens (dark default; light is a first-class theme)

```
                          Dark            Light
--bg            canvas    #0B0E11         #FBFCFD
--surface       panel     #14181D         #FFFFFF
--surface-2     raised    #1B2027         #F3F5F7
--border                  #262C34         #E3E7EB
--text                    #E7ECF1         #11151A
--text-muted              #9AA4AF         #5B6671
--accent        Tellur    #69D3A5         #1Fae84   (AA-tuned per theme)
--accent-weak             rgba(105,211,165,.14)
status: --ok #3FB07F  --warn #E0A33A  --risk #E5614C  --info #5AA9E6
provenance: --ai #8B7BF0  --human #3FB07F  --mixed #E0A33A
```

Rules: exactly **one** accent (green) for primary action/active nav; status
colors only for status; provenance colors are a separate, consistent legend used
wherever origin appears (gutter, charts, badges). Never use accent for decoration.
All pairings meet WCAG AA (4.5:1 text / 3:1 large+UI); both themes are audited,
not auto-derived.

### 7.3 Typography
- UI: **Inter** (or system fallback) — tight, neutral, excellent at small sizes.
- Code/IDs/hashes/metrics: **JetBrains Mono** (or `ui-monospace`) — reinforces
  the "instrument" feel; all hashes, repo ids, line numbers, and KPI numerals are
  mono (tabular figures for aligned tables).
- Type scale (1.20 ratio): 12 / 13 / 14(base) / 16 / 20 / 26 / 33. Numbers in KPI
  tiles step up to 33 with `font-variant-numeric: tabular-nums`.
- Self-host the fonts (no Google Fonts CDN — privacy + offline + CSP, see §8.5).

### 7.4 Space, shape, elevation, motion
- 4px spacing base (4/8/12/16/24/32/48). Generous in P1 surfaces, compact in
  tables.
- Radius: 6px controls, 10px cards, 999 pills. Borders over shadows; shadows are
  faint and only for overlays (drawers, palette, menus).
- Motion: 120–180ms ease-out for enter, 90ms for hover; respect
  `prefers-reduced-motion`. Charts animate once on load, never on every hover.
  No bounce, no parallax — motion communicates causality, not delight.

### 7.5 Voice & content
- Precise, quantified, calm. "62% of changed lines are AI-attributed (last 30d)."
- Risk language is neutral and actionable: "42% of AI lines unreviewed →" not
  "⚠ DANGER."
- Never frame individuals competitively. Numbers describe code provenance, not
  people's output.
- Empty states teach (show the exact CLI command). Errors are human + recoverable.

### 7.6 Component inventory (bespoke; build, don't import a kit)
AppShell (rail + topbar), CommandPalette, TimeRangePicker, KpiTile (+delta),
Chart (stacked-area, bar, sparkline — one small lib, themed), DataTable
(virtualized, sortable, faceted, keyboard), FacetFilterBar, RepoCard, RiskBadge,
ProvenanceBadge/Legend, CodeViewer-with-gutter, AttributionPanel, SessionTimeline
(from existing page), PolicyComplianceTable, JobStatus (poller), AuditRow/Drawer,
MemberRow + RoleProvenancePopover, StatusPill, Skeletons, EmptyState, Toast,
Modal/Drawer, VerifyChainBadge. Each component ships with its four states and is
documented in a lightweight in-repo gallery (see §8.8).

---

## 8. Runtime & architecture

### 8.1 Where it runs
Served **same-origin by the hub** at `/app/*` (static assets), so the browser
session cookie set by OIDC SSO is first-party (no CORS, no token-in-URL — the
exact issue flagged in the dashboard-coupling review). The SPA calls the existing
`/v1/...` JSON API with `credentials: include`. A `GET /app/*` fallback serves
`index.html` for client-side routing; unknown `/v1` paths still 404 as JSON.

### 8.2 Packaging: one binary
Embed the built assets into `tellur-server` via `rust-embed` (or `include_dir`)
behind a Cargo feature (`dashboard`, default on), so self-host stays "one binary,
zero extra infra" — consistent with the project's packaging principle. A dev mode
proxies to the Vite dev server for fast iteration. **Licensing (decided,
§12.3): the dashboard is part of `tellur-server` and ships under the same
license as the server — FSL-1.1-ALv2.** The UI source lives under the server
crate (e.g. `crates/server/ui/`), carries FSL headers/`LICENSE`, and the README
states this explicitly; it is *not* the Apache-2.0 `web/` session-replay page,
which stays as the daemon's local viewer.

### 8.3 Stack (recommendation + trade-offs)
Recommend **Vite + TypeScript + Svelte 5** (or SolidJS), a **bespoke component
layer** (no Material/AntD — those read as "AI default"), **uPlot** for charts
(tiny, fast, themable), **TanStack-style virtualization** for big tables, and a
typed API client **generated from the hub's OpenAPI** (which we should publish as
part of Phase 0 so client and server can't drift).

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| Keep vanilla (today) | zero build, one file | won't scale to this scope; hard to keep "top-notch" + tested | ✗ |
| React + a UI kit | hiring familiarity | bundle size; generic look; kit fights bespoke design | ✗ for look |
| **Svelte/Solid + bespoke** | tiny bundle, fast, full design control, great DX | smaller talent pool than React | **✓** |

Hard rule: **no analytics SDKs, no web fonts CDN, no runtime third-party calls.**
Everything ships with the binary.

### 8.4 State & data
- URL is the source of truth for view state (filters/range/selection) →
  shareable, back-button-correct.
- A thin typed fetch layer with request dedupe + small TTL cache + ETag revalidation;
  background refetch on focus. Optimistic UI only for low-risk admin actions
  (role grants), with rollback.
- Job-polling abstraction with backoff for Exports/compliance.

### 8.5 Security
- CSP: `default-src 'self'`; no inline scripts (hashed/nonce build), no remote
  origins; this is straightforward because everything is same-origin and
  self-hosted. Documented in the threat model when shipped.
- Auth purely via the existing session cookie (HttpOnly/Secure/SameSite=Lax) or
  Bearer for embedded/API use; the SPA holds no long-lived secret. 401 → redirect
  to `/auth/login?return=<path>`; 403 → clean explainer.
- Escape/encode all rendered data (the current `web/` already learned this);
  prefer framework auto-escaping; never `innerHTML` untrusted content.
- New read endpoints are tenant-scoped and role-gated server-side — the UI's
  hiding of admin nav is convenience, not the control.

### 8.6 Performance budgets (product requirements)
- Initial JS ≤ ~150KB gzipped (route-split; charts/table lazy-loaded).
- Overview interactive < 1.5s on a mid laptop over LAN; any view's first
  meaningful paint < 1s with skeletons.
- 60fps scroll on 100k-row audit/table via virtualization.
- Time-series endpoints must return pre-bucketed data (no client-side crunching of
  raw events).

### 8.7 Accessibility, responsive, i18n, theming
- WCAG 2.2 AA: focus-visible everywhere, full keyboard paths, ARIA for
  table/drawer/menu/palette, reduced-motion, contrast-audited themes, `prefers-color-scheme`.
- Responsive: 3-pane desktop → collapsing rail on tablet → stacked, read-mostly
  on phone (governance read, not admin write).
- i18n-ready: all copy via a message catalog; default `en`; locale-aware dates/
  numbers from day one even if only `en` ships.

### 8.8 Quality & CI
- Component gallery (Storybook-equivalent) as living docs + visual states.
- Tests: unit (logic/format), component (interaction), a few Playwright E2E flows
  against a seeded hub (login→overview→repo→file→verify; export job poll;
  audit filter; role change). Run headless in CI.
- Lint/format (eslint/prettier or Biome), typecheck, `cargo` build of the
  embedded-assets feature. Add a `web` CI job (mirrors how JetBrains/Docker are
  separate jobs). Lighthouse/axe budget checks gate the PR.

---

## 9. Wireframe — app shell (responsive intent)

```
Desktop ≥1200                         Tablet 768–1199          Phone <768
┌──┬───────────────────────┐         ┌─┬───────────────────┐  ┌───────────┐
│  │ topbar: org · range ⌘K│         │≡│ topbar            │  │ topbar  ≡ │
│ n├───────────────────────┤         │ ├───────────────────┤  ├───────────┤
│ a│  content (cards/table)│         │ │ content           │  │ content   │
│ v│  drill → drawer/panel │         │ │ (rail collapses)  │  │ (stacked, │
│  │                       │         │ │                   │  │  read)    │
└──┴───────────────────────┘         └─┴───────────────────┘  └───────────┘
```

---

## 10. Telemetry (privacy-respecting, opt-in)
To measure §2.3 without betraying the trust thesis: **self-hosted, aggregate,
opt-in** product analytics only — counts/timings stored in the hub's own DB
(e.g. a `ui_metric` table), never sent to a third party, never per-keystroke,
never raw content. Admins can disable and can see exactly what is recorded. This
is itself a selling point for P3.

---

## 11. Delivery plan (phased; each phase is shippable + reviewable)

Aligned with the project's one-feature-per-PR norm. "API-first": the read
endpoints land before the screens that need them.

- **D0 — Foundation & contract.** Publish the hub OpenAPI; scaffold the SPA
  (Vite/Svelte/TS), AppShell, routing, auth redirect, design tokens + base
  components + gallery; serve `/app/*` from the hub (embedded assets feature) with
  same-origin auth. Ship a real **Overview** on the *existing* `/dashboard`
  payload only. *Outcome:* you can log in via SSO and see a styled, trustworthy
  landing screen. (1–2 PRs.)
- **D1 — Time & repos.** Endpoint A1 (activity time-series) + A3 (repo summary) +
  A5 (event facets); Overview trends, Repositories list, Repo detail (Activity
  tab). (API PR, then UI PR.)
- **D2 — Evidence.** A4 (attribution read) + A6 (sessions index); File view with
  the **metadata-first provenance gutter** (mode 1, "open in Git" links — no
  source proxying; A12/full-source is a later opt-in) + AttributionPanel +
  Verify-chain; Sessions list + replay (elevate existing timeline). The "Git
  shows what, Tellur shows how" moment.
- **D3 — Governance.** A7 (audit read) + Audit screen; Exports screen (admin)
  honoring both export shapes — job-polled event/audit and synchronous
  per-repo SLSA/SPDX (§4.5). (P3 value.)
- **D4 — Policy & access.** A8 (policy compliance snapshots, job-backed) +
  Policies screen; A2 (members) + A11 (session-auth groups) + A10 (SSO status) +
  People & Access. (P3/P4 value.)
- **D5 — Polish.** Command palette breadth, density/theme/i18n finalization, a11y
  audit, performance pass, E2E coverage, docs.

Each D-phase updates README/AGENTS/PROJECT_STATUS/THREAT_MODEL as it lands
(repo norm). UI work is gated behind its API PR being merged.

## 12. Decisions (resolved 2026-06-08)

1. **Reviewed / review-gap definition — DECIDED.** An AI-attributed range counts
   as reviewed only with: an explicit human review state; a human `reviewer`
   distinct from the producing agent/actor; a `reviewed_at` after the latest AI
   modification of the range; and, where policy requires tests, passing test
   evidence. Review coverage = `reviewed_ai_lines / total_ai_attributed_lines`;
   review gap is the inverse. Computed server-side (see §6). Drives A1/A3/A9.
2. **Frontend stack — DECIDED.** Svelte 5 / Solid + bespoke components (no UI
   kit), per the §8.3 recommendation.
3. **Dashboard licensing — DECIDED.** Ships as part of `tellur-server` under the
   server's license (**FSL-1.1-ALv2**); explicit license headers + README
   wording (see §8.2).
4. **Policy compliance compute — DECIDED.** Durable-job-backed, cached as
   timestamped snapshots per `(org, repo, policy version)`; the UI reads the
   latest snapshot, admins can trigger re-evaluation (see §4.4, A8).
5. **Multi-org routing — DECIDED.** Org-scoped routes from day one
   (`/app/orgs/:org/...`); single-org installs redirect to the default org and may
   hide the switcher, but deep links stay future-proof (see §3.2).
6. **Telemetry — DECIDED.** Opt-in, self-hosted aggregate metric table; no
   third-party analytics (see §10).
7. **Phone scope — DECIDED.** Read-only: governance/investigation views must be
   readable from Slack/PR links on a phone; admin **writes** require
   tablet/desktop (see §8.7).

### Remaining (smaller) open items
- Whether to compose `/overview` (A9) or keep extending `/dashboard` — perf call,
  decide at D0/D1.
- Whether/when to add the opt-in full-source gutter (A12) and job-backed
  SLSA/SPDX (A13) — both are post-D2/D3 niceties, not blockers.

## 13. Appendix — traceability summary
Personas P1–P4 → JTBD (§1.2) → use cases U1–U9 (§1.3) → screens (§4) → API gaps
A1–A13 (§6) → delivery D0–D5 (§11). Every screen exists for a named job; every
job has a persona; every aggregate links to verifiable evidence.
