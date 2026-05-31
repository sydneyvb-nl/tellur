# TraceGit

**AI Code Provenance for Teams**

> Who changed that function? Which model generated it? What prompt and context produced that change? Did tests pass? Who reviewed it?

TraceGit is an open-source AI code provenance platform that records, attributes, and reports on AI-assisted development. It gives teams line-level AI blame, session replay, PR risk reports, and policy-as-code — without uploading your code anywhere.

Git tells you *what* changed. TraceGit tells you *how AI participated*.

## Status

**Early development.** Core engine, CLI, and schemas are functional. Editor extension and full adapter support are in progress.

## Why TraceGit?

AI coding tools (Cursor, Claude Code, Aider, Copilot, Codex, Windsurf, Gemini CLI) write production code every day. But teams have no visibility into:

- Which code was AI-generated vs human-written
- What prompts, models, and agents produced specific changes
- Whether tests were run before AI code was committed
- Whether sensitive files were accessed by agents
- Whether AI changes were properly reviewed

## Architecture

```
TraceGit/
├── crates/
│   ├── core/          # Core library — schemas, attribution, storage, policy, redaction, export
│   ├── cli/           # CLI binary (tracegit command)
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
tracegit init

# Check setup and detect AI tools
tracegit doctor

# Start capturing AI development activity
tracegit watch

# Explain who/what changed a specific line
tracegit explain src/auth/session.ts:84

# Show AI attribution for a file
tracegit blame src/auth/session.ts

# Generate a PR risk report
tracegit pr-report --base main --head feature/auth

# Check policy compliance
tracegit policy check

# Emit a single event (generic adapter / CI)
tracegit event --event-type file.write --session $SESSION --file src/api.ts

# Verify provenance integrity (hash chain)
tracegit verify

# Export provenance data
tracegit export --format json
```

## Data Model

TraceGit stores data in `.tracegit/` within your repository:

```
.tracegit/
├── config.yml           # Configuration (committed)
├── policies/
│   └── default.yml      # Policy rules (committed)
├── traces/
│   └── sessions/        # JSONL event logs (gitignored by default)
│       └── 2026/05/
│           └── events-2026-05-31.jsonl
├── index/
│   └── tracegit.db      # SQLite index (gitignored)
└── exports/             # Generated provenance bundles
```

### Schemas

All data conforms to versioned schemas:

| Schema | Description |
|--------|-------------|
| `tracegit.session.v1` | A bounded AI-assisted development interaction |
| `tracegit.event.v1` | A timestamped action within a session |
| `tracegit.attribution.v1` | Line-level origin mapping for a file |
| `tracegit.pr-report.v1` | PR risk report with AI involvement stats |
| `tracegit.provenance.v1` | Portable export bundle |

JSON Schema definitions are in [`schemas/`](./schemas/).

## Supported AI Tools

| Tool | Adapter | Status |
|------|---------|--------|
| Claude Code | Hooks + transcript | Adapter built, hooks pending |
| Cursor | Agent Trace import | Adapter built, import pending |
| Aider | Git commit attribution | Adapter built, import pending |
| GitHub Copilot | Metadata capture | Planned |
| Codex CLI | Event stream | Planned |
| Generic | CLI + HTTP API | Working |

## Policy Example

```yaml
# .tracegit/policies/default.yml
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

# Run tests (35 tests)
cargo test

# Run CLI
cargo run -p tracegit-cli -- init
cargo run -p tracegit-cli -- doctor
```

## Roadmap

- [ ] Claude Code hook installer
- [ ] Aider commit attribution import
- [ ] Cursor Agent Trace import/export
- [ ] VS Code extension (TypeScript)
- [ ] Session replay web dashboard
- [ ] Local HTTP event API (daemon mode)
- [ ] Git remapping across rebases
- [ ] SLSA/SPDX export integration
- [ ] Team/server mode
- [ ] Homebrew formula

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](./LICENSE).
