# Tellur — Code Review & Findings

**Date:** 2026-05-31
**Reviewer:** Claude (Opus 4.8) — full code review + security review
**Scope:** entire repository at `main` (Rust core/CLI/adapters, VS Code extension, schemas, CI, packaging, docs)
**Build status at review time:** `cargo build` green; `cargo test` 61 unit tests.

This document records every finding. Items are tracked through to resolution in the
"Status" column. `PROJECT_STATUS.md` previously marked most of these as ✅ Done; this
document is the corrected source of truth for what actually worked.

---

## A. Headline

The foundation is solid (clean workspace, good schema modelling, real SHA-256 hash chain,
parameterised SQLite, 61 unit tests). **But the end-to-end pipeline was not wired together.**
The core promise — line-level AI-vs-human attribution — did not work in practice because no
CLI path ever wrote attribution data. Several modules marked "Done" were stubs or dead code.

---

## B. Security findings

| ID | Severity | Location | Issue | Status |
|----|----------|----------|-------|--------|
| SEC-1 | High | `.github/workflows/tellur.yml` | Command injection via `github.head_ref`/`base_ref` interpolated into a `run:` shell step. Branch names allow shell metacharacters → arbitrary code execution on the CI runner from a fork PR. | FIXED |
| SEC-2 | Medium | `crates/core/src/daemon/mod.rs` | HTTP daemon had no authentication, no Origin/Host check (CSRF/DNS-rebind), and wrote client-supplied `prev_hash`/`event_hash` to disk without recomputing — provenance forgery, defeats the tamper-evident claim. | FIXED |
| SEC-3 | Low | `dist/npm/install.js` | (Future) binary downloader had no checksum/signature verification. | FIXED (sha256 verification added) |

---

## C. P0 — Core functionality missing / stubs (claims did not match reality)

| ID | Location | Issue | Status |
|----|----------|-------|--------|
| P0-1 | `crates/cli/src/main.rs` | Attribution was **never written**: `AttributionEngine::attribute_patch` + `TraceIndex::index_attribution` were only called in tests. `explain`/`blame`/`pr-report` therefore always returned empty. | FIXED |
| P0-2 | `cmd_watch` | `watch` watched nothing — wrote one `session.start` event and printed "coming soon". | FIXED (real `notify`-based watcher + git-diff capture + attribution) |
| P0-3 | `crates/core/src/mcp/mod.rs` | MCP server returned placeholder strings; no transport; no `tellur mcp` command. | FIXED (real stdio JSON-RPC server wired to core queries) |
| P0-4 | `crates/core/src/daemon/mod.rs` | Daemon was dead code (no `tellur daemon` command). | FIXED (command added; hash chain enforced server-side) |
| P0-5 | `crates/adapters/src/claude_code.rs` | Hook installer used a fabricated settings schema, was never invoked, and `$TELLUR_SESSION` was never set. | FIXED (real Claude Code hook format + `tellur hooks` command) |
| P0-6 | `cmd_redact` | `redact` only scanned and printed; never rewrote stored events. | FIXED (rewrites JSONL + reindexes) |
| P0-7 | `cmd_gc` | `gc` only printed counts; retention config ignored; `--dry-run` did nothing different. | FIXED (real retention-based deletion) |
| P0-8 | `dist/`, `PROJECT_STATUS.md` | `dist/tellur.rb` (Homebrew) did not exist; npm `install.js` downloaded nothing. | FIXED (formula added; downloader implemented) |
| P0-9 | `crates/core/src/storage/export.rs` | "corporate/audit/release" export profiles were identical to developer (no redaction/signing). | FIXED (profiles now differ meaningfully) |

---

## D. P1 — Correctness bugs

