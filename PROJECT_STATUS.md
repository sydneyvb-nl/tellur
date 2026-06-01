# Tellur — Project Status & Agent Guide

**Last updated:** 2026-06-01 (global agent/editor integrations)
**Maintained by:** agents — alle agents mogen dit updaten
**Repo:** github.com/sydneyvb-nl/tellur
**Branch:** main
**License:** Apache-2.0

> **2026-06-01 — Global agent setup.** Added one-time user-level setup for
> Codex, Claude Code, Cursor, and VS Code: `tellur setup agents` installs global
> hooks/settings with an absolute `tellur` executable path, generates a local
> Codex personal plugin/marketplace entry for manual workflows, writes Cursor
> MCP/settings and VS Code settings, and routes hook/editor payloads through
> `tellur hooks ingest --auto-init` so new Git repositories can start capturing
> without per-project plugin invocation. Hook ingest now ignores invalid JSON,
> refuses malformed setup config instead of overwriting it, and never falls back
> to whole-tree capture when a tool hook lacks a file path.
>
> **2026-06-01 — Adapter hardening.** Tightened adapter imports after review:
> imported events now preserve source IDs, timestamps, session IDs, actors, and
> event types while Tellur recomputes the local hash chain; malformed non-empty
> JSON/JSONL lines now fail instead of being silently dropped; Codex/Copilot
> prompt-like fields are hashed and retained metadata is sanitized; Claude Code
> hook capture is scoped to the hook file path when available; `tellur import
> aider <source>` now uses `<source>` as the Git repository path.
>
> **2026-05-31 — Git notes interop.** Added Git AI-compatible authorship notes
> support under `refs/notes/ai`: `tellur notes export/show/import/fetch/push`
> plus `install-config` for notes fetch/rewrite setup. Notes are a compact
> commit-attestation layer; Tellur's richer event log, redaction state, replay
> data, and policy evidence remain in local/private storage.
>
> **2026-05-31 — Codex + Copilot adapters.** Added `tellur import codex`
> for Codex CLI JSONL event streams/session transcripts and
> `tellur import copilot` for GitHub Copilot metadata exports. Both adapters
> normalize prompts, command events, file writes, and raw metadata into
> Tellur events and are covered by adapter unit tests plus CLI integration
> tests.
>
> **2026-05-31 — Dashboard live data.** The local daemon now backs the session
> replay dashboard with real indexed data: `GET /sessions` returns session rows
> with attribution stats, and `GET /sessions/{id}/events` returns timeline events
> for the selected session. The static dashboard client now understands canonical
> Tellur event wire types (`file.write`, `command.post_execute`, etc.) and
> escapes live event body text before rendering.
>
> **2026-05-31 — Code review & remediation.** A full review found that many
> modules previously marked ✅ were stubs or not wired together (watch, MCP,
> daemon, Claude Code hooks, gc, redact, Homebrew, npm) and that the core
> attribution pipeline was never connected end-to-end. All findings are
> documented in [`docs/FINDINGS.md`](docs/FINDINGS.md) and have been fixed:
> the capture → attribution → index → explain/blame/pr-report pipeline now
> works end-to-end, the daemon is loopback-only + token-authenticated, the MCP
> server speaks real stdio JSON-RPC, hooks use Claude Code's real schema, and
> two security issues (CI command injection, unauthenticated daemon) are
> resolved. That remediation pass was verified with `cargo build`,
> `cargo clippy` (0 warnings), and `cargo test`.

---

## Wat is Tellur

AI code provenance platform. Git vertelt je *wat* er veranderde. Tellur vertelt je *hoe AI participeerde*.

Open-source, lokaal-first, geen cloud dependency. Rust core + CLI, TypeScript editor extension.

---

## PRD Referentie

De PRD bevindt zich op een locatie die Sydney bepaalt. Als je de PRD niet hebt, vraag Sydney.

## Hoe te werken aan deze repo

### Regels voor alle agents

1. **Altijd `PROJECT_STATUS.md` updaten** na elke wijziging — dit is het single source of truth
2. **Build moet groen zijn** voor je commit: `cargo build && cargo test`
3. **Rust code** in `crates/` — TypeScript alleen in `editor/`
4. **Commits** in het Engels, conventional commits format (`feat:`, `fix:`, `docs:`)
5. **Push altijd** na commit — `git push origin main`
6. **Als je een module afmaakt**, update dan de checklist hieronder met ✅
7. **Als je iets tegenhoudt**, zet het in de Blockers sectie
8. **Geen breaking changes** aan bestaande schemas zonder Sydney's goedkeuring

### Build & Test

```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test
cd editor/tellur-vscode
npm run compile
npm run test:unit
npm run test:extension
```

### Structuur

