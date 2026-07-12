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
importers for Cursor, Aider, Codex CLI, Gemini CLI, Antigravity, GitHub
Copilot, Windsurf/Cascade, JetBrains AI/Junie, Devin, Continue, and
Cline/Roo Code, a local token-authenticated daemon, an MCP stdio server,
provenance export, and Git notes interop. For teams, the self-hosted
`tellur-server` hub adds multi-tenant ingest/read/report/policy/export with OIDC
SSO, SCIM provisioning, durable jobs, and an embedded web dashboard
(Overview, repositories, sessions, policy compliance, people & access, audit log,
and exports).

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

`tellur init --profile` accepts `default`, `team`, and `oss-maintainer`.
Unsupported profile names fail fast instead of silently using default setup.

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

Most agents are captured **live** once you run `tellur setup agents` (see
[One-Time Agent Setup](#one-time-agent-setup)). Use `tellur import` to backfill
history, or for tools that only expose an export:

```bash
tellur import claude-code path/to/transcript.jsonl
tellur import codex       path/to/codex-events.jsonl
tellur import gemini-cli  path/to/gemini-events.jsonl
tellur import cursor      path/to/agent-trace.json
tellur import copilot     path/to/copilot-metadata.jsonl
tellur import aider       path/to/repo          # an Aider git repository
tellur import windsurf    path/to/cascade-session.jsonl
tellur import jetbrains   path/to/ai-assistant-export.json
tellur import devin       path/to/devin-run.json
tellur import continue    path/to/.continue/dev_data/chat.jsonl
tellur import cline       path/to/tasks/<id>/ui_messages.json
```

Also available: `antigravity` and `generic` (JSONL). See
`tellur import --help` for the full list.

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
tellur connect --hub <url>          # One-time zero-touch setup (login + capture + git hooks)
tellur login --hub <url>            # Sign in to a team hub (browser; no token to paste)
tellur push                         # Send captured events + AI attribution to the hub
tellur logout                       # Forget stored hub credentials
tellur export --format json         # Export provenance data
tellur notes export                 # Write Git AI-compatible refs/notes/ai
tellur notes import                 # Import refs/notes/ai into the local index
tellur notes push                   # Push refs/notes/ai to origin
tellur notes fetch                  # Fetch refs/notes/ai from origin
tellur team report                  # Aggregate AI involvement across a commit range (no server)
tellur daemon                       # Run local HTTP ingestion/dashboard API
tellur mcp                          # Run MCP server over stdio
tellur setup agents                 # Install one-time global agent/editor integrations
tellur setup cursor                 # Install Cursor MCP/settings integration
tellur setup vscode                 # Install VS Code extension settings
tellur setup windsurf               # Install Windsurf MCP/settings integration
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
| Windsurf / Cascade | Windsurf MCP/settings, VS Code-compatible extension save/watch capture, Cascade session JSON/JSONL import | Working |
| JetBrains AI Assistant / Junie | JetBrains plugin save/watch capture (`editor/tellur-jetbrains`) + action-log JSON/JSONL import | Working |
| Devin | Live capture via daemon webhook (`POST /webhook/devin`) + cloud agent run/session export import | Working |
| Continue | `dev_data` JSONL import; live save/watch capture when running in a VS Code-family editor | Working |
| Cline / Roo Code | Task-history JSON/JSONL import; live save/watch capture when running in a VS Code-family editor | Working |
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

### Live Capture Beyond Import

Some tools have no documented local lifecycle hook, so Tellur captures them
through the durable surface each one does expose:

- **JetBrains IDEs (AI Assistant / Junie)** — the
  [`editor/tellur-jetbrains`](editor/tellur-jetbrains) plugin subscribes to the
  IDE's virtual-file changes and reports saved/created files to
  `tellur hooks ingest --source jetbrains --auto-init`. Edits made by the
  JetBrains AI Assistant and the Junie agent land on disk through the same path,
  so they are captured live. JetBrains MCP is configured in-IDE, so Tellur does
  not auto-write it. The plugin deduplicates repeated VFS events for the same
  file/repository while a capture is queued or running, re-runs capture once more
  when a new save arrives during an active capture, sends captures through a
  disposable bounded single-worker queue, and logs CLI failures/timeouts for
  troubleshooting.
- **Devin (cloud agent)** — has no local file surface. Point a Devin webhook (or
  a small relay) at the local daemon's authenticated
  `POST /webhook/devin` endpoint, which normalizes Devin's native run/session
  payload (messages, shell commands, file edits, status) into Tellur events and
  recomputes the hash chain. The endpoint is generic: `POST /webhook/{source}`
  works for any tool whose webhook posts a similar shape.

The daemon webhook requires the same bearer token as the other mutating
endpoints (see `.tellur/daemon.token`) and only accepts loopback hosts:

```bash
curl -X POST http://127.0.0.1:4917/webhook/devin \
  -H "Authorization: Bearer $(cat .tellur/daemon.token)" \
  -H 'Content-Type: application/json' \
  -d '{"devin_run_id":"run-1","messages":[{"type":"edit","file_path":"src/app.py"}]}'
```

## One-Time Agent Setup

For Codex, Claude Code, Gemini CLI, Antigravity, Cursor, VS Code, and Windsurf,
Tellur supports user-level installation so users do not need to invoke a skill
or plugin in every project:

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
(`~/.cursor/mcp.json` plus Cursor user settings), VS Code user settings, and
Windsurf MCP/settings (`~/.codeium/windsurf/mcp_config.json` plus Windsurf user
settings) so the Tellur extension can auto-init, watch, and capture saved files
in every Git workspace.

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
Codex plugin, Cursor MCP/settings, VS Code settings, and Windsurf MCP/settings.

Cursor, VS Code, and Windsurf do not have the same documented local lifecycle
hook model as Codex. Tellur therefore uses the durable editor surfaces they do
expose: extension save/watch capture, MCP tools, explicit prompt hashing, Git
policy checks, and import adapters. Because Windsurf is a VS Code-compatible
editor, the same Tellur extension capture also records edits made by agents that
run inside it, including Cline / Roo Code and Continue.

### Integration Mechanisms

| Surface | Setup command | Files written | Runtime behavior |
| --- | --- | --- | --- |
| Claude Code | `tellur setup claude-code` or `tellur setup agents` | `~/.claude/settings.json` | Lifecycle hooks call `tellur hooks ingest --source claude-code --auto-init`; project hooks remain available via `tellur hooks install claude-code`. |
| Codex CLI/App | `tellur setup codex` or `tellur setup agents` | `~/.codex/hooks.json`, `~/.codex/plugins/tellur-provenance`, `~/.agents/plugins/marketplace.json` | User hooks call `tellur hooks ingest --source codex --auto-init`; local plugin exposes manual Tellur workflows through Codex's plugin directory. |
| Gemini CLI | `tellur setup gemini-cli` or `tellur setup agents` | `~/.gemini/settings.json` | Gemini `BeforeTool`/`AfterTool`/agent/session hooks call `tellur hooks ingest --source gemini-cli --auto-init --json-response`. |
| Antigravity 2.0 | `tellur setup antigravity` or `tellur setup agents` | `~/.gemini/config/hooks.json`, `~/.gemini/antigravity/mcp_config.json`, `~/.gemini/antigravity-cli/mcp_config.json` | Antigravity lifecycle hooks call `tellur hooks ingest --source antigravity --auto-init --json-response`; MCP exposes Tellur tools to Antigravity agents. |
| Cursor | `tellur setup cursor` or `tellur setup agents` | `~/.cursor/mcp.json`, Cursor user `settings.json` | Cursor can call Tellur MCP tools; the installed Tellur extension uses auto-init, watch, and save capture with source `cursor`. |
| VS Code | `tellur setup vscode` or `tellur setup agents` | VS Code user `settings.json` | The installed extension auto-inits Git workspaces, starts `tellur watch`, and captures saved files through safe hook ingestion with source `vscode`. |
| Windsurf / Cascade | `tellur setup windsurf` or `tellur setup agents` | `~/.codeium/windsurf/mcp_config.json`, Windsurf user `settings.json` | Windsurf can call Tellur MCP tools; the installed VS Code-compatible extension uses auto-init, watch, and save capture with source `windsurf`. |

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
│   ├── adapters/      # Claude Code, Cursor, Aider, Codex, Copilot, Gemini, Antigravity, Windsurf, JetBrains, Devin, Continue, Cline, Generic
│   └── server/        # Tier 1 team hub (tellur-server) — FSL-1.1-ALv2 (in progress)
├── editor/            # VS Code extension + JetBrains plugin (tellur-jetbrains)
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

## Team Reports (no server)

Because authorship notes travel over your existing Git remote, a whole team can
share AI provenance without running any server. After contributors push their
`refs/notes/ai`, anyone can aggregate a PR or branch range into one view:

```bash
tellur notes fetch                              # get teammates' notes from origin
tellur team report --base main --head HEAD      # Markdown summary
tellur team report --base main --head HEAD --json
```

The report intersects each commit's portable note with its actual zero-context
Git patch. It reports added/deleted PR lines, AI/human/unknown added lines, a
breakdown by tool/model/author, and both commit and line-level provenance
coverage. Missing or unparseable notes never become “0% AI”: the affected diff
lines are explicitly unknown and the report is marked `missing` or `partial`.
This is the no-server
("Tier 0") slice of the team/server roadmap; see
[`docs/proposals/TEAM_SERVER_MODE.md`](docs/proposals/TEAM_SERVER_MODE.md).

To post the report automatically on pull requests, copy the example workflow in
[`docs/examples/github-actions-team-report.yml`](docs/examples/github-actions-team-report.yml)
into `.github/workflows/`.

The repository's own PR workflow uses this Git-notes report. It deliberately
does not run the local-index `tellur pr-report` command in CI: a fresh checkout
does not contain a developer's local SQLite attribution index.

## Self-Hosted Team Hub (preview)

For teams that want a shared, server-backed view, Tellur ships an optional
self-hostable hub, `tellur-server` (in `crates/server`). It is **source-available
under FSL-1.1-ALv2** (the Apache-2.0 core/CLI/adapters are unaffected) and is
under active development — see
[`docs/proposals/TEAM_SERVER_MODE.md`](docs/proposals/TEAM_SERVER_MODE.md) and
[`docs/proposals/TEAM_SERVER_IMPLEMENTATION.md`](docs/proposals/TEAM_SERVER_IMPLEMENTATION.md).

Local-first stays the default: the hub is opt-in, loopback-bound unless you
explicitly allow otherwise, token-authenticated, and tenant-isolated.

### Run it

```bash
tellur-server admin create-org --name "Acme"
tellur-server admin create-token --org <org-id> --role admin   # printed once
tellur-server admin set-policy --org <org-id> --file policy.yml # optional
tellur-server                                                   # serve at 127.0.0.1:4920
```

Or run the whole thing (including the built dashboard) in a container:

```bash
docker compose -f dist/docker/docker-compose.yml up --build
```

A CLI client can pull a central policy into a repo:

```bash
tellur policy pull --org <org-id> --hub http://hub:4920 --token <token>
```

### API

All `/v1` routes are org-scoped and authenticated with a Bearer token (or, in
the browser, an SSO session cookie). Cross-org access is denied and audited.

- **Ingest** — `POST /v1/orgs/{org}/repos/{repo}/events` (secret redaction +
  re-verified per-repo hash chain) and `POST .../attributions`.
- **Read** — `GET .../repos`, `.../events`, `.../report`, `.../overview`,
  `.../sessions[/{id}]`, `.../audit`.
- **Policy** — `PUT/GET .../policies[/{name}]`; `POST/GET .../policies/compliance`
  (durable evaluation + snapshots).
- **Exports** — `POST .../export/events|audit|evidence` enqueue durable jobs
  (`GET .../jobs[/{id}]` to poll); per-repo `GET/POST .../export/slsa|spdx`
  produce SLSA v1.0 / SPDX SBOM attestations.
- **Device login** — `POST /v1/device/authorize` + `/v1/device/token` back the
  CLI's `tellur login` (RFC 8628 device grant); the human approves at
  `/auth/device`.
- **Operational** — `GET /healthz`, `/readyz`, `/metrics` (Prometheus); no auth,
  no tenant data.

### Storage & retention

The default backend is embedded **SQLite** (zero-config, single-node). For
horizontal scale set `TELLUR_DATABASE_URL` to a **Postgres** DSN; Postgres is
reached over NoTls, so keep it on a private network or behind a TLS-terminating
proxy. A background retention loop minimises data-at-rest: expired sessions and
stale login transactions are always pruned, finished jobs after
`TELLUR_RETENTION_DAYS`, and audit entries after `TELLUR_AUDIT_RETENTION_DAYS`
via a **sealed checkpoint** (old entries are deleted but the pruned prefix's tip
hash is kept, so the chain still verifies). Both windows default to `0` = keep
forever; the event provenance log is never pruned.

### Authentication & authorization

- **RBAC** — `viewer` / `contributor` / `admin`, plus **additive per-repo
  grants** (`PUT/DELETE .../repos/{repo}/roles/{member}` or
  `tellur-server admin grant-repo-role`). Effective role is `max(org role, grant)`;
  grants only elevate, never restrict.
- **SSO (OIDC)** — Authorization Code + PKCE. Set `TELLUR_OIDC_ISSUER`,
  `TELLUR_OIDC_CLIENT_ID`, `TELLUR_OIDC_CLIENT_SECRET`, `TELLUR_OIDC_REDIRECT_URI`
  to enable `/auth/login|callback|logout`. Login issues an opaque, server-stored
  session cookie (`HttpOnly` / `Secure` / `SameSite=Lax`). There is no open
  self-registration: pre-provision members with `tellur-server admin add-member`,
  matched by verified email on first login and bound to their OIDC subject. The
  issuer/endpoints must be **`https`** (ID-token integrity rests on TLS); `http`
  is allowed only for loopback, or — for a trusted private network / homelab —
  with the explicit, **insecure** opt-in `TELLUR_OIDC_ALLOW_INSECURE_HTTP=1`
  (e.g. a LAN Keycloak at `http://192.168.x.x:8080`). A non-secure issuer without
  the opt-in is logged loudly at startup and rejected at login.
- **SCIM 2.0** — mint an org-scoped token (`create-scim-token`) and point your
  IdP at `/scim/v2/Users` + `/scim/v2/Groups`. Deprovisioning (`DELETE` or
  `PATCH active=false`) revokes every credential type at once. A group named
  `tellur-admin` / `tellur-contributor` / `tellur-viewer` drives its members'
  role, recomputed on membership change.
- **GitHub App (source access + notes harvesting)** — optional, GitHub-only. Set
  `TELLUR_GITHUB_APP_ID` + `TELLUR_GITHUB_APP_PRIVATE_KEY` (or
  `TELLUR_GITHUB_APP_PRIVATE_KEY_FILE`) so the private-repo blob proxy authenticates
  with short-lived, per-repo `Contents:read` **installation tokens** instead of a
  stored PAT (`TELLUR_GITHUB_API_BASE` overrides the API base for GitHub
  Enterprise). Add `TELLUR_GITHUB_WEBHOOK_SECRET` and map the installation with
  `tellur-server admin set-github-installation` to let signed GitHub `push`
  webhooks auto-provision repos and harvest pushed `refs/notes/ai` commit
  attribution into the hub. The App needs **Contents: read** + **Metadata: read**
  for P2/P3; P4 PR checks will add checks/pull-request permissions. Full
  step-by-step setup: [`docs/GITHUB_APP_SETUP.md`](docs/GITHUB_APP_SETUP.md).

### Zero-touch setup (`tellur connect`)

`tellur connect` is the one-time umbrella that makes capture and sync automatic —
after running it once, a developer never has to run a `tellur` command again. From
inside a repository:

```bash
tellur connect --hub https://hub.example.com
```

it (1) runs `tellur login`, (2) installs the global editor/agent capture
integrations (`tellur setup agents`), and (3) installs two **git hooks**:

- **`post-commit`** refreshes `refs/notes/ai` for the new commit from the local
  attribution index.
- **`pre-push`** flushes events to the hub (`tellur push`) and pushes the notes
  ref alongside whatever remote you push to.

It also configures the repo to fetch notes (`remote.<remote>.fetch`) so notes
travel with `git fetch` — but only if that remote already exists (otherwise it's
skipped with a hint, so it never creates a phantom `origin` that would break a
later `git remote add`). The hooks are **chained** — a pre-existing hook of yours
is preserved (Tellur's commands are added in a clearly marked block), and every
hub-touching step is **best-effort**: an unreachable hub never blocks a commit or
push (the high-water mark means the next push catches up).

For an **always-on** push (sync even on an idle machine that isn't committing or
pushing), add `--background`:

```bash
tellur connect --hub https://hub.example.com --background --push-interval 900
```

This installs a per-user, per-repository OS service that runs `tellur push` on an
interval — **launchd** (`~/Library/LaunchAgents/dev.tellur.push.<id>.plist`) on
macOS, **systemd `--user`** (`~/.config/systemd/user/tellur-push-<id>.{service,timer}`)
on Linux. It is **opt-in**: without `--background`, capture and sync still happen
on every commit and `git push` via the hooks above — the service only adds the
between-pushes catch-up. (Other platforms report it as unsupported.)

```bash
tellur connect --status   # show what's installed in this repo (hooks, notes, service)
tellur connect --remove   # remove the hooks, notes config, and background service
```

Flags: `--no-login` / `--no-agents` skip those steps, `--no-browser` prints the
login URL instead of opening it, `--remote <name>` selects the remote used for
notes fetch config (default `origin`), and `--push-interval <secs>` sets the
background push cadence (default 900s, used with `--background`).

> **Privacy:** the `pre-push` hook publishes commit-level AI attribution
> (`refs/notes/ai`) to anyone with repo read access. This is deliberate and
> reversible with `tellur connect --remove`. The rich line-level/session data in
> `.tellur/traces/` stays gitignored and only ever flows to the hub.

### Connect a developer (`tellur login` + `tellur push`)

Developers couple their machine to the hub without copying a token. `tellur login`
runs a browser-based device-authorization flow (RFC 8628): the CLI prints a short
code, opens the hub's approval page, and the signed-in member confirms it. The hub
then mints a member API token and the CLI stores it under the per-user config dir
(`~/.config/tellur/hosts.json`, `0600`). It requires SSO to be enabled.

```bash
tellur login --hub https://hub.example.com   # opens a browser; approve the code
tellur push                                   # send this repo's events to the hub
```

`tellur push` reads locally-captured events and forwards them to the ingest API,
tracking a per-`(hub, org, repo)` high-water mark in `.tellur/push_state.json` so
repeated runs are **incremental and idempotent** (no duplicates). The hub, org,
and token default to the stored login; override any of them with `--hub`, `--org`,
`--repo`, `--token` (or `TELLUR_HUB_URL` / `TELLUR_HUB_ORG` / `TELLUR_HUB_TOKEN`).
The repo name defaults to the working directory's name and is created on the hub
on first push. Use `--dry-run` to preview and `--reset` to re-push from scratch.
For unattended CI, skip `login` and pass a `tellur-server admin create-token`
token via `--token` / `TELLUR_HUB_TOKEN`.

### Team dashboard

The hub serves a built-in dashboard at **`/app`** — a Svelte SPA embedded in the
binary and served same-origin, so it reuses your first-party SSO session. Sign in
at `/auth/login`, then open `/app`. It is compiled into the binary at build time,
so build the SPA before the server (otherwise `/app` serves a placeholder; the
Docker image does this for you):

```bash
pnpm --dir crates/server/ui install
pnpm --dir crates/server/ui build      # → crates/server/ui/dist (embedded)
cargo run -p tellur-server             # /app now serves the real dashboard
```

Screens (per [`docs/proposals/TEAM_DASHBOARD_UI.md`](docs/proposals/TEAM_DASHBOARD_UI.md)):

- **Overview** — org totals, AI-share + review-coverage rollups, activity trend,
  and repos ranked by review gap, in one round-trip (`GET .../overview`).
- **Repositories & file provenance** — per-repo stats and a metadata-first
  attribution gutter. An admin connects a repo to its provider from a
  **Source connection** card (pick GitHub/GitLab/Bitbucket + `owner/repo` +
  branch — no template syntax to learn), enabling per-range **deep-links** and an
  inline source view. **Public** repos are fetched in the browser straight from
  the provider (the hub stores/serves no source); **private** repos use a stored,
  least-privilege token and are fetched through the hub's **SSRF-guarded blob
  proxy** (`GET .../blob`) — the token never leaves the hub. Also settable via
  `tellur-server admin set-repo-source`. For **GitHub** repos you can skip the
  stored PAT entirely: configure a **GitHub App** (`TELLUR_GITHUB_APP_ID` +
  `TELLUR_GITHUB_APP_PRIVATE_KEY` / `…_PRIVATE_KEY_FILE`, optional
  `TELLUR_GITHUB_API_BASE` for GitHub Enterprise) and the proxy mints a
  short-lived, per-repo, `Contents:read` **installation token** per fetch instead
  — auto-rotating, revoked by uninstalling the App, no human-managed secret in the
  DB. With `TELLUR_GITHUB_WEBHOOK_SECRET` and an installation mapping, the same
  App also accepts signed `push` webhooks at `/webhook/github`, syncs installed
  repos/source templates, and harvests pushed `refs/notes/ai` commit attribution
  idempotently. The PAT path stays as the fallback for GitLab/Bitbucket/self-managed
  (and GitHub when the App isn't installed).
- **Sessions & replay** — a dynamic per-session timeline: summary stats
  (events / duration / files / prompts), category + actor filters and search, and
  per-event nodes color-coded by kind (prompt / file / command / tool / test /
  git). Prompts appear inline **when prompt-excerpt capture is enabled** — by
  default Tellur stores only a prompt *hash*; set `redaction.store_prompt_excerpt:
  true` in `.tellur/config.yml` to also keep a secret-redacted, length-bounded
  excerpt (applies to activity captured after opting in).
- **Admin** — **Policies** compliance (violations by severity + one-click
  re-evaluation), **People & Access** (members, SCIM groups, SSO/SCIM health),
  an **Audit log** browser, and an **Exports** console.

A command palette (**⌘K** / **Ctrl-K**) jumps between screens, and topbar
controls switch theme (system / light / dark), density (cozy / compact), and
language (English / Dutch).

## Development

```bash
cargo fmt --check
cargo test
cargo run -p tellur-cli -- doctor
```

Dashboard SPA (`crates/server/ui`):

```bash
pnpm install
pnpm check && pnpm test      # svelte-check + vitest
pnpm e2e                      # Playwright (real bundle, mocked /v1 API)
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

The local pipeline, the multi-agent adapters, and the self-hosted hub (API, OIDC
SSO, SCIM user + group provisioning, durable exports, compliance snapshots,
retention, and the full team dashboard) are implemented. Active and upcoming work:

- **Zero-touch provenance + GitHub App** — automatic `refs/notes/ai` push and
  background hub sync are implemented, and the optional GitHub App now covers
  installation-token source access plus repo discovery / notes harvesting.
  Remaining: native PR Check Runs. Design:
  [`docs/proposals/GITHUB_APP.md`](docs/proposals/GITHUB_APP.md).
- **Packaged releases** for npm, Homebrew, and GitHub Releases.
- **Richer policy templates** for security-sensitive repositories.
- **Broader agent coverage** as more tools expose stable local lifecycle hooks.

The `tellur-core` binary is an internal diagnostic entrypoint for packaging
smoke tests. Use the `tellur` binary for normal CLI workflows.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](./LICENSE).