| ID | Location | Issue | Status |
|----|----------|-------|--------|
| P1-1 | `storage/event_log.rs:70` | Unknown event types silently became `file.write`; `EventType::Custom` could not round-trip (serialised as object, not string). | FIXED (manual ser/de as wire string; unknown → `Custom`) |
| P1-2 | `storage/index.rs:130` | `index_event` auto-created sessions with `agent_id="agent"`, `repo_id="local"`; no `index_session`; `model_name` never populated. | FIXED (`index_session` added; richer schema) |
| P1-3 | `attribution/engine.rs:48` | Every diff range hardcoded to `origin: Ai, confidence: 1.0`; no human-vs-AI logic; `human_lines` always 0. | FIXED (origin determined by capture source; human edits tracked) |
| P1-4 | `policy/mod.rs:179` | `glob_match` broke on `**/.env*` and trailing-`*` suffixes (the key secret path never matched); `.contains()` fallback too permissive. | FIXED (proper glob matcher) |
| P1-5 | `redaction/mod.rs:131` | `is_sensitive_path` `**/*.pem` matched literal `*.pem`. | FIXED (shared glob matcher) |
| P1-6 | `schema/types.rs:415` | `block_ai_read` (used by default policy + README) was not a struct field → silently ignored. | FIXED (field added + enforced) |
| P1-7 | `storage/file_watcher.rs:101` | `compute_file_hash` claimed git blob SHA but used SHA-256 while `get_blob_sha` used git SHA-1 → before/after never comparable. | FIXED (uses `git hash-object`) |
| P1-8 | daemon vs event_log | Daemon used `chrono::Local` for file partitioning; `EventWriter` used `Utc` → split-day chain ordering bugs. | FIXED (UTC everywhere, via EventWriter) |
| P1-9 | `storage/event_log.rs` | No concurrency control on the hash chain; parallel writers fork/break it. | FIXED (lock file guard) |

---

## E. P2 — Robustness & smaller issues

| ID | Location | Issue | Status |
|----|----------|-------|--------|
| P2-1 | `cli/main.rs:210` | `cmd_doctor` used `panic!` on missing dirs. | FIXED |
| P2-2 | `export/mod.rs:195`, `aider.rs:108` | `commit_sha[..8]` panics on short SHAs. | FIXED |
| P2-3 | `.github/workflows/tellur.yml` | CI downloaded a raw `arm64` binary on an x64 runner from a `.tar.gz` release asset → never worked. | FIXED |
| P2-4 | `.github/workflows/release.yml` | Windows asset named `…​.exe.zip`; npm map expected `…​.zip`. | FIXED |
| P2-5 | `.gitignore` | `Cargo.lock` and `dist/` ignored yet committed (contradictory). For a binary, `Cargo.lock` should be committed. | FIXED |
| P2-6 | `export/mod.rs` | `bundle_hash` computed over a bundle containing the empty `bundle_hash` field, with no verifier resetting it. | FIXED (documented + verify helper) |
| P2-7 | `aider.rs:15` | `(?i)aider` matched anywhere → false positives; regexes recompiled per call. | FIXED (lazy compiled, tighter patterns) |
| P2-8 | `schemas/` | README referenced `tellur.pr-report.v1` / `tellur.provenance.v1` schemas that did not exist. | FIXED (schemas added) |
| P2-9 | `CONTRIBUTING.md` | Described a TypeScript/npm project (`packages/`, `npm install`) — wrong stack entirely. | FIXED (Rust) |
| P2-10 | editor extension | `client.ts` called `explain/blame/sessions --json` flags that did not exist, with JSON shapes the CLI never emitted → extension non-functional. | FIXED (`--json` added to CLI; shapes aligned) |
| P2-11 | adapters | `EventType::Custom` from Cursor/Claude adapters collapsed to `file.write` on import. | FIXED (via P1-1) |
| P2-12 | `generic.rs` | `GenericAdapter::import_jsonl` existed but `cmd_import` never used the `generic` adapter. | FIXED (wired) |

---

## F. What was already good (kept)

- Clean Cargo workspace with shared `workspace.dependencies`.
- Expressive schema/type modelling in `schema/types.rs`.
- Real SHA-256 hash chain with cross-file continuity and corruption recovery.
- Parameterised SQLite queries (no injection).
- UUID v7 (time-ordered) IDs.
- 61 unit tests; modules individually well covered.

---

## G. Verification

After fixes: `cargo build` green, `cargo test` green, plus new tests covering the wired
pipeline (capture → attribution → index → explain/blame/pr-report), redaction rewrite, gc
retention, glob matching, and event-type round-tripping. See `PROJECT_STATUS.md` for the
honest module status.
</content>
</invoke>