```
Tellur/
├── PROJECT_STATUS.md        ← DIT BESTAND
├── Cargo.toml               ← Rust workspace root
├── crates/
│   ├── core/                ← Core library (schemas, attribution, storage, policy, redaction, export)
│   ├── cli/                 ← CLI binary (tellur command)
│   └── adapters/            ← AI tool adapters
├── schemas/                 ← JSON Schema definities
├── .github/workflows/       ← GitHub Actions
└── editor/                  ← VS Code/Cursor-compatible extension
```

---

## PRD Implementatie Checklist

### Phase 1: Foundation (PRD secties 1-7)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 1 | Project scaffold | 32.1 | ✅ Done | Rust workspace, 3 crates |
| 2 | Core schemas (Session, Event, Attribution, Policy, ProvenanceBundle, PRReport) | 4-6 | ✅ Done | `crates/core/src/schema/types.rs` |
| 3 | ID generation (hash_event, hash_content, generate_session_id, etc.) | 4.2 | ✅ Done | `crates/core/src/schema/ids.rs` |
| 4 | JSON Schema definities | 4-6 | ✅ Done | `schemas/*.json` |
| 5 | EventWriter (JSONL + SHA-256 hash chain) | 7 | ✅ Done | `crates/core/src/storage/event_log.rs` |
| 6 | TraceIndex (SQLite) | 7.3 | ✅ Done | `crates/core/src/storage/index.rs` |
| 7 | RepoStorage (.tellur directory) | 7.1 | ✅ Done | `crates/core/src/storage/repo.rs` |
| 8 | File change capture (git diff + blob SHA) | 8.3 | ✅ Done | `crates/core/src/storage/file_watcher.rs` |

### Phase 2: Core Intelligence (PRD secties 8-14)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 9 | AttributionEngine (line-level patch attribution) | 9 | ✅ Done | `crates/core/src/attribution/engine.rs` |
| 10 | RedactionEngine (regex secret detection) | 14 | ✅ Done | `crates/core/src/redaction/mod.rs` |
| 11 | PolicyEngine (YAML rules, sensitive paths) | 13 | ✅ Done | `crates/core/src/policy/mod.rs` |
| 12 | AgentAdapter trait (async_trait) | 8.3 | ✅ Done | `crates/core/src/adapter/mod.rs` |
| 13 | Built-in adapters (Claude Code, Aider, Cursor, Generic, Codex, Copilot) | 8.3 | ✅ Done | `crates/core/src/adapter/builtin.rs` + `crates/adapters/src/*` |
| 14 | Claude Code adapter implementation | 8.1 | ✅ Done | Real Claude Code hook schema (PostToolUse/SessionStart), `tellur hooks install`, stdin payload handler `tellur hooks claude` wired to capture pipeline and scoped to hook file path when available, transcript parse |
| 15 | Aider adapter implementation | 8.2 | ✅ Done | Git log parser, Aider pattern detection, source repo path honored by CLI import |
| 16 | Cursor adapter implementation | 8.2 | ✅ Done | Cursor MCP/settings setup, VS Code-compatible extension capture, JSON/JSONL trace parsing, workspace detection, adapter tests |
| 16a | Codex CLI adapter implementation | 8.2 | ✅ Done | JSONL event stream/session transcript import via `tellur import codex <file>`, command/prompt/file-write normalization, prompt hashing, strict JSONL errors |
| 16b | GitHub Copilot adapter implementation | 8.2 | ✅ Done | Metadata JSON/JSONL import via `tellur import copilot <file>`, accepted suggestion + prompt metadata normalization, prompt hashing, no raw metadata payload |
| 16c | Global agent/editor setup | 8.1/8.3/10/23 | ✅ Done | `tellur setup agents/status/uninstall/cursor/vscode`, user-level Codex/Claude hooks, Codex personal plugin scaffold, Cursor MCP/settings, VS Code settings, extension save capture, generic hook ingest with auto-init |

### Phase 3: CLI (PRD sectie 8.1)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 17 | `tellur init` | 8.1 | ✅ Done | CLI main.rs |
| 18 | `tellur doctor` | 8.1 | ✅ Done | Detecteert AI tools |
| 19 | `tellur status` | 8.1 | ✅ Done | Sessions overview |
| 20 | `tellur explain <file:line>` | 8.1 | ✅ Done | Line attribution lookup |
| 21 | `tellur blame <file>` | 8.1 | ✅ Done | File-wide attribution |
| 22 | `tellur pr-report` | 12 | ✅ Done | Risk report + markdown |
| 23 | `tellur policy check` | 13 | ✅ Done | Policy evaluation |
| 24 | `tellur watch` | 8.1 | ✅ Done | Real `notify` filesystem watcher with debounce → git-diff capture → attribution → index (incl. untracked/new files) |
| 25 | `tellur event` | 8.1 | ✅ Done | Single event emission |
| 26 | `tellur export` | 15 | ✅ Done | Provenance bundle export |
| 27 | `tellur import` | 8.1 | ✅ Done | JSONL import |
| 28 | `tellur verify` | 11 | ✅ Done | Hash chain verification |
| 29 | `tellur sessions` | 8.1 | ✅ Done | Session listing |
| 30 | `tellur gc` | 8.1 | ✅ Done | Real retention-based deletion (keep_days from config), rewrites logs + rebuilds index; `--dry-run` is truly dry |
| 31 | `tellur redact` | 14 | ✅ Done | Rewrites stored payloads in place, records RedactionInfo, re-seals hash chain so `verify` stays intact |

