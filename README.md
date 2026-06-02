<p align="center">
  <a href="https://tellur.dev">
    <img src="docs/assets/tellur-readme-hero.jpg" alt="Tellur local-first AI code provenance" width="100%" />
  </a>
</p>

# Tellur

**Local-first AI code provenance for teams shipping AI-assisted software.**

[![Website](https://img.shields.io/badge/website-tellur.dev-69d3a5)](https://tellur.dev)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-f46623.svg)](Cargo.toml)
[![Adapters](https://img.shields.io/badge/adapters-Codex%20%7C%20Claude%20%7C%20Cursor%20%7C%20Copilot-111827)](docs/ADAPTERS.md)

Git tells you **what** changed. Tellur tells you **how AI participated**.

Tellur records AI-assisted development evidence in your repository: which agent
changed which lines, what model and prompt hash were involved, whether tests
ran, and whether sensitive changes need human review. It is open source,
local-first, and built for developers, maintainers, security reviewers, and
teams that need an audit trail for AI-generated code.

**Website:** [tellur.dev](https://tellur.dev)

## Why Developers Star Tellur

- **AI code attribution:** explain whether a line was human-written,
  AI-generated, or mixed.
- **Local-first provenance:** store evidence in `.tellur/`; source code does not
  need to leave your machine.
- **Tamper-evident audit trail:** append-only JSONL events are sealed with a
  SHA-256 hash chain.
- **PR risk reports:** surface AI involvement, sensitive paths, test evidence,
  and review gaps before merge.
- **Multi-agent adapters:** capture or import evidence from Codex, Claude Code,
  Cursor, VS Code/Copilot, Gemini CLI, Antigravity, Aider, and generic tools.
- **Policy-as-code:** define rules for auth, payments, secrets, infra, blocked
  AI reads, required tests, and required human review.

If Tellur helps you make AI-assisted code review safer, star the repo so other
developers can find it.

## The Problem

AI coding agents are now part of everyday software development, but most teams
still review AI-assisted changes with standard Git metadata only. That leaves
important questions unanswered:

- Which lines were AI-generated, human-written, or mixed?
- Which model, tool, prompt hash, and session produced a change?
- Did the agent touch security-sensitive code such as auth, payments, secrets,
  or infrastructure?
- Were tests run before the change was merged?
- Does a PR need extra review because of AI involvement?

Tellur turns those questions into local, queryable evidence for code review,
compliance, supply-chain provenance, and engineering governance.

## What Tellur Does

| Capability | What you get |
| --- | --- |
| Line-level attribution | Code ranges mapped to agent, model, session, prompt hash, evidence strength, and confidence score. |
| Session capture | AI-assisted activity from CLI commands, editor hooks, importers, MCP, and the local daemon. |
| PR risk reports | AI involvement, sensitive paths, test evidence, review gaps, and policy warnings. |
| Tamper-evident logs | Append-only JSONL with a SHA-256 hash chain for local verification. |
| Fast local queries | SQLite index powering CLI, VS Code/Cursor extension, MCP tools, daemon, and dashboard views. |
| Portable exports | Provenance bundles for developer, OSS, corporate, audit, release, and CI workflows. |
| Secret redaction | Cleanup for common keys, tokens, passwords, and private key material. |

## Status

Tellur is in beta. The local pipeline is functional end to end:

```text
capture -> attribution -> event log -> SQLite index -> CLI/editor/reports
```

Implemented surfaces include the CLI, global Codex/Claude Code/Gemini
CLI/Antigravity hooks, Cursor MCP/settings, VS Code/Cursor extension capture,
importers for Cursor, Aider, Codex CLI, Gemini CLI, Antigravity, and GitHub
Copilot, a local token-authenticated daemon, an MCP stdio server, provenance
export, Git notes interop, and a static session replay dashboard backed by
daemon data.

Team/server mode is not implemented yet.

## Install

From source:

```bash
cargo install --path crates/cli
```

For development:

```bash
cargo build
cargo test
cargo run -p tellur-cli -- --help
```

Tellur currently targets Rust stable with edition 2024.

## Quickstart

Initialize Tellur in a Git repository:

```bash
tellur init
```

Check the local setup and detected AI tools:

```bash
tellur doctor
```

Start capturing file changes:

```bash
tellur watch
```

Install one-time global agent/editor integrations:

```bash
tellur setup agents
```

Import activity from supported tools:

```bash
tellur import cursor path/to/agent-trace.json
tellur import aider path/to/repo
tellur import codex path/to/codex-events.jsonl
tellur import copilot path/to/copilot-metadata.jsonl
```

Query attribution:

```bash
tellur explain src/auth/session.ts:84
tellur blame src/auth/session.ts
```

Generate a PR report:

```bash
tellur pr-report --base main --head HEAD
```

Verify the event log:

```bash
tellur verify
```

## CLI Reference

```bash
tellur init                         # Initialize .tellur/
tellur doctor                       # Check setup and detect tools
tellur status                       # Show repository capture status
tellur watch                        # Capture working tree changes
tellur explain <file:line>          # Explain attribution for one line
tellur blame <file>                 # Show file attribution ranges
tellur sessions                     # List captured sessions
tellur pr-report --base main        # Generate a PR risk report
tellur policy check                 # Evaluate configured policies
tellur event --event-type file.write --session <id> --file <path>
tellur import <adapter> <source>    # Import external AI tool data
tellur export --format json         # Export provenance data
tellur notes export                 # Write Git AI-compatible refs/notes/ai
tellur notes import                 # Import refs/notes/ai into the local index
tellur notes push                   # Push refs/notes/ai to origin
tellur notes fetch                  # Fetch refs/notes/ai from origin
tellur daemon                       # Run local HTTP ingestion/dashboard API
tellur mcp                          # Run MCP server over stdio
tellur setup agents                 # Install one-time global agent/editor integrations
tellur setup cursor                 # Install Cursor MCP/settings integration
tellur setup vscode                 # Install VS Code extension settings
tellur setup gemini-cli             # Install Gemini CLI hooks
tellur setup antigravity            # Install Antigravity hooks/MCP integration
tellur setup status                 # Check global agent integration status
tellur gc --dry-run                 # Garbage-collect expired events
tellur redact                       # Redact secrets from stored events
tellur verify                       # Verify hash-chain integrity
```

`explain`, `blame`, and `sessions` support `--json` for machine-readable output.

## Supported Adapters And Integrations

| Tool | Mechanism | Status |
| --- | --- | --- |
| Claude Code | User/project lifecycle hooks + transcript import | Working |
| Codex CLI/App | User lifecycle hooks, local personal plugin, JSONL import | Working |
| Gemini CLI | User lifecycle hooks, JSONL import | Working |
| Google Antigravity 2.0 | User hooks, MCP config, JSONL import | Working |
| Cursor IDE/CLI | Cursor MCP/settings, VS Code-compatible extension save/watch capture, JSON/JSONL import | Working |
| VS Code/Copilot | VS Code extension auto-init, watch, save capture, explicit prompt hashing, metadata import | Working with VS Code API limits |
| Aider | Git commit attribution import | Working |
| GitHub Copilot | Metadata JSON/JSONL import | Working |
| Generic | CLI events, JSONL, local HTTP daemon | Working |

Import adapters preserve source event IDs, source timestamps, session IDs, actor,
event type, and payload while recomputing Tellur's local hash chain. Invalid
non-empty JSON/JSONL lines fail the import instead of being silently skipped.
Prompt-like fields are stored as hashes, not raw prompt text; secret-looking
strings in retained metadata are redacted.

`tellur import aider <source>` expects `<source>` to be a Git repository path.
Other import adapters expect a file path unless the adapter-specific docs say
otherwise.

The adapter layer is pluggable, so additional tools can normalize their events
into Tellur's schema without changing the core attribution engine.
See [`docs/ADAPTERS.md`](docs/ADAPTERS.md) for current adapter guarantees,
known limits, and the adoption roadmap.

## One-Time Agent Setup

For Codex, Claude Code, Gemini CLI, Antigravity, Cursor, and VS Code, Tellur
supports user-level installation so users do not need to invoke a skill or
plugin in every project:

```bash
tellur setup agents
```

This installs global hooks for Claude Code (`~/.claude/settings.json`) and Codex
(`~/.codex/hooks.json`). It publishes a local Codex personal plugin under
`~/.codex/plugins/tellur-provenance` with a marketplace entry in
`~/.agents/plugins/marketplace.json` for manual workflows such as status,
verification, and PR reporting. It writes Gemini CLI hooks to
`~/.gemini/settings.json`, Antigravity hooks to `~/.gemini/config/hooks.json`,
Antigravity MCP config to `~/.gemini/antigravity/mcp_config.json` and
`~/.gemini/antigravity-cli/mcp_config.json`, Cursor MCP/settings
(`~/.cursor/mcp.json` plus Cursor user settings), and VS Code user settings so
the Tellur extension can auto-init, watch, and capture saved files in every Git
workspace.

Global hooks call the absolute path of the installed `tellur` executable:

```bash
/absolute/path/to/tellur hooks ingest --source <agent> --auto-init
```

When a hook runs outside a Git repository it no-ops. When it runs inside a Git
repository without `.tellur/`, `--auto-init` creates the local Tellur storage
with safe defaults. To disable capture for a repository, create
`.tellur/disable`. Invalid hook payloads no-op, and tool hooks only capture
working-tree changes when the hook payload includes a concrete file path.

Use `tellur setup status` to inspect installed global integrations and
`tellur setup uninstall` to remove Tellur-installed global hooks and the local
Codex plugin, Cursor MCP/settings, and VS Code settings.

Cursor and VS Code do not have the same documented local lifecycle hook model as
Codex. Tellur therefore uses the durable editor surfaces they do expose:
extension save/watch capture, Cursor MCP tools, explicit prompt hashing, Git
policy checks, and import adapters.

### Integration Mechanisms

| Surface | Setup command | Files written | Runtime behavior |
| --- | --- | --- | --- |
| Claude Code | `tellur setup claude-code` or `tellur setup agents` | `~/.claude/settings.json` | Lifecycle hooks call `tellur hooks ingest --source claude-code --auto-init`; project hooks remain available via `tellur hooks install claude-code`. |
| Codex CLI/App | `tellur setup codex` or `tellur setup agents` | `~/.codex/hooks.json`, `~/.codex/plugins/tellur-provenance`, `~/.agents/plugins/marketplace.json` | User hooks call `tellur hooks ingest --source codex --auto-init`; local plugin exposes manual Tellur workflows through Codex's plugin directory. |
| Gemini CLI | `tellur setup gemini-cli` or `tellur setup agents` | `~/.gemini/settings.json` | Gemini `BeforeTool`/`AfterTool`/agent/session hooks call `tellur hooks ingest --source gemini-cli --auto-init --json-response`. |
| Antigravity 2.0 | `tellur setup antigravity` or `tellur setup agents` | `~/.gemini/config/hooks.json`, `~/.gemini/antigravity/mcp_config.json`, `~/.gemini/antigravity-cli/mcp_config.json` | Antigravity lifecycle hooks call `tellur hooks ingest --source antigravity --auto-init --json-response`; MCP exposes Tellur tools to Antigravity agents. |
| Cursor | `tellur setup cursor` or `tellur setup agents` | `~/.cursor/mcp.json`, Cursor user `settings.json` | Cursor can call Tellur MCP tools; the installed Tellur extension uses auto-init, watch, and save capture with source `cursor`. |
| VS Code | `tellur setup vscode` or `tellur setup agents` | VS Code user `settings.json` | The installed extension auto-inits Git workspaces, starts `tellur watch`, and captures saved files through safe hook ingestion with source `vscode`. |

All setup commands write absolute `tellur` executable paths. Existing malformed
JSON settings are not overwritten; setup fails so the user can repair or back up
the file.

## Data Model

Tellur stores repository-local data under `.tellur/`:

```text
.tellur/
├── config.yml           # Configuration, intended to be committed
├── policies/
│   └── default.yml      # Policy rules, intended to be committed
├── traces/
│   └── sessions/        # JSONL event logs, gitignored by default
├── index/
│   └── tellur.db        # SQLite index, gitignored by default
└── exports/             # Generated provenance bundles
```

Versioned JSON schemas live in [`schemas/`](./schemas/):

| Schema | Description |
| --- | --- |
| `tellur.session.v1` | A bounded AI-assisted development interaction |
| `tellur.event.v1` | A timestamped action within a session |
| `tellur.attribution.v1` | Line-level origin mapping for a file |
| `tellur.pr-report.v1` | PR risk report with AI involvement stats |
| `tellur.provenance.v1` | Portable export bundle |

## Policy Example

```yaml
# .tellur/policies/default.yml
version: 1

sensitive_paths:
  - path: "src/auth/**"
    tags: ["auth", "security-sensitive"]
    require_human_review: true
    require_tests: true

  - path: "**/.env*"
    tags: ["secrets"]
    block_ai_read: true

rules:
  - id: require-tests-for-ai-code
    description: "AI code changes > 20 lines require test evidence"
    when:
      attribution.origin: ai
      changed_lines.greater_than: 20
    action: warn
    require:
      tests_run: true
```

## Architecture

```text
Tellur/
├── crates/
│   ├── core/          # Schemas, attribution, storage, policy, export, daemon, MCP
│   ├── cli/           # tellur command
│   └── adapters/      # Claude Code, Cursor, Aider, Codex, Copilot, Generic
├── editor/            # VS Code extension
├── schemas/           # JSON Schema definitions
├── dist/              # npm wrapper and Homebrew formula
└── web/               # Session replay dashboard
```

Core storage is intentionally simple: append-only JSONL for auditability,
SQLite for query speed, and Git blob SHAs for stable attribution across file
states.

## Git Notes Interop

Tellur can publish compact authorship attestations as Git notes using the Git
AI-compatible `refs/notes/ai` namespace:

```bash
tellur notes export --print   # inspect the note payload
tellur notes export           # attach it to HEAD
tellur notes push             # publish refs/notes/ai
tellur notes fetch            # fetch refs/notes/ai
tellur notes import           # import a note back into Tellur's local index
```

Git notes are treated as an interoperability and transport layer, not as
Tellur's primary database. Prompts, transcripts, redaction state, replay data,
and policy evidence remain in Tellur's local/private storage; notes contain only
line ranges, lightweight session metadata, and commit-scoped attribution.

## Development

```bash
cargo fmt --check
cargo test
cargo run -p tellur-cli -- doctor
```

VS Code extension:

```bash
cd editor/tellur-vscode
npm install
npm run compile
npm test
npm run package
```

Tellur's VS Code extension supports VS Code/Copilot Bring Your Own Key model
metadata for watch sessions. Configure BYOK in VS Code with
`Chat: Manage Language Models`, then run `Tellur: Select VS Code AI Model` if
more than one model is available. `Tellur: Diagnose VS Code AI Models` shows
which models VS Code exposes to extensions and what Tellur will attach to new
watch sessions.

VS Code does not expose a public API that lets one extension intercept arbitrary
Copilot/BYOK chat prompts from other chat participants. For prompt provenance,
use `Tellur: Record AI Prompt`; Tellur records a SHA-256 prompt hash plus model
metadata, not the raw prompt text.

## Roadmap

- Team/server mode for shared organizational visibility
- More first-party adapters for emerging AI coding tools, prioritized as:
  Windsurf/Cascade, JetBrains AI Assistant / Junie, Devin, Continue, and Cline/Roo Code
- Richer policy templates for security-sensitive repositories
- Packaged releases for npm, Homebrew, and GitHub Releases

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](./LICENSE).
