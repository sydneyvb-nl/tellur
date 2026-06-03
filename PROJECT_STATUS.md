# Tellur — Project Status & Agent Guide

**Last updated:** 2026-06-03 (Tier 1 B1 — identity & tenancy; on feature branch)
**Maintained by:** agents — alle agents mogen dit updaten
**Repo:** github.com/sydneyvb-nl/tellur
**Branch:** main
**License:** Apache-2.0 (core) · FSL-1.1-ALv2 (`crates/server`)

> **2026-06-03 — Tier 1 B1 (identity & tenancy).** On branch
> `feat/server-b1-identity-tenancy`. Added to `crates/server`: an `auth` module
> (viewer/contributor/admin roles; API tokens `tlr_<id>_<secret>` with the secret
> stored only as an **Argon2id** hash; split id/secret for constant-work lookup),
> storage for orgs/members/tokens + a **tamper-evident hash-chained audit log**
> (append/verify/tamper-detect), a deny-by-default axum auth extractor
> (`Principal` from `Authorization: Bearer`), tenant-scoped endpoints
> `GET /v1/me` and `GET /v1/orgs/{org}/me` (object+tenant authz → **BOLA**
> blocked), and a `tellur-server admin create-org/create-token` bootstrap CLI.
> Verified: 25 server tests incl. cross-org BOLA regression; live end-to-end
> smoke (401/200/403); fmt/clippy/test/deny green. Next: B2 (ingest & verify).
>
> **2026-06-03 — B1 review fixes (Codex).** Addressed 4 P2 findings on PR #2:
> audit appends now run in a `BEGIN IMMEDIATE` transaction (atomic across
> connections); the audit chain persists a head-hash/length checkpoint so tail
> truncation is detected by `verify_audit_chain`; Argon2 verification runs off
> the async worker via `spawn_blocking` and releases the DB lock first; and
> presented-but-invalid bearer tokens are now audited (`auth_denied`) while
> header-less requests are not (avoids anonymous audit flooding).
>
> **2026-06-03 — CI hardening.** Fixed a clippy failure that only surfaced on
> the Ubuntu CI (an unnecessary `match` inside a Linux-only `#[cfg]` block that
> macOS-local clippy never compiles). Durable follow-ups: pinned the toolchain in
> `rust-toolchain.toml` (1.96.0) so local/CI clippy match; dropped the redundant
> `cargo-audit` CI step (cargo-deny already scans RustSec); bumped
> `actions/checkout` v4→v5 (Node 20 EOL). CI is green.
>
> **2026-06-03 — Tier 1 B0 shipped (server scaffolding).** New FSL-licensed
> `crates/server` (`tellur-server` binary), the secure foundation before any data
> endpoints. Secure-by-default config (refuses non-loopback bind without explicit
> opt-in; validated at boot), typed errors → RFC 9457 `problem+json` that never
> leak internals on 5xx, a swappable `Store` trait with a SQLite backend
> (WAL/foreign-keys, idempotent migrate), `AppState` + axum router with
> `/healthz` + `/readyz`, structured `tracing`, graceful shutdown. Added
> `SECURITY.md` (coordinated disclosure + CRA reporting), `docs/THREAT_MODEL.md`
> (STRIDE), `deny.toml` + a CI workflow (`cargo fmt`/`clippy`/`test` +
> `cargo-deny` + `cargo-audit`). Verified: 10 server tests; `cargo deny check` →
> advisories/bans/licenses/sources ok; live binary smoke-tested.
>
> **2026-06-03 — Tier 1 implementation plan.** Researched current security &
> compliance standards (OWASP ASVS 5.0, OWASP API Top 10 2023/BOLA, EU CRA,
> SLSA v1.0, NIST SSDF, GDPR, SOC 2/ISO 27001 readiness, Sigstore/SBOM) and
> wrote a secure-by-design, maintainable, scalable plan for the `tellur serve`
> hub: `docs/proposals/TEAM_SERVER_IMPLEMENTATION.md` (new FSL `crates/server`,
> thin-handler/fat-service layering, swappable SQLite→Postgres `Store` trait,
> data-layer tenant scoping to kill BOLA, hash-chained audit log, phased B0–B6
> with CI security gates). Plan only — no code yet.
>
> **2026-06-03 — Team mode Tier 0 shipped.** First build of roadmap item #8.
> `tellur team report` aggregates the `refs/notes/ai` authorship notes of every
> commit in a `--base..--head` range into one team view: AI vs human lines, by
> tool / model / author, plus provenance coverage (which commits carry a note).
> Pure Git-native, no server — notes travel over the existing remote. Pure
> aggregation core in `crates/core/src/report/team_report.rs` (+3 unit tests),
> CLI `tellur team report [--base --head --notes-ref --json]` (+1 integration
> test). Tolerant: missing/unparseable notes count as "without provenance"
> rather than failing. Phase A is complete: an example PR workflow lives at
> `docs/examples/github-actions-team-report.yml` (fetch notes → aggregate →
> upsert one PR comment).
>
> **2026-06-03 — Licensing direction.** Documented the license/structure
> direction in `docs/proposals/LICENSING.md`: Apache-2.0 for the core
> (CLI/core/adapters/schemas/editor); the future team/server component
> (`crates/server`) under FSL-1.1-Apache-2.0 in the same monorepo; contributions
> via DCO, no CLA. Direction only — not implemented; not legal advice.
>
> **2026-06-03 — Team/server mode proposal.** Researched roadmap item #8 against
> the existing local-first primitives (Git notes, per-repo hash chain, daemon,
> export profiles, policy) and the target segments (independent/OSS, SMB,
> corporate). Wrote a phased design proposal at
> `docs/proposals/TEAM_SERVER_MODE.md`: keep local-first as default, use Git
> notes as the zero-infra team transport, and add an optional self-hostable
> `tellur serve` hub. Decided MVP path: **Tier 0 (`tellur team report`, no
> server) first, then Tier 1 (self-hosted hub)**. Proposal only — no code yet;
> reconciled with the PRD (§6 surface 11, §16.2 Layer 5, §32 Step 20; note PRD
> §24 is *Architecture Guardian*, not team mode).
>
> **2026-06-03 — Devin webhook + JetBrains plugin.** Continued roadmap item #7
> ("live capture beyond import") for the remaining adoption tools.
> **Devin** now has a first-class daemon webhook: `POST /webhook/{source}`
> (token-auth, loopback-only) in `crates/core/src/daemon/` normalizes a tool's
> native run/session payload (messages, shell commands, file edits, status) into
> canonical Tellur events, hashing prompt-like fields, redacting commands, and
> **recomputing the hash chain** so provenance cannot be forged. The normalizer
> lives in `crates/core/src/daemon/webhook.rs` (core can't depend on the adapters
> crate). **JetBrains** now has a real IntelliJ Platform plugin under
> `editor/tellur-jetbrains/` (Kotlin/Gradle): it subscribes to `VFS_CHANGES` and
> routes saved/created files to `tellur hooks ingest --source jetbrains
> --auto-init`, capturing AI Assistant and Junie edits live; capture is
> best-effort and off the EDT. The plugin builds outside the Rust workspace CI
> (JDK 17 + IntelliJ SDK via Gradle); the Gradle wrapper is committed and
> `./gradlew buildPlugin` is verified green on JDK 17 (loadable plugin zip).
> Added 4 Rust tests (webhook normalization + authenticated route).
>
> **2026-06-02 — Windsurf live capture.** Started roadmap item #7 ("live capture
> beyond import") for the adoption tools. Windsurf/Cascade now has live capture,
> not just import: `tellur setup windsurf` (and `tellur setup agents`) writes
> Windsurf user settings plus `~/.codeium/windsurf/mcp_config.json`, so the
> VS Code-compatible Tellur extension captures saves with source `windsurf` and
> Windsurf agents can call Tellur MCP tools — mirroring the Cursor integration.
> The same extension capture also records edits made by Continue and Cline/Roo
> Code when they run inside any VS Code-family editor (VS Code, Cursor, Windsurf).
> JetBrains stays import-only (MCP is configured in-IDE, no stable global config);
> Devin stays import-only with live capture available by posting events to the
> authenticated daemon (`POST /events`). Added `windsurf`/`cascade`,
> `jetbrains`/`junie`, `devin`, `continue`, and `cline`/`roo` source
> normalization for `hooks ingest`. Cursor/Windsurf MCP writing is now a shared
> helper. Covered by a new `setup windsurf` CLI test plus extended `setup agents`
> assertions.
>
> **2026-06-01 — Global agent setup.** Added one-time user-level setup for
> Codex, Claude Code, Gemini CLI, Antigravity, Cursor, and VS Code:
> `tellur setup agents` installs global hooks/settings with an absolute `tellur`
> executable path, generates a local Codex personal plugin/marketplace entry for
> manual workflows, writes Gemini CLI hooks, Antigravity hooks/MCP, Cursor
> MCP/settings, and VS Code settings, and routes hook/editor payloads through
> `tellur hooks ingest --auto-init` so new Git repositories can start capturing
> without per-project plugin invocation. Hook ingest now ignores invalid JSON,
> refuses malformed setup config instead of overwriting it, and never falls back
> to whole-tree capture when a tool hook lacks a file path.
>
> **2026-06-02 — Adoption import adapters.** Added five import adapters from the
> roadmap: Windsurf/Cascade, JetBrains AI Assistant / Junie, Devin, Continue, and
> Cline/Roo Code (the latter covers Roo Code via the shared task format). To keep
> these maintainable, the tolerant JSONL/array/envelope parsing loop, payload
> sanitization, prompt hashing, and field extraction now live in one shared
> module `crates/adapters/src/import.rs`; each new adapter only defines its
> tool-specific event-type mapping. `import.rs` also reads single JSON objects,
> envelope objects (`events`/`messages` arrays), and numeric epoch timestamps
> (Cline). `first_string`/`json_path` moved out of the Gemini adapter into the
> shared module. All five are wired into `tellur import <adapter> <source>`,
> registered as built-in adapters, and covered by unit + CLI integration tests.
> They are import-only because none expose a documented local lifecycle hook.
> Hardened after Codex review: envelope wrappers now propagate session/run
> identity onto child events; field extraction is a bounded recursive scan that
> reaches nested objects and Anthropic Messages `content`-block arrays (so Cline/
> Roo `tool_use` writes, Windsurf transcript `code_action`/`user_input`, and
> Continue `nextEditWithHistory`/`fileURI` are recognized); command events
> recover the command from a generic `text` field; and the prompt-hash key set is
> kept in sync with the redaction key set.
>
> **2026-06-02 — README discoverability refresh.** Updated the public README
> with a generated social/hero image, `https://tellur.dev`, clearer value
> proposition, badges, search-friendly AI code provenance language, and a
> stronger star/discovery prompt. No adapter behavior changed.
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
3. **Rust code** in `crates/` — editor-integraties in `editor/` (VS Code: TypeScript, JetBrains: Kotlin)
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
cargo deny check          # supply-chain gate (licenses + advisories + sources)
cd editor/tellur-vscode
npm run compile
npm run test:unit
npm run test:extension
# JetBrains plugin (Kotlin/Gradle, niet via cargo/CI gebouwd):
cd ../tellur-jetbrains && ./gradlew buildPlugin
```

### Structuur

```
Tellur/
├── PROJECT_STATUS.md        ← DIT BESTAND
├── Cargo.toml               ← Rust workspace root
├── crates/
│   ├── core/                ← Core library (schema, storage, attribution, policy,
│   │                          redaction, export, reports, notes, remap, daemon, mcp)
│   ├── cli/                 ← CLI binary (tellur command)
│   ├── adapters/            ← AI tool adapters (import + hook/payload normalization)
│   └── server/              ← Tier 1 team hub (tellur-server, FSL-1.1-ALv2)
├── schemas/                 ← JSON Schema definities
├── web/                     ← Static session-replay dashboard
├── .github/workflows/       ← GitHub Actions
└── editor/
    ├── tellur-vscode/       ← VS Code / Cursor / Windsurf extension (TypeScript)
    └── tellur-jetbrains/    ← JetBrains IDE plugin (Kotlin/Gradle)
