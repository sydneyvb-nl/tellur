# Contributing to Tellur

Thanks for your interest in contributing! Tellur is an open-source AI code provenance platform and we welcome contributions of all kinds.

## Development Setup

Tellur's core and CLI are written in **Rust**; the editor extension is TypeScript.
Agents should also read [`AGENTS.md`](./AGENTS.md) before making changes. It
defines repo-wide requirements for keeping documentation aligned with behavior.

```bash
# Clone the repo
git clone https://github.com/sydneyvb-nl/tellur.git
cd Tellur

# Build all crates
cargo build

# Run the test suite
cargo test

# Run the CLI
cargo run -p tellur-cli -- init
cargo run -p tellur-cli -- doctor
```

Rust toolchain: stable (edition 2024). Install via [rustup](https://rustup.rs).

## Project Structure

```
Tellur/
├── crates/
│   ├── core/          # Schemas, attribution engine, storage, policy, redaction,
│   │                  #   capture pipeline, export, daemon, MCP server
│   ├── cli/           # CLI binary (the `tellur` command)
│   └── adapters/      # AI tool adapters (Claude Code, Aider, Cursor, Codex,
│                      #   Copilot, Gemini CLI, Antigravity, Generic)
├── schemas/           # JSON Schema definitions
├── editor/            # VS Code extension (TypeScript)
├── dist/              # Packaging: npm wrapper, Homebrew formula
└── docs/              # Documentation (incl. FINDINGS.md)
```

## Editor Extension

```bash
cd editor/tellur-vscode
npm install
npm run compile
```

The extension shells out to the `tellur` binary and consumes its `--json`
output (`explain --json`, `blame --json`, `sessions --json`). It also uses
`tellur hooks ingest --source <vscode|cursor> --auto-init` for save capture.

Global editor setup is configured through:

```bash
tellur setup agents      # Codex, Claude Code, Cursor, and VS Code
tellur setup cursor      # Cursor MCP/settings only
tellur setup vscode      # VS Code settings only
tellur setup gemini-cli  # Gemini CLI hooks only
tellur setup antigravity # Antigravity hooks/MCP only
```

## Code Style

- Rust: keep `cargo clippy --workspace --all-targets -- -D warnings` clean; run `cargo fmt`.
- Editor: run `npm run compile`, `npm run test:unit`, and when practical `npm run test:extension` from `editor/tellur-vscode`.
- No `unwrap()`/`panic!` on user-reachable paths — return `anyhow::Result`.
- Match the surrounding code's naming and comment density.

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Claude Code hook installer
fix: handle missing git root in init command
docs: add adapter authoring guide
test: add event schema validation tests
chore: update dependencies
```

## Pull Requests

1. Create a feature branch from `main`.
2. Make your changes with tests.
3. Ensure `cargo build` and `cargo test` pass.
4. Submit a PR with a clear description.

## Reporting Issues

- Use GitHub Issues.
- Include steps to reproduce.
- Include the Tellur version (`tellur --version`).
- Include your OS and Rust toolchain (`rustc --version`).

## License

By contributing, you agree that your contributions will be licensed under the Apache-2.0 license.
