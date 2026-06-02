# Tellur Adapter Notes

Last updated: 2026-06-02

## Current Guarantees

- `tellur setup agents` installs user-level Codex, Claude Code, Gemini CLI, and
  Antigravity hooks, Cursor MCP/settings, VS Code settings, and Windsurf
  MCP/settings once so capture can work automatically in new Git repositories.
- Global hooks use an absolute path to the installed `tellur` executable and
  run `hooks ingest --source <agent> --auto-init`. Outside a Git repository
  they no-op; inside a Git repository they initialize `.tellur/` when needed.
- A repository can opt out of global hook capture by creating `.tellur/disable`.
- Imports preserve source `id`, `session_id`, `timestamp`, `event_type`,
  `actor`, and payload content.
- Tellur always recomputes `prev_hash` and `event_hash` when imported events are
  appended to the local event log.
- Non-empty malformed JSON/JSONL input fails the import with a line-specific
  error instead of silently dropping data.
- Prompt-like fields (`message`, `prompt`, `text`, `content`) are hashed rather
  than stored as raw text across all import adapters, including nested `data`/
  `payload` objects.
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
| Claude Code | User/project hooks and transcript import | Highest-fidelity first-party integration. User-level hooks can be installed once with `tellur setup agents`; project hooks remain available through `tellur hooks install claude-code`. |
| Codex CLI/App | User hooks, personal plugin, JSONL import | User-level hooks can be installed once with `tellur setup agents`. A local Codex plugin is also generated for manual workflows and marketplace discovery. |
| Gemini CLI | User hooks and JSONL import | `tellur setup gemini-cli` writes Gemini's documented `~/.gemini/settings.json` hooks for `BeforeTool`, `AfterTool`, agent, and session events. Hook commands return `{}` on stdout as Gemini requires. |
| Google Antigravity 2.0 | User hooks, MCP, JSONL import | `tellur setup antigravity` writes Antigravity hooks under `~/.gemini/config/hooks.json` and MCP configs under `~/.gemini/antigravity*/mcp_config.json`. |
| Cursor IDE/CLI | VS Code-compatible extension capture, global MCP, JSON/JSONL import | `tellur setup cursor` writes Cursor user settings and `~/.cursor/mcp.json`. Cursor does not currently expose a documented local IDE lifecycle hook equivalent to Codex hooks, so live capture is handled by the extension save/watch path and Cursor CLI traces can still be imported. |
| VS Code/Copilot | VS Code extension save/watch capture, metadata JSON/JSONL import | `tellur setup vscode` writes user settings so the installed extension can auto-init, watch, and capture saved files in every Git workspace. Prompt capture remains explicit because VS Code does not expose arbitrary Copilot prompts to extensions. |
| GitHub Copilot | Metadata JSON/JSONL import | Import-only. Does not intercept Copilot prompts directly because VS Code does not expose that API to extensions. |
| Windsurf / Cascade | VS Code-compatible extension capture, global MCP, JSON/JSONL session import | `tellur setup windsurf` writes Windsurf user settings and `~/.codeium/windsurf/mcp_config.json`. Windsurf is a VS Code-compatible editor, so live capture is handled by the extension save/watch path (source `windsurf`); Cascade session exports can still be imported. |
| JetBrains AI Assistant / Junie | JSON/JSONL action-log import | Import-only today. Covers the AI Assistant plugin and the Junie agent across IntelliJ-family IDEs from an exported action log. JetBrains MCP is configured in-IDE, not through a stable global config file, so Tellur does not auto-write it. |
| Devin | Run/session export import | Import-only by default. Reads a Devin run object (or array/JSONL) of messages, shell commands, and file edits for per-run provenance. Real-time capture is possible by posting events to the authenticated local daemon (`POST /events`). |
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
| VS Code-compatible extension | VS Code, Cursor, Windsurf | Best available editor-level live capture where lifecycle hooks are not documented. Also captures edits from agents that run inside the editor (e.g. Cline/Roo Code, Continue). | User settings enable `autoInit`, `autoWatch`, and `captureOnSave`; save capture routes through `hooks ingest` with source `vscode`, `cursor`, or `windsurf`. |
| Import adapters | Cursor, Codex, Copilot, Aider, Gemini CLI, Antigravity, Windsurf, JetBrains, Devin, Continue, Cline/Roo Code, Generic | Historical or metadata-based evidence. | `tellur import <adapter> <source>` normalizes external event streams while preserving source identity and timestamps. JSONL/array/envelope adapters share one tolerant parsing loop (`crates/adapters/src/import.rs`); each adapter only defines its event-type mapping. |
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
- **JetBrains AI Assistant / Junie** — import-only today. JetBrains MCP is
  configured in-IDE rather than through a stable global config file, so Tellur
  does not auto-write it; a dedicated JetBrains plugin would be needed for
  editor-level live capture.
- **Devin** — import-only by default. As a cloud agent it has no local
  file-edit surface; real-time capture is possible by posting events to the
  authenticated local daemon (`POST /events`).

Next candidates, when they expose durable capture surfaces:

1. Live lifecycle-hook capture for Windsurf and JetBrains if/when those tools
   document a local hook API (today only Codex, Claude Code, Gemini CLI, and
   Antigravity do).
2. A first-class Devin webhook ingestion path through the local daemon that
   normalizes Devin's native webhook payload, instead of requiring callers to
   post pre-shaped Tellur events.

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
- For line-level attribution, hooks and live capture are stronger evidence than
  metadata-only imports.