```

> Authoritative architecture map: zie [`AGENTS.md`](AGENTS.md) ("Start Here" + Architecture Map).

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
| 13 | Built-in adapters (Claude Code, Aider, Cursor, Generic, Codex, Copilot, Gemini CLI, Antigravity) | 8.3 | ✅ Done | `crates/core/src/adapter/builtin.rs` + `crates/adapters/src/*` |
| 14 | Claude Code adapter implementation | 8.1 | ✅ Done | Real Claude Code hook schema (PostToolUse/SessionStart), `tellur hooks install`, stdin payload handler `tellur hooks claude` wired to capture pipeline and scoped to hook file path when available, transcript parse |
| 15 | Aider adapter implementation | 8.2 | ✅ Done | Git log parser, Aider pattern detection, source repo path honored by CLI import |
| 16 | Cursor adapter implementation | 8.2 | ✅ Done | Cursor MCP/settings setup, VS Code-compatible extension capture, JSON/JSONL trace parsing, workspace detection, adapter tests |
| 16a | Codex CLI adapter implementation | 8.2 | ✅ Done | JSONL event stream/session transcript import via `tellur import codex <file>`, command/prompt/file-write normalization, prompt hashing, strict JSONL errors |
| 16b | GitHub Copilot adapter implementation | 8.2 | ✅ Done | Metadata JSON/JSONL import via `tellur import copilot <file>`, accepted suggestion + prompt metadata normalization, prompt hashing, no raw metadata payload |
| 16c | Global agent/editor setup | 8.1/8.3/10/23 | ✅ Done | `tellur setup agents/status/uninstall/cursor/vscode/windsurf/gemini-cli/antigravity`, user-level Codex/Claude/Gemini/Antigravity hooks, Codex personal plugin scaffold, Antigravity MCP, Cursor MCP/settings, VS Code settings, Windsurf MCP/settings, extension save capture, generic hook ingest with auto-init |
| 16d | Gemini CLI adapter implementation | 8.2 | ✅ Done | `tellur setup gemini-cli`, `~/.gemini/settings.json` hooks, JSONL import via `tellur import gemini-cli <file>`, prompt hashing and metadata sanitization |
| 16e | Antigravity 2.0 adapter implementation | 8.2/23 | ✅ Done | `tellur setup antigravity`, `~/.gemini/config/hooks.json`, Antigravity app/CLI MCP configs, JSONL import via `tellur import antigravity <file>` |
| 16f | Shared import loop | 8.2 | ✅ Done | `crates/adapters/src/import.rs`: tolerant JSONL/array/envelope/single-object reader, line-specific errors, sanitized+prompt-hashed payloads, nested-field extraction, numeric epoch timestamps. Reused by the adoption adapters below |
| 16g | Windsurf/Cascade adapter | 8.2 | ✅ Done | Import via `tellur import windsurf <file>`; Cascade tool calls, file edits, commands, chat turns |
| 16h | JetBrains AI Assistant / Junie adapter | 8.2 | ✅ Done | Import via `tellur import jetbrains <file>`; AI Assistant + Junie action logs (JSON/array/JSONL) |
| 16i | Devin adapter | 8.2 | ✅ Done | Import via `tellur import devin <file>`; run/session export (object/array/JSONL) for cloud-agent provenance |
| 16j | Continue adapter | 8.2 | ✅ Done | Import via `tellur import continue <file>`; `dev_data` JSONL with nested `data` payloads |
| 16k | Cline / Roo Code adapter | 8.2 | ✅ Done | Import via `tellur import cline <file>`; `ui_messages.json`/`api_conversation_history.json` task history, shared by Roo Code |
| 16l | Windsurf live capture | 8.1/10/23 | ✅ Done | `tellur setup windsurf` writes Windsurf user settings + `~/.codeium/windsurf/mcp_config.json`; VS Code-compatible extension captures saves with source `windsurf` (mirrors Cursor). Same extension also covers Continue/Cline/Roo running in a VS Code-family editor. `hooks ingest` source normalization for windsurf/jetbrains/devin/continue/cline |
| 16m | Devin webhook live capture | 22/23 | ✅ Done | Daemon `POST /webhook/{source}` (token-auth, loopback-only) normalizes native run/session payloads → events with recomputed hash chain (`crates/core/src/daemon/webhook.rs`). 4 tests |
| 16n | JetBrains live-capture plugin | 10 | ✅ Done | `editor/tellur-jetbrains/` IntelliJ Platform plugin (Kotlin/Gradle): `VFS_CHANGES` listener → `hooks ingest --source jetbrains --auto-init`, settings UI for the tellur path. Builds outside Rust CI (JDK 17 + IntelliJ SDK) |

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
| 31a | `tellur team report` | §6.11/§32 Step 20 | ✅ Done | Tier 0 team mode: aggregates `refs/notes/ai` notes over a `--base..--head` range into AI involvement by tool/model/author + provenance coverage. Markdown/`--json`. No server. `crates/core/src/report/team_report.rs` |

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
| 36a | JetBrains IDE plugin | 10 | ✅ Done | `editor/tellur-jetbrains/` — IntelliJ Platform plugin, `VFS_CHANGES` listener → `hooks ingest --source jetbrains`, settings UI. Live capture for AI Assistant/Junie. Builds outside Rust CI |
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
| 44 | HTTP daemon (axum) | 22 | ✅ Done | `tellur daemon` (loopback-only, token-auth, Host check). Server **recomputes the hash chain** via EventWriter — clients cannot forge provenance. 7 endpoints incl. `POST /webhook/{source}` for cloud-agent (Devin) live capture. |
| 45 | MCP server | 23 | ✅ Done | `tellur mcp` — real stdio JSON-RPC 2.0 (initialize/tools/list/tools/call). 6 tools backed by actual index/policy/verify queries. |
| 46 | Team/server mode | §6.11 / §16.2 L5 / §32 Step 20 | 📋 Proposed | Design proposal: [`docs/proposals/TEAM_SERVER_MODE.md`](docs/proposals/TEAM_SERVER_MODE.md). MVP = Tier 0 (Git-native, no server) → Tier 1 (`tellur serve` hub). Not yet implemented |
| 47 | Plugin/adapter SDK | §8.3 / §32 Step 18 | ❌ Not started | Requires stable adapter/event API |

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
2. **Team/server mode** (PRD §6.11 / §16.2 Layer 5 / §32 Step 20) — design
   proposal klaar
   ([`docs/proposals/TEAM_SERVER_MODE.md`](docs/proposals/TEAM_SERVER_MODE.md)).
   **Tier 0 / Phase A klaar:** `tellur team report` + voorbeeld-PR-workflow.
   **Tier 1 (self-host hub) in aanbouw:** B0 (FSL `crates/server` scaffolding)
   klaar; B1 (identity & tenancy) volgt.
3. **SOC 2 compliance** (PRD sectie 26) — far future
4. **Plugin SDK** (PRD sectie 25) — API stabiliteit eerst nodig
5. **Release signing** (PRD sectie 20) — na v1.0 (SLSA/SPDX *export* is wel klaar)
6. ~~**Session replay web dashboard met live data**~~ — ✅ Done via local daemon endpoints
7. ~~**GitHub Copilot / Codex CLI adapters**~~ — ✅ Done as import adapters

---

## Huidige Test Status

```
169 Rust tests, 0 failures, 0 clippy warnings. `cargo deny check` green.
- server:    25 tests (config secure-by-default bind, SQLite store migrate+health,
             error mapping, /healthz+/readyz+404; B1: Argon2id token roundtrip +
             role rules, org/member/token auth, hash-chained audit append/verify/
             tamper-detect + tail-truncation + two-connection chain, authn +
             tenant-scoping/BOLA API tests + auth-denied auditing)
- core:      72 tests (schema/event-type round-trip, glob matcher, storage,
             hash-chain verify + reseal, index session/attribution round-trip,
             capture pipeline end-to-end, block_ai_read, attribution, redaction,
             policy, export, PR report, team report aggregation, dashboard daemon
             endpoints + webhook normalization & authenticated POST /webhook route)
- adapters:  47 tests (Claude Code, Aider, Cursor, Codex, Copilot, Gemini CLI,
             Antigravity, Windsurf, JetBrains, Devin, Continue, Cline/Roo Code,
             Generic, and the shared import loop incl. envelope inheritance,
             content-block extraction, and command-text recovery)
- cli:       25 integration tests (version/help/init/doctor/status/sessions/verify/import/setup incl. windsurf/hooks ingest/team report)
- editor:    VS Code — TypeScript compile, 5 unit tests, extension integration tests.
             JetBrains — `editor/tellur-jetbrains` (Kotlin/Gradle, committed wrapper
             pinned to 8.9) builds outside the Rust CI; `./gradlew buildPlugin`
             verified green on JDK 17 (produces a loadable plugin zip)
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
6. ~~**Next first-party adapters for adoption**~~ — ✅ Done as import adapters (Windsurf/Cascade, JetBrains AI / Junie, Devin, Continue, Cline/Roo Code)
7. **Live capture beyond import** — ✅ Done for all adoption tools' durable
   surfaces. Windsurf/Cascade live via `tellur setup windsurf` (VS Code-compatible
   extension + MCP); Continue and Cline/Roo covered by the same extension inside
   any VS Code-family editor; JetBrains live via the `editor/tellur-jetbrains`
   IntelliJ plugin (`VFS_CHANGES` → `hooks ingest`); Devin live via the daemon
   `POST /webhook/devin` endpoint. Remaining (optional): lifecycle-hook capture
   for Windsurf/JetBrains if/when they document a local hook API; publish the
   JetBrains plugin to the Marketplace.
8. **Team/server mode** — proposal
   [`docs/proposals/TEAM_SERVER_MODE.md`](docs/proposals/TEAM_SERVER_MODE.md);
   reconciled with PRD §6.11 / §16.2 Layer 5 / §32 Step 20.
   **Tier 0 ✅ done** (`tellur team report` + example PR CI).
   **Tier 1 in progress** per
   [`docs/proposals/TEAM_SERVER_IMPLEMENTATION.md`](docs/proposals/TEAM_SERVER_IMPLEMENTATION.md)
   (B0–B6). **B0 ✅** scaffolding (config, errors, Store+SQLite, health, SECURITY/
   STRIDE/cargo-deny/CI). **B1 ✅** (branch `feat/server-b1-identity-tenancy`):
   roles + Argon2id API tokens, orgs/members, hash-chained audit log, deny-by-
   default auth extractor, tenant-scoped `/v1/me` + `/v1/orgs/{org}/me` (BOLA
   blocked), admin bootstrap CLI. Next: **B2 — ingest & verify**.
9. **Plugin SDK** — requires stable adapter/event API

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
6. **Multi-adapter** — Claude Code, Aider, Cursor, Codex, Copilot, Gemini CLI, Antigravity, Generic uit de doos
7. **Rust** — single binary, snel, geen runtime dependency
8. **SLSA/SPDX ready** — export naar supply chain formats

### Actiepunten uit concurrentie
- Git Notes integratie uitbreiden met release/CI workflows waar nuttig
- Dashboard metrics (tool/model breakdown) toevoegen aan `tellur stats`
- `/ask` feature (chat met AI die code schreef) — uniek, overwegen voor later

---

*Agents: update dit bestand na elke werk sessie. Voeg je naam + timestamp toe aan de "Last updated" regel.*
