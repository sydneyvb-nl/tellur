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

    // Auto-watch
    if (config.get('autoWatch', false)) {
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
    client.startWatch({
        agentId: config.get('vscodeAgentId', 'vscode-ai'),
        agentName: config.get('vscodeAgentName', 'VS Code AI'),
        modelId: await resolveConfiguredVSCodeModel(),
    });
}

async function updateStatusItem(statusItem: vscode.StatusBarItem): Promise<void> {
    const config = vscode.workspace.getConfiguration('tellur');
    const selection = resolveModelSelection(
        await listVSCodeLanguageModels(),
        config.get('vscodeModelId', ''),
    );
    statusItem.tooltip = `Tellur\n${selection.message}`;
}
