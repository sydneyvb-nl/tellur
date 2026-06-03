# Tellur Agent Instructions

These instructions apply to the whole repository.

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
| Core data model | `crates/core/src/schema/` | Sessions, events, attribution, reports, IDs, wire event types. |
| Append-only provenance log | `crates/core/src/storage/event_log.rs` | JSONL event writing, imported event preservation, hash-chain resealing and verification. |
| SQLite query index | `crates/core/src/storage/index.rs` | Session/event/attribution indexing used by CLI, MCP, daemon, editor, dashboard. |
| Git/file capture | `crates/core/src/capture.rs`, `crates/core/src/storage/file_watcher.rs` | Working-tree diff capture, filtered path capture, attribution writes. |
| Policy/redaction/export | `crates/core/src/policy/`, `crates/core/src/redaction/`, `crates/core/src/storage/export.rs`, `crates/core/src/export/` | Policy checks, secret cleanup, provenance/SLSA/SPDX export. |
| Local daemon | `crates/core/src/daemon/` | Loopback HTTP API for ingestion and dashboard data. `webhook.rs` normalizes cloud-agent (Devin) native payloads on `POST /webhook/{source}`. |
| MCP server | `crates/core/src/mcp/` | Stdio JSON-RPC tools exposed to agents/editors. |
| Adapter implementations | `crates/adapters/src/` | Tool-specific import parsers and hook payload normalization. Keep prompt hashing/redaction here. |
| Adapter registry | `crates/core/src/adapter/builtin.rs`, `crates/adapters/src/lib.rs` | Built-in adapter metadata and exports. |
| CLI commands/setup | `crates/cli/src/main.rs` | User commands, global setup/uninstall/status, hook ingestion, import dispatch. |
| CLI integration tests | `crates/cli/tests/cli_integration.rs` | End-to-end binary behavior and generated config fixtures. |
| VS Code/Cursor extension | `editor/tellur-vscode/src/` | Extension client, commands, tree providers, save/watch capture, model diagnostics. |
| JetBrains plugin | `editor/tellur-jetbrains/` | IntelliJ Platform plugin (Kotlin/Gradle): `VFS_CHANGES` listener → `hooks ingest --source jetbrains`. Builds outside Rust CI. |
| Web dashboard | `web/index.html` | Static dashboard client backed by daemon endpoints. |
| Packaging | `dist/`, `.github/workflows/` | npm wrapper, Homebrew formula, release and CI workflows. |
| User docs | `README.md`, `docs/ADAPTERS.md` | Public commands, integration mechanisms, guarantees, limits, adapter roadmap. |
| Project source of truth | `PROJECT_STATUS.md` | Implementation status, open work, test counts, roadmap, blockers. |

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
  --auto-init` unless the surface has a stronger documented API; never capture
  the whole working tree from a tool hook without a concrete file path.
- New editor/runtime behavior: update both `editor/tellur-vscode` docs/settings
  and the setup docs, because users should configure it once globally.

## Verification

For Rust changes, run:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

For editor changes, also run from `editor/tellur-vscode`:

```bash
npm run compile
npm run test:unit
```

Run `npm run test:extension` when the change touches activation, commands,
settings, or VS Code runtime behavior.
