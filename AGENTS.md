# Tellur Agent Instructions

These instructions apply to the whole repository.

## Start Here (orientation for every run)

Tellur is an AI code provenance platform: Git tells you *what* changed, Tellur
tells you *how AI participated*. Local-first, no cloud dependency. A Rust core +
CLI (`crates/`), a VS Code extension and a JetBrains plugin (`editor/`), a static
dashboard (`web/`), and packaging (`dist/`).

Read these in order before acting; they are the source of truth:

1. **`PROJECT_STATUS.md`** — what is built, what is open, test counts, roadmap,
   blockers, and a dated changelog at the top. The single source of truth for
   status.
2. **This file's [Architecture Map](#architecture-map)** — where every layer
   lives, so you land changes in the right place.
3. **`README.md`** — user-facing behavior: commands, setup, adapters, limits.
4. **`docs/ADAPTERS.md`** — adapter mechanics, integration mechanisms (hooks vs
   extension vs MCP vs daemon webhook vs import), guarantees, and known limits.
5. **`CONTRIBUTING.md`** — dev workflow and repo structure.
6. **`docs/FINDINGS.md`** — historical review/remediation notes.

One-glance repository layout:

```
tellur/
├── crates/
│   ├── core/         # library: schema, storage, attribution, policy, redaction,
│   │                 #          export, daemon, mcp, notes, remap, report
│   ├── cli/          # the `tellur` binary (all commands + global setup)
│   └── adapters/     # per-tool import parsers + hook/payload normalization
├── editor/
│   ├── tellur-vscode/    # VS Code / Cursor / Windsurf extension (TypeScript)
│   └── tellur-jetbrains/ # JetBrains IDE plugin (Kotlin/Gradle)
├── schemas/          # JSON Schema for session/event/attribution/pr-report/provenance
├── web/              # static session-replay dashboard (served by the daemon)
├── dist/             # npm wrapper + Homebrew formula
└── .github/workflows # CI + release automation
```

## Documentation Must Track Behavior

When changing any behavior, workflow, adapter, integration mechanism, setup
command, config shape, storage format, policy behavior, public CLI surface, or
editor/plugin/MCP/daemon implementation detail, update the relevant
documentation in the same change.

At minimum, check these files before finishing:

- `README.md` for user-facing behavior, commands, setup, and limits.
- `docs/ADAPTERS.md` for adapter mechanics, integration mechanisms, guarantees,
  and known limits.
- `PROJECT_STATUS.md` for implementation status, open work, test counts, and
  roadmap state.
- `CONTRIBUTING.md` when development workflow, verification, or repo structure
  changes.

If a code change intentionally does not require documentation updates, state why
in the final response or commit/PR summary.

Do not leave stale documentation that contradicts the implementation. In
particular, update docs when changing:

- lifecycle hooks or hook payload handling;
- editor integration behavior for VS Code, Cursor, Codex, Claude Code, or
  future agents;
- setup/uninstall/status commands and files they write;
- adapter import formats, redaction, prompt hashing, or provenance guarantees;
- daemon, MCP, Git notes, policy, export, or storage behavior;
- test counts or verification commands in `PROJECT_STATUS.md`.

## Architecture Map

Use this map before changing code so you land changes in the right layer.

