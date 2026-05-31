# Tellur

**AI Code Provenance for Teams**

> Who changed that function? Which model generated it? What prompt and context produced that change? Did tests pass? Who reviewed it?

Tellur is an open-source AI code provenance platform that records, attributes, and reports on AI-assisted development. It gives teams line-level AI blame, session replay, PR risk reports, and policy-as-code — without uploading your code anywhere.

Git tells you *what* changed. Tellur tells you *how AI participated*.

## Status

**Beta.** The full local pipeline is functional end-to-end: capture (watch + Claude Code hooks) → line attribution → SQLite index → `explain`/`blame`/`pr-report`/`verify`, plus policy-as-code, secret redaction, provenance export, a token-authenticated local daemon, an MCP server, and a VS Code extension. Team/server mode and additional adapters (Copilot, Codex) are planned.

## Why Tellur?

AI coding tools (Cursor, Claude Code, Aider, Copilot, Codex, Windsurf, Gemini CLI) write production code every day. But teams have no visibility into:

- Which code was AI-generated vs human-written
- What prompts, models, and agents produced specific changes
- Whether tests were run before AI code was committed
- Whether sensitive files were accessed by agents
- Whether AI changes were properly reviewed

## Architecture

```
Tellur/
├── crates/
│   ├── core/          # Core library — schemas, attribution, storage, policy, redaction, export
│   ├── cli/           # CLI binary (tellur command)
│   └── adapters/      # AI tool adapters (Claude Code, Aider, Cursor, Generic)
├── schemas/           # JSON Schema definitions
└── .github/           # GitHub Action for PR checks
```

**Tech stack:** Rust (core + CLI), SQLite (index), JSONL (append-only event log)

## Features (implemented)

- **Line-level AI attribution** — maps code ranges to AI agent, model, prompt hash, and confidence score
- **Tamper-evident event log** — SHA-256 hash chain across all events in JSONL format
- **SQLite index** — fast queries for CLI, editor, and PR reports
- **Policy engine** — YAML-based rules for sensitive paths, required reviews, and test evidence
- **Secret redaction** — regex-based detection and sanitization of API keys, tokens, passwords
- **PR risk reports** — risk scoring, AI involvement stats, reviewer checklist, markdown output
- **Provenance export** — 6 profiles (developer, OSS, corporate, audit, release, CI)
- **File change capture** — git diff integration with blob SHA tracking
- **Adapter interface** — async trait for pluggable AI tool integrations
- **GitHub Action** — automated PR provenance checks

## CLI

```bash
# Install (from source)
cargo install --path crates/cli

# Initialize in a repository
tellur init

# Check setup and detect AI tools
tellur doctor

# Start capturing AI development activity
tellur watch

# Explain who/what changed a specific line
tellur explain src/auth/session.ts:84

# Show AI attribution for a file
tellur blame src/auth/session.ts

# Generate a PR risk report
tellur pr-report --base main --head feature/auth

# Check policy compliance
tellur policy check

# Emit a single event (generic adapter / CI)
tellur event --event-type file.write --session $SESSION --file src/api.ts

# Verify provenance integrity (hash chain)
tellur verify

# Export provenance data
tellur export --format json

# Install AI-tool hooks (Claude Code) for automatic capture
tellur hooks install

# Run the local event-ingestion daemon (loopback only, token-authenticated)
tellur daemon

# Run the MCP server over stdio (for AI agents)
tellur mcp

# Garbage-collect events past the retention window
tellur gc --dry-run

# Redact secrets from stored events
tellur redact
```

`explain`, `blame`, and `sessions` accept `--json` for machine-readable output
(used by the editor extension and CI).

## Data Model

Tellur stores data in `.tellur/` within your repository:

```
.tellur/
├── config.yml           # Configuration (committed)
├── policies/
│   └── default.yml      # Policy rules (committed)
├── traces/
│   └── sessions/        # JSONL event logs (gitignored by default)
│       └── 2026/05/
│           └── events-2026-05-31.jsonl
├── index/
│   └── tellur.db      # SQLite index (gitignored)
└── exports/             # Generated provenance bundles
```

### Schemas

All data conforms to versioned schemas:

| Schema | Description |
|--------|-------------|
| `tellur.session.v1` | A bounded AI-assisted development interaction |
| `tellur.event.v1` | A timestamped action within a session |
| `tellur.attribution.v1` | Line-level origin mapping for a file |
| `tellur.pr-report.v1` | PR risk report with AI involvement stats |
| `tellur.provenance.v1` | Portable export bundle |

JSON Schema definitions are in [`schemas/`](./schemas/).

## Supported AI Tools

| Tool | Adapter | Status |
|------|---------|--------|
| Claude Code | Hooks + transcript | Working (`tellur hooks install`) |
| Cursor | Agent Trace import | Working (`tellur import cursor <file>`) |
| Aider | Git commit attribution | Working (`tellur import aider <repo>`) |
| Generic | CLI + JSONL + HTTP daemon | Working |
| Codex CLI | JSONL event stream import | Working (`tellur import codex <file>`) |
| GitHub Copilot | Metadata import | Working (`tellur import copilot <file>`) |

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

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Run CLI
cargo run -p tellur-cli -- init
cargo run -p tellur-cli -- doctor
```

## Roadmap

- [x] Claude Code hook installer
- [x] Aider commit attribution import
- [x] Cursor Agent Trace import
- [x] VS Code extension (TypeScript)
- [x] Local HTTP event API (daemon mode, loopback + token auth)
- [x] MCP server (stdio)
- [x] Git remapping across rebases
- [x] SLSA/SPDX export integration
- [x] Homebrew formula
- [x] Session replay web dashboard (static UI + local daemon live data)
- [ ] Team/server mode
- [x] GitHub Copilot / Codex CLI adapters

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](./LICENSE).
