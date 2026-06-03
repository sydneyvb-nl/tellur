# Tellur Provenance — JetBrains plugin

Live AI code provenance capture for IntelliJ-family IDEs (IntelliJ IDEA,
PyCharm, WebStorm, GoLand, RubyMine, CLion, Rider, PhpStorm, …).

JetBrains does not expose a documented local lifecycle-hook surface comparable
to Codex or Gemini CLI, and its MCP support is configured in-IDE rather than
through a stable global config file. This plugin is therefore the editor-level
live-capture surface for JetBrains, the equivalent of the Tellur VS Code
extension: it records files that change on disk — including edits made by the
**JetBrains AI Assistant** and the **Junie** agent — and routes them to the
local `tellur` CLI.

## How it works

The plugin subscribes to the platform `VFS_CHANGES` topic
([`BulkFileListener`](src/main/kotlin/dev/tellur/jetbrains/TellurVfsListener.kt)).
When a file's content is written or a file is created, it runs:

```bash
tellur hooks ingest --source jetbrains --auto-init
```

in the project base directory, piping a hook payload on stdin:

```json
{
  "hook_event_name": "PostToolUse",
  "tool_name": "jetbrains-ide",
  "session_id": "jetbrains-<uuid>",
  "cwd": "/path/to/project",
  "tool_input": { "file_path": "/path/to/project/src/Main.kt" }
}
```

This is the same hook contract the CLI accepts for every agent source, so all
gating lives in one place:

- Outside a Git repository the CLI no-ops.
- Inside a Git repository without `.tellur/`, `--auto-init` creates local
  storage with safe defaults.
- A repository can opt out by creating `.tellur/disable`.
- Only the working-tree changes for the concrete file path are captured.

Capture runs on a pooled thread and never blocks the IDE write path; if `tellur`
is not installed or not on `PATH`, capture is silently skipped.

## Settings

**Preferences → Tools → Tellur Provenance**:

- **Path to the tellur executable** — defaults to `tellur` (resolved on `PATH`).
  Set an absolute path if the CLI is not on the IDE's `PATH`.
- **Capture file changes on save** — toggles the listener on/off.

## Building

> **Note:** Building requires JDK 17 and downloads the IntelliJ Platform SDK via
> Gradle, so it needs network access and is not built as part of the Rust
> workspace CI.

```bash
cd editor/tellur-jetbrains
gradle wrapper            # one-time: generate ./gradlew
./gradlew buildPlugin     # produces build/distributions/tellur-jetbrains-<version>.zip
./gradlew runIde          # launch a sandbox IDE with the plugin for manual testing
```

Install the resulting zip via **Preferences → Plugins → ⚙ → Install Plugin from
Disk…**.

## Relationship to the CLI

This plugin is a thin capture client. All provenance logic — attribution,
redaction, policy checks, the tamper-evident hash chain, and export — lives in
the `tellur` CLI and core. See the repository
[`README.md`](../../README.md) and [`docs/ADAPTERS.md`](../../docs/ADAPTERS.md).