| Area | Path | What lives there |
| --- | --- | --- |
| Core data model | `crates/core/src/schema/` | `types.rs` (Session, Event, Actor, AgentInfo, EventType + wire strings), `ids.rs` (hashing + prefixed ID generation). |
| JSON Schemas | `schemas/*.json` | Canonical session/event/attribution/pr-report/provenance schemas. Keep in sync with `schema/types.rs`. |
| Append-only provenance log | `crates/core/src/storage/event_log.rs` | `EventWriter`: JSONL event writing, imported-event preservation, SHA-256 hash-chain (re)sealing + verification. |
| SQLite query index | `crates/core/src/storage/index.rs` | `TraceIndex`: session/event/attribution indexing used by CLI, MCP, daemon, editor, dashboard. |
| Repo storage layout | `crates/core/src/storage/repo.rs` | `.tellur/` discovery/init (traces, index, policies, config, daemon token, `disable`). |
| Git/file capture | `crates/core/src/capture.rs`, `crates/core/src/storage/file_watcher.rs` | Working-tree diff capture, filtered path capture, attribution writes. |
| Attribution engine | `crates/core/src/attribution/` | Line-level patch → AI/human attribution that powers `explain`/`blame`/`pr-report`. |
| Policy / redaction | `crates/core/src/policy/`, `crates/core/src/redaction/` | YAML policy rules + sensitive paths; regex secret detection/redaction. |
| Export | `crates/core/src/storage/export.rs`, `crates/core/src/export/` | Provenance bundles + SLSA v1.0 / SPDX 2.3 export profiles. |
| Reports | `crates/core/src/report/` | PR risk report generation + markdown rendering. |
| Git notes | `crates/core/src/notes.rs` | `refs/notes/ai` Git AI-compatible authorship notes (`tellur notes …`). |
| Git remapping | `crates/core/src/remap/` | SHA remap across rebase/amend via `git diff-tree`. |
| Glob matching | `crates/core/src/glob.rs` | Path glob matcher shared by policy/capture filters. |
| Local daemon | `crates/core/src/daemon/` | `mod.rs`: loopback-only, token-auth HTTP API (events, sessions, export, dashboard). `webhook.rs`: normalizes cloud-agent (Devin) native payloads on `POST /webhook/{source}` and recomputes the hash chain. |
| MCP server | `crates/core/src/mcp/` | Stdio JSON-RPC tools exposed to agents/editors. |
| Adapter implementations | `crates/adapters/src/` | Per-tool import parsers (`<tool>.rs`) + hook payload normalization; `import.rs` is the shared tolerant JSONL/array/envelope loop; `sanitize.rs` redaction/hashing helpers. Keep prompt hashing/redaction here. |
| Adapter registry | `crates/core/src/adapter/builtin.rs`, `crates/adapters/src/lib.rs` | Built-in adapter metadata (`supports_hooks`) and exports. |
| CLI commands/setup | `crates/cli/src/main.rs` | All `tellur` commands, global setup/uninstall/status, `hooks ingest` (the universal hook/webhook entrypoint + source normalization), import dispatch. |
| CLI integration tests | `crates/cli/tests/cli_integration.rs` | End-to-end binary behavior and generated config fixtures. |
| VS Code/Cursor/Windsurf extension | `editor/tellur-vscode/src/` | Extension client, commands, tree providers, save/watch capture, model diagnostics. Same extension serves VS Code, Cursor, and Windsurf (all VS Code-compatible). |
| JetBrains plugin | `editor/tellur-jetbrains/` | IntelliJ Platform plugin (Kotlin/Gradle): `VFS_CHANGES` listener → `hooks ingest --source jetbrains`, settings UI. Built with Gradle, **not** the Rust CI (see Verification). |
| Web dashboard | `web/index.html` | Static dashboard client backed by daemon endpoints. |
| Packaging | `dist/`, `.github/workflows/` | npm wrapper, Homebrew formula, release and CI workflows. |
| User docs | `README.md`, `docs/ADAPTERS.md`, `docs/FINDINGS.md` | Public commands/mechanisms/limits; adapter mechanics; historical review notes. |
| Project source of truth | `PROJECT_STATUS.md` | Implementation status, open work, test counts, roadmap, blockers, changelog. |

Cross-layer rules:

- New adapter: add `crates/adapters/src/<adapter>.rs`, export it from
  `crates/adapters/src/lib.rs`, register metadata in
  `crates/core/src/adapter/builtin.rs`, add CLI import/setup dispatch in
  `crates/cli/src/main.rs`, add tests, and update `README.md`,
  `docs/ADAPTERS.md`, and `PROJECT_STATUS.md`.
- New global setup surface: add install/status/uninstall paths together; use an
  absolute `tellur` executable path; refuse to overwrite malformed JSON; add an
  integration test for generated config.
- New hook source: route through `tellur hooks ingest --source <source>
  --auto-init` unless the surface has a stronger documented API; add the source
  to `normalize_hook_source` in `crates/cli/src/main.rs`; never capture the whole
  working tree from a tool hook without a concrete file path.
- New webhook/cloud-agent source: add the native-payload mapping to
  `crates/core/src/daemon/webhook.rs` (core cannot depend on the adapters crate);
  it is reached via the existing `POST /webhook/{source}` route. Keep prompt
  hashing + command redaction and let `EventWriter` recompute the hash chain.
- New editor/runtime behavior: update the relevant editor integration
  (`editor/tellur-vscode` and/or `editor/tellur-jetbrains`) and the setup docs,
  because users should configure it once globally.
- Pick the integration mechanism honestly per tool (lifecycle hook > editor
  extension/plugin > MCP tool access > daemon webhook > import). Document which
  one a tool uses in `docs/ADAPTERS.md` and do not model a tool as having
  Codex-style hooks when it does not.

## Verification

For Rust changes, run:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

For VS Code extension changes, also run from `editor/tellur-vscode`:

```bash
npm run compile
npm run test:unit
```

Run `npm run test:extension` when the change touches activation, commands,
settings, or VS Code runtime behavior.

### JetBrains plugin (`editor/tellur-jetbrains`)

This plugin is Kotlin built with **Gradle + the IntelliJ Platform SDK**, which
the Rust `cargo` toolchain and the Rust CI do **not** build. Building it requires
JDK 17 and downloads the IntelliJ SDK over the network, so it is verified
separately, not by `cargo test`:

```bash
cd editor/tellur-jetbrains
gradle wrapper          # one-time: generate ./gradlew
./gradlew buildPlugin    # compile + package the plugin zip
./gradlew runIde         # launch a sandbox IDE to test capture manually
```

"Builds outside Rust CI" means exactly this: changes to the plugin are not
covered by `cargo test`/clippy and must be compiled/run with Gradle to verify
them. The plugin itself is normal, working code — only the *verification path*
differs from the Rust workspace. If you cannot run Gradle in your environment,
say so explicitly instead of claiming the plugin was compiled.
