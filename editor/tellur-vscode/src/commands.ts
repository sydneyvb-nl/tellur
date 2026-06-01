// Command registration

import * as vscode from 'vscode';
import { createHash } from 'crypto';
import { TellurClient } from './client';
import { SessionProvider } from './providers/sessions';
import { AttributionProvider } from './providers/attribution';
import { InlineDecorationManager } from './decorations';
import { formatModelDiagnostics } from './modelMetadata';
import {
    hasLanguageModelApi,
    listVSCodeLanguageModels,
    modelKey,
    resolveConfiguredVSCodeModel,
} from './vscodeModels';

export function registerCommands(
    context: vscode.ExtensionContext,
    client: TellurClient,
    sessionProvider: SessionProvider,
    attributionProvider: AttributionProvider,
    decorationManager: InlineDecorationManager | undefined,
) {
    // Init
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.init', async () => {
            try {
                const output = await client.init();
                vscode.window.showInformationMessage(`Tellur initialized: ${output.trim()}`);
            } catch (e: any) {
                vscode.window.showErrorMessage(`Init failed: ${e.message}`);
            }
        })
    );

    // Explain current line
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.explain', async () => {
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
        vscode.commands.registerCommand('tellur.blame', async () => {
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
        vscode.commands.registerCommand('tellur.prReport', async () => {
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
        vscode.commands.registerCommand('tellur.sessions', () => {
            sessionProvider.refresh();
            vscode.commands.executeCommand('tellur.sessions.focus');
        })
    );

    // Policy check
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.policyCheck', async () => {
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
        vscode.commands.registerCommand('tellur.startWatch', async () => {
            const config = vscode.workspace.getConfiguration('tellur');
            const modelId = await resolveConfiguredVSCodeModel();
            client.startWatch({
                agentId: config.get('vscodeAgentId', 'vscode-ai'),
                agentName: config.get('vscodeAgentName', 'VS Code AI'),
                modelId,
            });
            vscode.window.showInformationMessage(
                modelId ? `Tellur: Watching started (${modelId})` : 'Tellur: Watching started'
            );
        })
    );

    // Stop watch
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.stopWatch', () => {
            client.stopWatch();
            vscode.window.showInformationMessage('Tellur: Watching stopped');
        })
    );

    // Select VS Code/Copilot/BYOK model metadata for watch sessions
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.selectVSCodeModel', async () => {
            const models = await listVSCodeLanguageModels();
            if (models.length === 0) {
                vscode.window.showWarningMessage(
                    'No VS Code language models are available. Configure BYOK with "Chat: Manage Language Models" first.'
                );
                return;
            }

            const picked = await vscode.window.showQuickPick(
                models.map(model => ({
                    label: model.name,
                    description: modelKey(model),
                    detail: [model.family, model.version].filter(Boolean).join(' · '),
                    model,
                })),
                { placeHolder: 'Select the VS Code AI model Tellur should attach to watch sessions' }
            );
            if (!picked) return;

            const value = modelKey(picked.model);
            await vscode.workspace
                .getConfiguration('tellur')
                .update('vscodeModelId', value, vscode.ConfigurationTarget.Workspace);
            vscode.window.showInformationMessage(`Tellur: VS Code AI model set to ${value}`);
        })
    );

    // Diagnose VS Code/Copilot/BYOK model visibility and platform limits
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.diagnoseVSCodeModels', async () => {
            const config = vscode.workspace.getConfiguration('tellur');
            const models = await listVSCodeLanguageModels();
            const diagnostics = formatModelDiagnostics(
                models,
                config.get('vscodeModelId', ''),
                hasLanguageModelApi(),
            );
            const doc = await vscode.workspace.openTextDocument({
                content: diagnostics,
                language: 'markdown',
            });
            await vscode.window.showTextDocument(doc);
        })
    );

    // Explicit prompt hash recording. VS Code does not expose arbitrary chat
    // prompts from other participants, so Tellur records only a hash here.
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.recordPrompt', async () => {
            const prompt = await vscode.window.showInputBox({
                prompt: 'Prompt to hash and record',
                placeHolder: 'Paste the prompt text. Tellur stores only a SHA-256 hash.',
                ignoreFocusOut: true,
            });
            if (!prompt) return;

            const config = vscode.workspace.getConfiguration('tellur');
            const modelId = await resolveConfiguredVSCodeModel();
            const promptHash = `sha256:${createHash('sha256').update(prompt).digest('hex')}`;
            const session = config.get('vscodePromptSessionId', 'vscode-ai');

            try {
                await client.event('prompt.submitted', session, {
                    prompt_hash: promptHash,
                    model_id: modelId,
                    tool: 'vscode',
                    source: 'manual-vscode-command',
                });
                vscode.window.showInformationMessage(`Tellur: Recorded prompt ${promptHash.slice(0, 19)}...`);
            } catch (e: any) {
                vscode.window.showErrorMessage(`Record prompt failed: ${e.message}`);
            }
        })
    );

    // Goto line (internal)
    context.subscriptions.push(
        vscode.commands.registerCommand('tellur.gotoLine', async (filePath: string, line: number) => {
            const doc = await vscode.workspace.openTextDocument(filePath);
            const editor = await vscode.window.showTextDocument(doc);
            const position = new vscode.Position(line - 1, 0);
            editor.selection = new vscode.Selection(position, position);
            editor.revealRange(new vscode.Range(position, position));
        })
    );
}
