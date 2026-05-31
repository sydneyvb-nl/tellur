# Tellur

**Local-first AI code provenance for software teams.**

Tellur records how AI participated in a codebase: which agent changed which
lines, what model and prompt context were involved, whether tests ran, and
whether sensitive changes were reviewed.

Git tells you what changed. Tellur tells you how AI participated.

Tellur is open source, runs locally, and stores provenance data inside your
repository. Your source code does not need to leave your machine.

## Why Tellur

AI coding agents are now part of everyday development, but most teams still
review AI-assisted changes with standard Git metadata only. That leaves
important questions unanswered:

- Which lines were AI-generated, human-written, or mixed?
- Which model, tool, prompt, and session produced a change?
- Did the agent touch sensitive files such as auth, payments, secrets, or infra?
- Were tests run before the change was merged?
- Does a PR need extra review because of AI involvement?

Tellur turns those questions into local, queryable evidence for developers,
reviewers, maintainers, and compliance workflows.

## What It Does

- **Line-level attribution** maps code ranges to an agent, model, session,
  prompt hash, evidence strength, and confidence score.
- **Session capture** records AI-assisted activity from CLI commands, editor
  hooks, importers, and the local daemon.
- **PR risk reports** summarize AI involvement, sensitive paths, tests, review
  gaps, and policy warnings.
- **Policy-as-code** lets teams define YAML rules for sensitive paths, required
  tests, human review, and blocked AI reads.
- **Tamper-evident logs** store events as JSONL with a SHA-256 hash chain.
- **Fast local queries** use a SQLite index for CLI, VS Code, MCP, and dashboard
  views.
- **Provenance export** produces portable bundles for developer, OSS,
  corporate, audit, release, and CI workflows.
- **Secret redaction** detects and sanitizes common keys, tokens, passwords, and
  private key material.

## Status

Tellur is in beta. The local pipeline is functional end to end:

```text
capture -> attribution -> event log -> SQLite index -> CLI/editor/reports
```

Implemented surfaces include the CLI, Claude Code hooks, importers for Cursor,
Aider, Codex CLI, and GitHub Copilot, a local token-authenticated daemon, an MCP
stdio server, a VS Code extension, provenance export, and a static session replay
dashboard backed by daemon data.

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

Install Claude Code hooks for automatic capture:

```bash
tellur hooks install
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
tellur daemon                       # Run local HTTP ingestion/dashboard API
tellur mcp                          # Run MCP server over stdio
tellur gc --dry-run                 # Garbage-collect expired events
tellur redact                       # Redact secrets from stored events
tellur verify                       # Verify hash-chain integrity
```

`explain`, `blame`, and `sessions` support `--json` for machine-readable output.

## Supported Adapters

| Tool | Input | Status |
| --- | --- | --- |
| Claude Code | Hooks + transcript parsing | Working |
| Cursor | Agent Trace JSON import | Working |
| Aider | Git commit attribution import | Working |
| Codex CLI | JSONL event stream/session transcript import | Working |
| GitHub Copilot | Metadata JSON/JSONL import | Working |
| Generic | CLI events, JSONL, local HTTP daemon | Working |

The adapter layer is pluggable, so additional tools can normalize their events
into Tellur's schema without changing the core attribution engine.

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
```

## Roadmap

- Team/server mode for shared organizational visibility
- More first-party adapters for emerging AI coding tools
- Richer policy templates for security-sensitive repositories
- Packaged releases for npm, Homebrew, and GitHub Releases

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](./LICENSE).
