// Command registration

import * as vscode from 'vscode';
import { TraceGitClient } from './client';
import { SessionProvider } from './providers/sessions';
import { AttributionProvider } from './providers/attribution';
import { InlineDecorationManager } from './decorations';

export function registerCommands(
    context: vscode.ExtensionContext,
    client: TraceGitClient,
    sessionProvider: SessionProvider,
    attributionProvider: AttributionProvider,
    decorationManager: InlineDecorationManager | undefined,
) {
    // Init
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.init', async () => {
            try {
                const output = await client.init();
                vscode.window.showInformationMessage(`TraceGit initialized: ${output.trim()}`);
            } catch (e: any) {
                vscode.window.showErrorMessage(`Init failed: ${e.message}`);
            }
        })
    );

    // Explain current line
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.explain', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) return;

            const filePath = editor.document.uri.fsPath;
            const line = editor.selection.active.line + 1;

            try {
                const result = await client.explain(filePath, line);
                if (result) {
                    const originIcon = result.origin === 'ai' ? '🤖 AI' : result.origin === 'human' ? '👤 Human' : '❓ Unknown';
                    const msg = `${originIcon} · ${result.agent_id || 'unknown'} · confidence: ${((result.confidence || 0) * 100).toFixed(0)}%`;
                    vscode.window.showInformationMessage(msg, { modal: false }, 'Copy Details').then(choice => {
                        if (choice === 'Copy Details') {
                            vscode.env.clipboard.writeText(JSON.stringify(result, null, 2));
                        }
                    });
                } else {
                    vscode.window.showInformationMessage('No attribution data for this line');
                }
            } catch (e: any) {
                vscode.window.showErrorMessage(`Explain failed: ${e.message}`);
            }
        })
    );

    // Blame (file attribution)
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.blame', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) return;

            const filePath = editor.document.uri.fsPath;
            try {
                const attr = await client.blame(filePath);
                attributionProvider.setAttribution(attr);
            } catch (e: any) {
                vscode.window.showErrorMessage(`Blame failed: ${e.message}`);
            }
        })
    );

    // PR Report
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.prReport', async () => {
            const base = await vscode.window.showInputBox({ prompt: 'Base ref', value: 'main' });
            if (!base) return;
            const head = await vscode.window.showInputBox({ prompt: 'Head ref', value: 'HEAD' });
            if (!head) return;

            try {
                const report = await client.prReport(base, head);
                const doc = await vscode.workspace.openTextDocument({
                    content: report,
                    language: 'markdown',
                });
                vscode.window.showTextDocument(doc);
            } catch (e: any) {
                vscode.window.showErrorMessage(`PR report failed: ${e.message}`);
            }
        })
    );

    // Sessions
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.sessions', () => {
            sessionProvider.refresh();
            vscode.commands.executeCommand('tracegit.sessions.focus');
        })
    );

    // Policy check
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.policyCheck', async () => {
            try {
                const output = await client.policyCheck();
                const doc = await vscode.workspace.openTextDocument({
                    content: output,
                    language: 'plaintext',
                });
                vscode.window.showTextDocument(doc);
            } catch (e: any) {
                vscode.window.showErrorMessage(`Policy check failed: ${e.message}`);
            }
        })
    );

    // Start watch
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.startWatch', () => {
            client.startWatch();
            vscode.window.showInformationMessage('TraceGit: Watching started');
        })
    );

    // Stop watch
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.stopWatch', () => {
            client.stopWatch();
            vscode.window.showInformationMessage('TraceGit: Watching stopped');
        })
    );

    // Goto line (internal)
    context.subscriptions.push(
        vscode.commands.registerCommand('tracegit.gotoLine', async (filePath: string, line: number) => {
            const doc = await vscode.workspace.openTextDocument(filePath);
            const editor = await vscode.window.showTextDocument(doc);
            const position = new vscode.Position(line - 1, 0);
            editor.selection = new vscode.Selection(position, position);
            editor.revealRange(new vscode.Range(position, position));
        })
    );
}
