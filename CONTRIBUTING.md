# Contributing to Tellur

Thanks for your interest in contributing! Tellur is an open-source AI code provenance platform and we welcome contributions of all kinds.

## Development Setup

Tellur's core and CLI are written in **Rust**; the VS Code extension is
TypeScript and the JetBrains plugin is Kotlin.
Agents should also read [`AGENTS.md`](./AGENTS.md) before making changes — start
with its "Start Here" orientation and Architecture Map. It defines repo-wide
requirements for keeping documentation aligned with behavior, and is the fastest
way to learn where each layer lives.

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
│   │                  #   capture pipeline, export, reports, git notes, remap,
│   │                  #   daemon (incl. webhook), MCP server
│   ├── cli/           # CLI binary (the `tellur` command)
│   └── adapters/      # AI tool adapters (Claude Code, Aider, Cursor, Codex,
│                      #   Copilot, Gemini CLI, Antigravity, Windsurf, JetBrains,
│                      #   Devin, Continue, Cline/Roo, Generic)
├── schemas/           # JSON Schema definitions
├── editor/
│   ├── tellur-vscode/    # VS Code / Cursor / Windsurf extension (TypeScript)
│   └── tellur-jetbrains/ # JetBrains IDE plugin (Kotlin/Gradle)
├── web/               # Static session-replay dashboard
├── dist/              # Packaging: npm wrapper, Homebrew formula
└── docs/              # Documentation (ADAPTERS.md, FINDINGS.md)
```

The authoritative, more detailed map lives in [`AGENTS.md`](./AGENTS.md).

## Editor Integrations

### VS Code extension (also Cursor and Windsurf)

```bash
cd editor/tellur-vscode
npm install
npm run compile
```

The extension shells out to the `tellur` binary and consumes its `--json`
output (`explain --json`, `blame --json`, `sessions --json`). It also uses
`tellur hooks ingest --source <vscode|cursor|windsurf> --auto-init` for save
capture. The same extension serves VS Code, Cursor, and Windsurf, which are all
VS Code-compatible.

### JetBrains plugin

```bash
cd editor/tellur-jetbrains
./gradlew buildPlugin  # or: ./gradlew runIde   (use JAVA_HOME=<jdk-17> if your default JDK is newer)
```

This plugin is Kotlin and built with Gradle + the IntelliJ Platform SDK (JDK 17,
network download on first run). The Gradle wrapper is committed (pinned to 8.9),
so no global Gradle is needed. It is **not** built by `cargo`/the Rust CI, so
verify plugin changes by building/running it with Gradle. See the Verification
section of [`AGENTS.md`](./AGENTS.md) for details.

The supported end-user onboarding and configuration-reconciliation paths are:

```bash
curl -fsSL https://github.com/sydneyvb-nl/tellur/releases/latest/download/install.sh | bash
tellur setup             # wizard, normally launched by the installer
tellur setup update      # refresh generated paths/hooks after a binary upgrade
tellur setup status      # combined machine/current-repo health
```

The granular commands below exist for integration development and recovery;
do not present them as the normal README setup journey:

```bash
tellur setup agents      # all hook/MCP/settings generators
tellur setup cursor      # Cursor MCP/settings only
tellur setup vscode      # VS Code extension settings only; no VSIX install
tellur setup windsurf    # Windsurf MCP/settings only
tellur setup gemini-cli  # Gemini CLI hooks only
tellur setup antigravity # Antigravity hooks/MCP only
```

Release packaging is defined in `.github/workflows/release.yml`. Every `v*` tag
builds platform CLI archives, a version-matched VSIX, a version-matched
JetBrains ZIP, SHA-256 sidecars, and the two bootstrap installers. CI builds both
editor packages and runs the Unix installer E2E test before changes can merge.

Devin (cloud agent) has no local editor surface; capture it live by POSTing its
webhook to the local daemon's `POST /webhook/devin` endpoint. JetBrains live
capture uses the plugin above.

## Code Style

- Rust: keep `cargo clippy --workspace --all-targets -- -D warnings` clean; run `cargo fmt`.
- VS Code extension: run `npm run compile`, `npm run test:unit`, and when practical `npm run test:extension` from `editor/tellur-vscode`.
- JetBrains plugin: build with `./gradlew buildPlugin` from `editor/tellur-jetbrains` (Gradle, not cargo).
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
