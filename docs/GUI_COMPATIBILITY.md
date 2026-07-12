# Developer GUI Compatibility

Last audited: 2026-07-12

This document records which GUI contracts Tellur relies on and what was verified
locally. It prevents the integrations from drifting into claims that the host
editors do not support.

## VS Code, Cursor, and Windsurf

Tellur uses the standard VS Code extension API for all three desktop editors.
VS Code documents multi-root changes through
[`onDidChangeWorkspaceFolders`](https://code.visualstudio.com/api/references/vscode-api),
remote CLI execution through workspace extension hosts in its
[Remote Extensions guide](https://code.visualstudio.com/api/advanced-topics/remote-extensions),
and executable-workspace gating through
[Workspace Trust](https://code.visualstudio.com/api/extension-guides/workspace-trust).

Tellur therefore:

- declares `extensionKind: ["workspace"]`, placing the extension and CLI next
  to local or remote workspace files;
- declares untrusted workspaces unsupported, so opening an unknown repository
  cannot automatically execute the configured CLI;
- starts and stops an independent `tellur watch` process for every workspace
  root, including roots added or removed after activation;
- resolves saves against their containing workspace root;
- detects `vscode`, `cursor`, or `windsurf` from `vscode.env.appName` when no
  explicit setup override exists.

Cursor's official MCP documentation confirms global `~/.cursor/mcp.json` and
the `mcpServers` shape used by `tellur setup cursor`:
[Cursor MCP](https://docs.cursor.com/context/model-context-protocol). Windsurf's
official Cascade documentation confirms
`~/.codeium/windsurf/mcp_config.json`:
[Windsurf MCP](https://docs.windsurf.com/windsurf/cascade/mcp).

Verified locally:

- TypeScript compilation;
- seven pure unit tests, including VS Code/Cursor/Windsurf identity selection;
- two tests inside a real VS Code 1.128 Extension Host;
- VSIX packaging.

Known limit: vscode.dev/github.dev-style virtual workspaces without a runnable
Node workspace host cannot execute the local-first Rust CLI. Remote SSH, WSL,
Dev Containers, and Codespaces need `tellur` installed in the remote workspace
environment.

## JetBrains IDEs

JetBrains recommends `BulkFileListener` on `VFS_CHANGES` for efficient VFS
observation and notes that the listener is application-wide, requiring project
filtering: [Virtual File System](https://plugins.jetbrains.com/docs/intellij/virtual-file-system.html).
Tellur uses that mechanism, filters each local file through `ProjectLocator`,
and moves CLI work off the write thread into a bounded application-service
queue.

The plugin depends only on `com.intellij.modules.platform`, the portable module
JetBrains documents for IntelliJ-family products:
[Plugin Compatibility](https://plugins.jetbrains.com/docs/intellij/plugin-compatibility.html).
Its descriptor supports build branches 241–253 (2024.1–2025.3), following the
official [build-number ranges](https://plugins.jetbrains.com/docs/intellij/build-number-ranges.html).

Verified locally with JDK 17:

- three Kotlin tests;
- `buildPlugin`, including `buildSearchableOptions`, which boots a headless
  2024.1 IDE with the plugin installed;
- packaged plugin ZIP generation.

The current repo-pinned IntelliJ Gradle Plugin 2.0.1 cannot resolve the renamed
2025.3 Community distribution for Plugin Verifier. Cross-version binary
verification must be added together with a deliberate Gradle/IntelliJ build
tooling upgrade; 2025.3 is declared compatible but was not boot-tested locally.