### Phase 4: Reports & Export (PRD secties 12, 15, 20)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 32 | PRReportGenerator | 12 | ✅ Done | `crates/core/src/report/pr_report.rs` |
| 33 | Markdown report output | 12.3 | ✅ Done | `PRReportGenerator::to_markdown()` |
| 34 | Provenance export (6 profiles) | 15, 20 | ✅ Done | `crates/core/src/storage/export.rs` |
| 35 | GitHub Action (PR check) | 12.4 | ✅ Done | `.github/workflows/tellur.yml` |

### Phase 5: Editor Extension (PRD sectie 10)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 36 | VS Code/Cursor-compatible extension | 10 | ✅ Done | Full extension: client, decorations, tree views, commands, auto-init, auto-watch, save capture through `hooks ingest` |
| 37 | Inline attribution decorations | 10.1 | ✅ Done | Purple (AI) vs green (human) line decorations | |
| 38 | Hover cards (origin, model, confidence) | 10.2 | ✅ Done | Explain command shows origin, model, confidence, session | |
| 39 | Sidebar panel | 10.3 | ✅ Done | Sessions + Attributions tree views in activity bar | |
| 40 | Session explorer | 10.4 | ✅ Done | SessionProvider tree with agent, model, event count | |

### Phase 6: Advanced Features (PRD secties 16-25)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 41 | Session replay web UI | 16 | ✅ Done | Dark theme timeline, session sidebar, attribution bar, diff viewer, demo fallback, live daemon data via `/sessions` + `/sessions/{id}/events` | Web dashboard |
| 42 | Git remapping | 17 | ✅ Done | SHA remap via git diff-tree, rebase detection, 3 tests | |
| 43 | SLSA/SPDX export | 20 | ✅ Done | SLSA v1.0 provenance + SPDX 2.3 SBOM with AI metadata, 2 tests | |
| 44 | HTTP daemon (axum) | 22 | ✅ Done | `tellur daemon` (loopback-only, token-auth, Host check). Server **recomputes the hash chain** via EventWriter — clients cannot forge provenance. 6 endpoints. |
| 45 | MCP server | 23 | ✅ Done | `tellur mcp` — real stdio JSON-RPC 2.0 (initialize/tools/list/tools/call). 6 tools backed by actual index/policy/verify queries. |
| 46 | Team/server mode | 24 | ❌ Not started | |
| 47 | Plugin SDK | 25 | ❌ Not started | |

### Phase 7: Distribution (PRD sectie 32.3)

| # | Module | PRD Sectie | Status | Details |
|---|--------|-----------|--------|---------|
| 48 | Cross-compilation | 32.3 | ✅ Done | mac arm64/x64 + linux x64 (musl), 3.7-4.4MB binaries | |
| 49 | Homebrew formula | 32.3 | ✅ Done | `dist/tellur.rb` (build-from-source; `sha256` placeholder to fill at release tag) |
| 50 | npm package (CLI wrapper) | 32.3 | ✅ Done | JS API wrapper + CLI runner + post-install downloader that **verifies SHA-256** against the release `.sha256` sidecar before installing |
| 51 | GitHub Release automation | 32.3 | ✅ Done | 5-target matrix build on tag push | |

---

## Wat is NIET uit de PRD opgepakt

Deze onderdelen staan in de PRD maar zijn bewust overgeslagen of vereisen Sydney's beslissing:

1. **Pricing / Business model** (PRD sectie 27-31) — niet relevant voor dev, Sydney beslist
2. **Team/server mode** (PRD sectie 24) — later, eerst local-first afmaken
3. **SOC 2 compliance** (PRD sectie 26) — far future
4. **Plugin SDK** (PRD sectie 25) — API stabiliteit eerst nodig
5. **Release signing** (PRD sectie 20) — na v1.0 (SLSA/SPDX *export* is wel klaar)
6. ~~**Session replay web dashboard met live data**~~ — ✅ Done via local daemon endpoints
7. ~~**GitHub Copilot / Codex CLI adapters**~~ — ✅ Done as import adapters

---

