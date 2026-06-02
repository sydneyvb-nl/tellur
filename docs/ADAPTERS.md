# Tellur Adapter Notes

Last updated: 2026-06-02

## Current Guarantees

- `tellur setup agents` installs user-level Codex, Claude Code, Gemini CLI, and
  Antigravity hooks, Cursor MCP/settings, and VS Code settings once so capture
  can work automatically in new Git repositories.
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
| Windsurf / Cascade | JSONL/JSON session import | Import-only. Normalizes Cascade tool calls, file edits, terminal commands, and chat turns. |
| JetBrains AI Assistant / Junie | JSON/JSONL action-log import | Import-only. Covers the AI Assistant plugin and the Junie agent across IntelliJ-family IDEs from an exported action log. |
| Devin | Run/session export import | Import-only. Reads a Devin run object (or array/JSONL) of messages, shell commands, and file edits for per-run provenance. |
| Continue | `dev_data` JSONL import | Import-only. Reads Continue development-data files (`chat.jsonl`, `editInteraction.jsonl`, ...) where each line has a `name` and nested `data`. |
| Cline / Roo Code | Task-history import | Import-only. Reads a task's `ui_messages.json` / `api_conversation_history.json`; one adapter covers Cline and its Roo Code fork (shared format). |
| Aider | Git log import | Uses Aider commit-message markers and file status from the source Git repository. |
| Generic | Tellur JSONL / CLI / daemon | For CI, custom tools, and local HTTP ingestion. |

## Integration Mechanisms

Tellur uses the strongest documented mechanism each surface exposes. Do not
model every editor as if it had Codex-style hooks.

| Mechanism | Used by | Strength | Implementation |
| --- | --- | --- | --- |
| User lifecycle hooks | Codex, Claude Code, Gemini CLI, Antigravity | Strongest local live capture when hook payloads include concrete file paths. | Global setup writes each tool's documented hook config with absolute `tellur hooks ingest --source <agent> --auto-init` commands; Gemini/Antigravity use `--json-response` because their hooks require JSON stdout. |
| Personal plugin / marketplace | Codex | Manual workflow discovery, not required per project. | Setup writes `~/.codex/plugins/tellur-provenance` and `~/.agents/plugins/marketplace.json`. |
| MCP server | Cursor, Antigravity, external agents | Tool access for status, explain, blame, verify, and policy checks. | Setup writes `~/.cursor/mcp.json` and Antigravity MCP configs pointing to the absolute `tellur mcp` command. |
| VS Code-compatible extension | VS Code, Cursor | Best available editor-level live capture where lifecycle hooks are not documented. | User settings enable `autoInit`, `autoWatch`, and `captureOnSave`; save capture routes through `hooks ingest` with source `vscode` or `cursor`. |
| Import adapters | Cursor, Codex, Copilot, Aider, Gemini CLI, Antigravity, Windsurf, JetBrains, Devin, Continue, Cline/Roo Code, Generic | Historical or metadata-based evidence. | `tellur import <adapter> <source>` normalizes external event streams while preserving source identity and timestamps. JSONL/array/envelope adapters share one tolerant parsing loop (`crates/adapters/src/import.rs`); each adapter only defines its event-type mapping. |
| Git/policy fallback | All editors | Enforcement at review/commit time. | `tellur policy check`, PR reports, Git notes, and future pre-commit/CI wiring catch gaps in editor APIs. |

## Adoption Adapter Roadmap

The 2026-Q2 adoption batch shipped as import adapters (see Current Adapters):
Windsurf/Cascade, JetBrains AI Assistant / Junie, Devin, Continue, and
Cline/Roo Code. They currently ship as import-only because none of these tools
expose a documented local lifecycle-hook surface comparable to Codex or Gemini
CLI; see Known Limits for what import-only evidence proves.

Next candidates, when they expose durable capture surfaces:

1. Live lifecycle-hook capture for Windsurf and JetBrains if/when those tools
   document a local hook API (today only Codex, Claude Code, Gemini CLI, and
   Antigravity do).
2. A Devin webhook/API ingestion path through the local daemon for real-time
   cloud-agent provenance instead of after-the-fact run export.

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
