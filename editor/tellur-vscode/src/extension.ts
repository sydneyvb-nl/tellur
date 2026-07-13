// Tellur VS Code Extension — AI Code Provenance

import * as vscode from 'vscode';
import { TellurClient } from './client';
import { AttributionProvider } from './providers/attribution';
import { SessionProvider } from './providers/sessions';
import { InlineDecorationManager } from './decorations';
import { registerCommands } from './commands';
import { resolveConfiguredVSCodeModel } from './vscodeModels';
import { resolveModelSelection } from './modelMetadata';
import { listVSCodeLanguageModels } from './vscodeModels';
import { configuredEditorIdentity } from './editorIdentity';

let client: TellurClient;
let sessionProvider: SessionProvider;
let attributionProvider: AttributionProvider;
let decorationManager: InlineDecorationManager;

export function activate(context: vscode.ExtensionContext) {
    console.log('Tellur extension activated');

    const config = vscode.workspace.getConfiguration('tellur');
    client = new TellurClient(config.get('tellurPath', 'tellur'));

    // Register tree data providers
    sessionProvider = new SessionProvider(client);
    attributionProvider = new AttributionProvider(client);

    vscode.window.registerTreeDataProvider('tellur.sessions', sessionProvider);
    vscode.window.registerTreeDataProvider('tellur.attributions', attributionProvider);

    // Inline decorations
    if (config.get('showInlineDecorations', true)) {
        decorationManager = new InlineDecorationManager(client);
        context.subscriptions.push(decorationManager);
    }

    // Commands
    registerCommands(context, client, sessionProvider, attributionProvider, decorationManager);

    if (config.get('captureOnSave', true)) {
        context.subscriptions.push(
            vscode.workspace.onDidSaveTextDocument(document => {
                captureSavedDocument(document);
            })
        );
    }

    // One CLI watcher per workspace folder. VS Code can add/remove roots without
    // restarting the extension host, so keep the processes in sync explicitly.
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
        startWorkspaceCapture(folder);
    }
    context.subscriptions.push(
        vscode.workspace.onDidChangeWorkspaceFolders(event => {
            for (const folder of event.removed) {
                client.stopWatch(folder.uri.fsPath);
            }
            for (const folder of event.added) {
                startWorkspaceCapture(folder);
            }
        })
    );

    // Status bar
    const statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusItem.text = '$(eye) Tellur';
    statusItem.command = 'tellur.sessions';
    statusItem.show();
    context.subscriptions.push(statusItem);
    updateStatusItem(statusItem);
}

export function deactivate() {
    client?.stopWatch();
}

async function startWorkspaceCapture(folder: vscode.WorkspaceFolder): Promise<void> {
    const config = vscode.workspace.getConfiguration('tellur', folder.uri);
    const identity = configuredEditorIdentity(
        vscode.env.appName,
        config.get('vscodeAgentId', ''),
        config.get('vscodeAgentName', ''),
    );
    if (config.get('autoInit', true)) {
        try {
            await client.ensureInitialized(folder.uri.fsPath);
        } catch (err: any) {
            console.warn(`Tellur auto-init failed for ${folder.name}: ${err?.message || err}`);
            return;
        }
    }
    if (!config.get('autoWatch', true)) {
        return;
    }
    client.startWatch(folder.uri.fsPath, {
        agentId: identity.source,
        agentName: identity.name,
        modelId: await resolveConfiguredVSCodeModel(),
    });
}

async function captureSavedDocument(document: vscode.TextDocument): Promise<void> {
    if (document.isUntitled) {
        return;
    }
    const workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
    if (!workspaceFolder) {
        return;
    }
    const config = vscode.workspace.getConfiguration('tellur', document.uri);
    const identity = configuredEditorIdentity(
        vscode.env.appName,
        config.get('vscodeAgentId', ''),
        config.get('vscodeAgentName', ''),
    );
    try {
        await client.ingestHook(identity.source, {
            event: 'PostToolUse',
            session_id: config.get('vscodePromptSessionId', '').trim() || identity.source,
            cwd: workspaceFolder.uri.fsPath,
            model: await resolveConfiguredVSCodeModel(),
            tool: {
                name: 'VSCodeSave',
                input: {
                    file_path: document.uri.fsPath,
                },
            },
        }, workspaceFolder.uri.fsPath);
    } catch (err: any) {
        console.warn(`Tellur save capture failed: ${err?.message || err}`);
    }
}

async function updateStatusItem(statusItem: vscode.StatusBarItem): Promise<void> {
    const config = vscode.workspace.getConfiguration('tellur');
    const selection = resolveModelSelection(
        await listVSCodeLanguageModels(),
        config.get('vscodeModelId', ''),
    );
    statusItem.tooltip = `Tellur\n${selection.message}`;
}
