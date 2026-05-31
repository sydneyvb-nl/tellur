# TraceGit

**AI Code Provenance for Teams**

Who changed that function? Which model generated it? What prompt and context produced that change? Did tests pass? Who reviewed it?

TraceGit is an open-source AI code provenance platform that records, attributes, and reports on AI-assisted development. It gives teams line-level AI blame, session replay, PR risk reports, and policy-as-code — without uploading your code anywhere.

## Why TraceGit?

AI coding tools (Cursor, Claude Code, Aider, Copilot, Codex, Windsurf, Gemini CLI) write production code every day. But teams have no visibility into:

- Which code was AI-generated vs human-written
- What prompts, models, and agents produced specific changes
- Whether tests were run before AI code was committed
- Whether sensitive files were accessed by agents
- Whether AI changes were properly reviewed

Git tells you *what* changed. TraceGit tells you *how AI participated*.

## Features

- **Line-level AI attribution** — know which agent, model, and prompt produced every line
- **Session replay** — reconstruct what happened during AI development sessions
- **PR risk reports** — highlight AI-generated changes, sensitive areas, missing tests
- **Policy engine** — define rules for what AI is allowed to do in your codebase
- **Vendor-neutral** — works with any AI coding tool through adapters
- **Local-first** — no cloud required, no code uploaded, no SaaS dependency
- **MCP-native** — integrates via Model Context Protocol for zero-friction adoption
- **CI-ready** — GitHub Action, GitLab CI, and CLI for automated checks

## Quick Start

```bash
# Install
npm install -g tracegit

# Initialize in a repository
tracegit init

# Check setup
tracegit doctor

# Start capturing AI development activity
tracegit watch
```

## Usage

```bash
# See who/what changed a specific line
tracegit explain src/auth/session.ts:84

# Show AI attribution for a file
tracegit blame src/auth/session.ts

# Generate a PR risk report
tracegit pr-report

# Check policy compliance
tracegit policy check

# Export provenance data
tracegit export --format agent-trace
```

## Supported AI Tools

| Tool | Adapter | Status |
|------|---------|--------|
| Claude Code | Hooks + transcript | Planned |
| Cursor | Agent Trace import | Planned |
| Aider | Git commit attribution | Planned |
| GitHub Copilot | Metadata capture | Planned |
| Codex CLI | Event stream | Planned |
| OpenClaw | Custom adapter | Planned |
| Generic | CLI + HTTP API | Planned |

## Architecture

```
tracegit/
├── packages/
│   ├── core/          # Schemas, attribution engine, policy engine
│   ├── cli/           # CLI interface
│   ├── adapters/      # AI tool adapters (Claude Code, Cursor, etc.)
│   └── vscode/        # VS Code extension
├── schemas/           # JSON Schema definitions
└── docs/              # Documentation
```

## Contributing

We welcome contributions. See [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

## License

Apache-2.0 — see [LICENSE](./LICENSE) for details.
