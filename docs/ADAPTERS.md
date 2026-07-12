# Tellur Adapter Notes

Last updated: 2026-07-12

## Current Guarantees

- `tellur setup agents` installs user-level Codex, Claude Code, Gemini CLI, and
  Antigravity hooks, Cursor MCP/settings, VS Code settings, and Windsurf
  MCP/settings once so capture can work automatically in new Git repositories.
- The VS Code-family extension declares itself a workspace extension and waits
  for Workspace Trust before running the CLI. It initializes/watches every
  workspace root independently and follows roots added or removed at runtime.
- Global hooks use an absolute path to the installed `tellur` executable and
  run `hooks ingest --source <agent> --auto-init`. Outside a Git repository
  they no-op; inside a Git repository they initialize `.tellur/` when needed.
- A repository can opt out of global hook capture by creating `.tellur/disable`.
- Imports preserve source `id`, `session_id`, `timestamp`, `event_type`,
  `actor`, and payload content.
- JSON-stream importers accept JSONL, arrays, single objects, and common
  envelope shapes. Envelope metadata is inherited by child events, and epoch
  seconds/milliseconds are normalized to RFC 3339 timestamps.
- Tellur always recomputes `prev_hash` and `event_hash` when imported events are
  appended to the local event log.
- Non-empty malformed JSON/JSONL input fails the import with a line-specific
  error instead of silently dropping data.
- Prompt-like fields (`message`, `prompt`, `text`, `content`) are hashed rather
  than stored as raw text across all import adapters, including nested `data`/
  `payload` objects.
- A repo may **opt in** to keeping a redacted, length-bounded prompt *excerpt*
  (alongside the hash) for live agent-hook capture by setting
  `redaction.store_prompt_excerpt: true` in `.tellur/config.yml`. It is `false`
  by default, uses the repo's own redaction rules (plus the built-in secret
  patterns), and applies only to activity captured after opting in — the hub
  session timeline then shows the excerpt.
- Secret-looking strings in retained adapter metadata are redacted with the core
  redaction engine.
- Codex and Claude Code hooks scope capture to the hook payload's file path when
  the tool provides one. If a hook payload does not include a file path, Tellur
  records the hook event but does not capture the whole working tree.
- Invalid hook JSON is ignored and never auto-initializes a repository.
- Setup refuses to overwrite malformed existing JSON config files.

## Current Adapters

| Adapter | Capture Mode | Notes |
| --- | --- | --- |
| Claude Code | User/project hooks and transcript import | Highest-fidelity first-party integration. User-level hooks can be installed once with `tellur setup agents`; project hooks remain available through `tellur hooks install claude-code`. Transcript import accepts JSONL/array/envelope exports, role messages, top-level tool records, and Anthropic `content`-block `tool_use` records. |
| Codex CLI/App | User hooks, personal plugin, JSONL/array/envelope import | User-level hooks can be installed once with `tellur setup agents`. A local Codex plugin is also generated for manual workflows and marketplace discovery. Imports preserve rollout source IDs/timestamps and track session metadata across subsequent events. |
| Gemini CLI | User hooks and JSONL import | `tellur setup gemini-cli` writes Gemini's documented `~/.gemini/settings.json` hooks for `BeforeTool`, `AfterTool`, agent, and session events. Hook commands return `{}` on stdout as Gemini requires. |
| Google Antigravity 2.0 | User hooks, MCP, JSONL import | `tellur setup antigravity` writes Antigravity hooks under `~/.gemini/config/hooks.json` and MCP configs under `~/.gemini/antigravity*/mcp_config.json`. |
| Cursor IDE/CLI | VS Code-compatible extension capture, global MCP, JSON/JSONL import | `tellur setup cursor` writes Cursor user settings and `~/.cursor/mcp.json`. Cursor does not currently expose a documented local IDE lifecycle hook equivalent to Codex hooks, so live capture is handled by the extension save/watch path and Cursor CLI traces can still be imported. |
| VS Code/Copilot | VS Code extension save/watch capture, metadata JSON/JSONL import | `tellur setup vscode` writes user settings so the installed extension can auto-init, watch, and capture saved files in every Git workspace. Prompt capture remains explicit because VS Code does not expose arbitrary Copilot prompts to extensions. |
| GitHub Copilot | Metadata JSON/JSONL/array/envelope import | Import-only. Handles editor/telemetry harness wrappers and preserves completion/suggestion correlation metadata. Does not intercept Copilot prompts directly because VS Code does not expose that API to extensions. |
| Windsurf / Cascade | VS Code-compatible extension capture, global MCP, JSON/JSONL session import | `tellur setup windsurf` writes Windsurf user settings and `~/.codeium/windsurf/mcp_config.json`. Windsurf is a VS Code-compatible editor, so live capture is handled by the extension save/watch path (source `windsurf`); Cascade session exports can still be imported. |
| JetBrains AI Assistant / Junie | JetBrains plugin save/watch capture + JSON/JSONL action-log import | The `editor/tellur-jetbrains` plugin subscribes to IDE virtual-file changes, coalesces duplicate file events, and routes saved/created files through a disposable bounded single-worker queue to `tellur hooks ingest --source jetbrains --auto-init`, capturing AI Assistant and Junie edits live. If a duplicate arrives while capture is already running, one follow-up capture is queued so the final file state is not dropped. Non-zero CLI exits and timeouts are logged. Exported action logs can still be imported. JetBrains MCP is configured in-IDE, not through a stable global config file, so Tellur does not auto-write it. |
| Devin | Daemon webhook live capture + run/session export import | The daemon's authenticated `POST /webhook/devin` normalizes Devin's native run/session payload (messages, shell commands, file edits, status) into Tellur events with a recomputed hash chain. Run exports (object/array/JSONL) can still be imported after the fact. |
| Continue | `dev_data` JSONL import | Reads Continue development-data files (`chat.jsonl`, `editInteraction.jsonl`, ...) where each line has a `name` and nested `data`. When Continue runs inside a VS Code-family editor, the Tellur extension save/watch path also captures its file edits live. |
| Cline / Roo Code | Task-history import | Reads a task's `ui_messages.json` / `api_conversation_history.json`; one adapter covers Cline and its Roo Code fork (shared format). When Cline/Roo runs inside a VS Code-family editor, the Tellur extension save/watch path also captures its file edits live. |
| Aider | Git log import | Uses Aider commit-message markers and file status from the source Git repository. |
| Generic | Tellur JSONL / CLI / daemon | For CI, custom tools, and local HTTP ingestion. |

