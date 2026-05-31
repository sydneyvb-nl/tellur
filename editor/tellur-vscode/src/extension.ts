// Tellur VS Code Extension — AI Code Provenance

import * as vscode from 'vscode';
import { TellurClient } from './client';
import { AttributionProvider } from './providers/attribution';
import { SessionProvider } from './providers/sessions';
import { InlineDecorationManager } from './decorations';
import { registerCommands } from './commands';

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
        client.startWatch();
    }

    // Status bar
    const statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusItem.text = '$(eye) Tellur';
    statusItem.command = 'tellur.sessions';
    statusItem.show();
    context.subscriptions.push(statusItem);
}

export function deactivate() {
    client?.stopWatch();
}
