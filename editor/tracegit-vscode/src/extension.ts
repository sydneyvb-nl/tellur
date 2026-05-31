// TraceGit VS Code Extension — AI Code Provenance

import * as vscode from 'vscode';
import { TraceGitClient } from './client';
import { AttributionProvider } from './providers/attribution';
import { SessionProvider } from './providers/sessions';
import { InlineDecorationManager } from './decorations';
import { registerCommands } from './commands';

let client: TraceGitClient;
let sessionProvider: SessionProvider;
let attributionProvider: AttributionProvider;
let decorationManager: InlineDecorationManager;

export function activate(context: vscode.ExtensionContext) {
    console.log('TraceGit extension activated');

    const config = vscode.workspace.getConfiguration('tracegit');
    client = new TraceGitClient(config.get('tracegitPath', 'tracegit'));

    // Register tree data providers
    sessionProvider = new SessionProvider(client);
    attributionProvider = new AttributionProvider(client);

    vscode.window.registerTreeDataProvider('tracegit.sessions', sessionProvider);
    vscode.window.registerTreeDataProvider('tracegit.attributions', attributionProvider);

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
    statusItem.text = '$(eye) TraceGit';
    statusItem.command = 'tracegit.sessions';
    statusItem.show();
    context.subscriptions.push(statusItem);
}

export function deactivate() {
    client?.stopWatch();
}