## Integration Mechanisms

Tellur uses the strongest documented mechanism each surface exposes. Do not
model every editor as if it had Codex-style hooks.

| Mechanism | Used by | Strength | Implementation |
| --- | --- | --- | --- |
| User lifecycle hooks | Codex, Claude Code, Gemini CLI, Antigravity | Strongest local live capture when hook payloads include concrete file paths. | Global setup writes each tool's documented hook config with absolute `tellur hooks ingest --source <agent> --auto-init` commands; Gemini/Antigravity use `--json-response` because their hooks require JSON stdout. |
| Personal plugin / marketplace | Codex | Manual workflow discovery, not required per project. | Setup writes `~/.codex/plugins/tellur-provenance` and `~/.agents/plugins/marketplace.json`. |
| MCP server | Cursor, Antigravity, Windsurf, external agents | Tool access for status, explain, blame, verify, and policy checks. | Setup writes `~/.cursor/mcp.json`, `~/.codeium/windsurf/mcp_config.json`, and Antigravity MCP configs pointing to the absolute `tellur mcp` command. |
| VS Code-compatible extension | VS Code, Cursor, Windsurf | Best available editor-level live capture where lifecycle hooks are not documented. Also captures edits from agents that run inside the editor (e.g. Cline/Roo Code, Continue). | User settings enable `autoInit`, `autoWatch`, and `captureOnSave`; save capture routes through `hooks ingest` with an explicit setup source or host-detected `vscode`, `cursor`, or `windsurf`. One watcher runs per workspace root, including workspace-side remote extension hosts. |
| JetBrains plugin | JetBrains IDEs (AI Assistant, Junie) | Editor-level live capture for IntelliJ-family IDEs, which have no documented local hook. | `editor/tellur-jetbrains` subscribes to `VFS_CHANGES`, deduplicates repeated file events in each batch, and routes saved/created files through a disposable bounded single-worker queue to `hooks ingest --source jetbrains --auto-init`. Duplicate saves during an active capture queue one follow-up capture. Non-zero exits and timeouts are logged. |
| Daemon webhook | Devin, any cloud/CI agent | Live capture for cloud agents with no local surface. | `POST /webhook/{source}` (token-auth, loopback-only) normalizes a tool's native webhook payload into events and recomputes the hash chain (`crates/core/src/daemon/webhook.rs`). |
| Import adapters | Claude Code, Cursor, Codex, Copilot, Aider, Gemini CLI, Antigravity, Windsurf, JetBrains, Devin, Continue, Cline/Roo Code, Generic | Historical or metadata-based evidence. | `tellur import <adapter> <source>` normalizes external event streams while preserving source identity and timestamps. JSONL/array/envelope adapters share one tolerant parsing loop (`crates/adapters/src/import.rs`); each adapter only defines its event-type mapping. |
| Git/policy fallback | All editors | Enforcement at review/commit time. | `tellur policy check`, PR reports, Git notes, and future pre-commit/CI wiring catch gaps in editor APIs. |

## Adoption Adapter Roadmap

The 2026-Q2 adoption batch first shipped as import adapters (see Current
Adapters): Windsurf/Cascade, JetBrains AI Assistant / Junie, Devin, Continue,
and Cline/Roo Code. None of these tools expose a documented local
lifecycle-hook surface comparable to Codex or Gemini CLI, so live capture is
added through whichever durable surface each one does expose:

- **Windsurf/Cascade** — now has live capture. `tellur setup windsurf` (and
  `tellur setup agents`) writes Windsurf user settings and
  `~/.codeium/windsurf/mcp_config.json`. Because Windsurf is VS Code-compatible,
  the Tellur extension's save/watch path captures edits live with source
  `windsurf`, mirroring the Cursor integration.
