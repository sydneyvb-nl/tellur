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

    // Auto-init and auto-watch
    if (config.get('autoInit', true)) {
        client.ensureInitialized().catch(err => {
            console.warn(`Tellur auto-init failed: ${err?.message || err}`);
        });
    }
    if (config.get('autoWatch', true)) {
        startVSCodeWatch(config);
    }

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

async function startVSCodeWatch(config: vscode.WorkspaceConfiguration): Promise<void> {
    if (config.get('autoInit', true)) {
        await client.ensureInitialized();
    }
    client.startWatch({
        agentId: config.get('vscodeAgentId', defaultEditorSource()),
        agentName: config.get('vscodeAgentName', defaultEditorName()),
        modelId: await resolveConfiguredVSCodeModel(),
    });
}

async function captureSavedDocument(document: vscode.TextDocument): Promise<void> {
    if (document.uri.scheme !== 'file' || document.isUntitled) {
        return;
    }
    const workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
    if (!workspaceFolder) {
        return;
    }
    const config = vscode.workspace.getConfiguration('tellur', document.uri);
    const source = config.get('vscodeAgentId', defaultEditorSource());
    try {
        await client.ingestHook(source, {
            event: 'PostToolUse',
            session_id: config.get('vscodePromptSessionId', source),
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

function defaultEditorSource(): string {
    return vscode.env.appName.toLowerCase().includes('cursor') ? 'cursor' : 'vscode';
}

function defaultEditorName(): string {
    return vscode.env.appName.toLowerCase().includes('cursor') ? 'Cursor' : 'VS Code AI';
}

async function updateStatusItem(statusItem: vscode.StatusBarItem): Promise<void> {
    const config = vscode.workspace.getConfiguration('tellur');
    const selection = resolveModelSelection(
        await listVSCodeLanguageModels(),
        config.get('vscodeModelId', ''),
    );
    statusItem.tooltip = `Tellur\n${selection.message}`;
}