## Huidige Test Status

```
101 Rust tests, 0 failures, 0 clippy warnings.
- core:      65 tests (schema/event-type round-trip, glob matcher, storage,
             hash-chain verify + reseal, index session/attribution round-trip,
             capture pipeline end-to-end, block_ai_read, attribution, redaction,
             policy, export, PR report, dashboard daemon endpoints)
- adapters:  16 tests (Claude Code, Aider, Cursor, Codex, Copilot, Generic)
- cli:       20 integration tests (version/help/init/doctor/status/sessions/verify/import/setup/hooks ingest)
- editor:    TypeScript compile, 5 unit tests, VS Code extension integration tests
```

Run: `cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test`, then `cd editor/tellur-vscode && npm run compile && npm run test:unit && npm run test:extension`.

---

## Blockers

| Blocker | Impact | Oplossing |
|---------|--------|-----------|
| Geen | — | — |

---

## Tech Stack Beslissingen

| Keuze | PRD Sectie | Reden |
|-------|-----------|-------|
| Rust (core + CLI) | 32.1 | Performance, safety, single binary |
| SQLite (index) | 7.3 | Local, zero-config, fast queries |
| JSONL (event log) | 7.2 | Append-only, human-readable, git-friendly |
| SHA-256 (hash chain) | 7.2 | Tamper evidence, lightweight |
| TypeScript (editor) | 10 | VS Code API vereist TS |
| YAML (policy) | 13 | Git-friendly config |

---

## Git Log (laatste 5 commits)

```
3073a75 feat: release workflow, homebrew formula, zero warnings
19f466e feat: cross-compilation + CLI integration tests
1db9723 feat: Rust rewrite — core engine, CLI, attribution, policy, redaction
8cb1d1e feat: initial project scaffold — core schemas, CLI foundation, monorepo setup
2a20ab8 Initial commit
```

---

## Volgende Stappen (prioriteit)

1. ~~**Claude Code hook installer**~~ — ✅ Done
2. ~~**Aider commit parser**~~ — ✅ Done  
3. ~~**VS Code/Cursor-compatible extension**~~ — ✅ Done
4. ~~**Codex CLI adapter**~~ — ✅ Done
5. ~~**GitHub Copilot adapter**~~ — ✅ Done
6. **Next first-party adapters for adoption** — Gemini CLI / Google Antigravity, Windsurf/Cascade, JetBrains AI Assistant / Junie, Devin, Continue, Cline/Roo Code
7. **Team/server mode** — decide architecture after local dashboard settles
8. **Plugin SDK** — requires stable adapter/event API

---

## Concurrentieonderzoek (2026-05-31)

Directe concurrenten die hetzelfde probleem oplossen:

### 1. Git AI (usegitai.com)
- **Open-source** git extension
- Gebruikt **Git Notes** voor AI authorship tracking
- `git-ai blame` + `git-ai stats` commands
- Ondersteunt Agent Hooks voor automatische tracking
- `refs/notes/ai` open standard
- **+/−**: Git-native (goed), maar geen policy engine, geen redaction, geen export profiles

### 2. AgentBlame (mesa.dev/agentblame)
- **Line-level AI attribution** via git notes
- Combineert git blame met AI attributie
- Tool/model breakdown dashboards
- Ondersteunt Cursor, Claude Code, OpenCode
- **+/−**: Mooie UX, maar gesloten platform (SaaS?), geen policy/redaction

### 3. Entire CLI
- Captureert AI session transcripts in git commits
- Line-level AI vs human attributie
- **+/−**: Focus op session capture, minder breed dan Tellur

### 4. AI Footprint
- Git-native AI code tracking
- **+/−**: Vroeg stadium, vergelijkbare aanpak

### Tellur differentiators
1. **Policy engine** — YAML-based rules voor sensitive paths, required reviews, test evidence
2. **Secret redaction** — regex-based detection van API keys, tokens, passwords
3. **6 export profiles** — developer, OSS, corporate, audit, release, CI
4. **PR risk reports** — risico scoring, AI involvement stats, reviewer checklist
5. **Tamper-evident log** — SHA-256 hash chain (geen concurrent heeft dit)
6. **Multi-adapter** — Claude Code, Aider, Cursor, Generic uit de doos
7. **Rust** — single binary, snel, geen runtime dependency
8. **SLSA/SPDX ready** — export naar supply chain formats

### Actiepunten uit concurrentie
- Git Notes integratie overwegen (git-ai gebruikt dit als open standaard)
- Dashboard metrics (tool/model breakdown) toevoegen aan `tellur stats`
- `/ask` feature (chat met AI die code schreef) — uniek, overwegen voor later

---

*Agents: update dit bestand na elke werk sessie. Voeg je naam + timestamp toe aan de "Last updated" regel.*
