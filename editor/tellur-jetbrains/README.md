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

Capture runs through a single bounded background queue owned by an IntelliJ
application service and never blocks the IDE write path. Duplicate captures for
the same file/repository are coalesced while one is queued or running; if a new
save arrives during an active capture, one follow-up capture is queued so the
final file state is still recorded. If `tellur` is not installed or not on
`PATH`, capture is skipped and logged at debug level; non-zero exits and
timeouts are logged as warnings so repeated provenance loss is visible during
troubleshooting.

## Settings

**Preferences → Tools → Tellur Provenance**:

- **Path to the tellur executable** — defaults to `tellur` (resolved on `PATH`).
  Set an absolute path if the CLI is not on the IDE's `PATH`.
- **Capture file changes on save** — toggles the listener on/off.

## Building

The Gradle wrapper is committed (pinned to Gradle 8.9), so no global Gradle
install is needed. Building requires **JDK 17** and downloads the IntelliJ
Platform SDK on first run, so it needs network access. It is built with Gradle,
not the Rust `cargo` CI.

```bash
cd editor/tellur-jetbrains
./gradlew buildPlugin     # produces build/distributions/tellur-jetbrains-<version>.zip
./gradlew runIde          # launch a sandbox IDE with the plugin for manual testing
```

If your default JDK is newer than 17, point Gradle at a JDK 17, e.g.:

```bash
JAVA_HOME=/path/to/jdk-17 ./gradlew buildPlugin
```

Install the resulting zip via **Preferences → Plugins → ⚙ → Install Plugin from
Disk…**.

The plugin declares compatibility with IntelliJ Platform 2024.1 through 2025.3
(`sinceBuild=241`, `untilBuild=253.*`). It compiles and runs its tests against
the oldest supported 2024.1 SDK. The pinned IntelliJ Gradle Plugin 2.0.1 cannot
resolve the renamed 2025.3 Community distribution for a local Plugin Verifier
run; upgrading that build tooling and adding cross-version verification is a
tracked follow-up, not silently treated as verified coverage.

> **Verified:** `./gradlew test` and `./gradlew buildPlugin` succeed on JDK 17 and produce a loadable
> plugin (the build's `buildSearchableOptions` step boots a headless IDE with the
> plugin installed, which exercises `plugin.xml` and the listener/service
> classes).

## Relationship to the CLI

This plugin is a thin capture client. All provenance logic — attribution,
redaction, policy checks, the tamper-evident hash chain, and export — lives in
the `tellur` CLI and core. See the repository
[`README.md`](../../README.md) and [`docs/ADAPTERS.md`](../../docs/ADAPTERS.md).