- **Continue, Cline / Roo Code** — captured live by the Tellur extension
  whenever they run inside a VS Code-family editor (VS Code, Cursor, or
  Windsurf), since the extension records file saves regardless of which agent
  made the edit. Their import adapters remain available for history outside a
  configured editor.
- **JetBrains AI Assistant / Junie** — now has live capture through the
  `editor/tellur-jetbrains` plugin, which subscribes to IDE virtual-file changes
  and routes saves to `hooks ingest --source jetbrains`. JetBrains MCP remains
  in-IDE configuration; Tellur does not auto-write it.
- **Devin** — now has live capture through the daemon's
  `POST /webhook/devin` endpoint, which normalizes Devin's native run/session
  payload. Run-export import remains available for after-the-fact provenance.

Next candidates, when they expose durable capture surfaces:

1. Live lifecycle-hook capture for Windsurf and JetBrains if/when those tools
   document a local hook API (today only Codex, Claude Code, Gemini CLI, and
   Antigravity do); the JetBrains plugin's VFS capture is the current best
   available surface.
2. Publishing the JetBrains plugin to the JetBrains Marketplace and the VS Code
   extension flow so users can install live capture without a manual build.

## Known Limits

- Codex requires users to review/trust non-managed hooks before they run. This is
  a one-time trust step per hook definition, not a per-project skill invocation.
- Codex discovers the generated personal plugin through
  `~/.agents/plugins/marketplace.json`. The marketplace path points to
  `./.codex/plugins/tellur-provenance`, relative to the personal marketplace
  root, matching Codex's documented local marketplace resolution.
- Codex hook interception is not a complete enforcement boundary. Tellur uses
  `PostToolUse` plus working-tree capture for provenance and keeps `watch` and
  imports as fallbacks.
- Gemini CLI hooks are configured in `~/.gemini/settings.json` using the
  documented `hooks` object. Tellur writes only command hooks and does not alter
  model/tool policy settings.
- Antigravity 2.0 is split between hook capture and MCP tool access. Hook
  capture goes through `~/.gemini/config/hooks.json`; MCP access is configured
  separately for Antigravity app/CLI.
- Cursor IDE integration is deliberately built through documented surfaces:
  global MCP configuration plus the VS Code-compatible Tellur extension. Cursor
  background-agent webhooks are server/API notifications and are not a local IDE
  file-edit hook.
- Windsurf integration is built through the same documented surfaces as Cursor:
  global MCP configuration plus the VS Code-compatible Tellur extension. It does
  not assume a Windsurf-specific local lifecycle hook.
- VS Code/Copilot cannot expose every chat prompt or third-party agent action to
  another extension. Tellur therefore combines auto-init, save capture, watch,
  explicit prompt hashing, Git policy checks, and imports instead of pretending
  VS Code has Codex-style hooks.
- VS Code-family capture requires a trusted workspace because it executes the
  configured CLI and writes `.tellur/`. Browser-only virtual workspaces without
  a Node workspace extension host and runnable CLI are unsupported. Remote SSH,
  WSL, containers, and Codespaces require `tellur` in the remote environment.
- Claude Code command hooks run with the user's OS permissions. Tellur-installed
  commands are intentionally small and route through the local `tellur` CLI.
- Import-only adapters can prove what was present in the imported source, not
  that Tellur observed the action live.
- Cursor, Copilot, Windsurf, JetBrains, Devin, Continue, and Cline/Roo Code
  imports should be validated against representative real exports before treating
  their normalized event coverage as complete. Their event-type mappings are
  intentionally tolerant: unknown event kinds are preserved as
  `<tool>.<kind>` custom events rather than dropped, and ambiguous tool actions
  (e.g. a Cline `tool` message that may be a read or a write) are kept neutral
  rather than guessed as file writes.
- The JetBrains plugin captures any file written through the IDE's VFS, not only
  AI-authored edits; origin (AI vs human) is decided by the core attribution
  layer, the same as the VS Code extension. Capture is best-effort but no longer
  silently swallows CLI failures: non-zero exits and timeouts are logged, and
  duplicate captures for the same file/repo are coalesced while queued/running;
  if another save arrives during an active capture, the plugin queues one
  follow-up capture for the final file state. The worker queue is owned by an
  IntelliJ application service and shut down on plugin disposal. It builds
  outside the Rust workspace CI (JDK 17 + IntelliJ SDK via Gradle, committed
  wrapper); verify with `./gradlew buildPlugin` rather than `cargo test`.
- The JetBrains plugin descriptor supports platform builds 241 through 253
  (2024.1–2025.3). It is compiled/tested against the oldest supported SDK so it
  remains loadable there; updating the pinned Gradle/IntelliJ tooling and adding
  current cross-version Plugin Verifier runs remains follow-up work.
- The daemon webhook (`POST /webhook/{source}`) is a tolerant normalizer, not a
  signed channel: it proves what a caller posted to the local, token-protected
  endpoint, not that Devin's cloud independently attested the events. Validate it
  against representative real Devin payloads before treating coverage as
  complete.
- For line-level attribution, hooks and live capture are stronger evidence than
  metadata-only imports.
