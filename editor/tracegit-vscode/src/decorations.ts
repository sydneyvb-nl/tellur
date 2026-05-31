// Inline decoration manager — color-code AI vs human lines

import * as vscode from 'vscode';
import { TraceGitClient, FileAttribution } from './client';

export class InlineDecorationManager implements vscode.Disposable {
    private aiDecorationType: vscode.TextEditorDecorationType;
    private humanDecorationType: vscode.TextEditorDecorationType;
    private unknownDecorationType: vscode.TextEditorDecorationType;
    private disposables: vscode.Disposable[] = [];

    constructor(private client: TraceGitClient) {
        const config = vscode.workspace.getConfiguration('tracegit');

        this.aiDecorationType = vscode.window.createTextEditorDecorationType({
            backgroundColor: config.get('aiColor', 'rgba(138, 43, 226, 0.15)'),
            after: {
                contentText: ' 🤖',
                color: 'rgba(138, 43, 226, 0.5)',
                margin: '0 0 0 1em',
            },
            isWholeLine: true,
        });

        this.humanDecorationType = vscode.window.createTextEditorDecorationType({
            backgroundColor: config.get('humanColor', 'rgba(34, 139, 34, 0.15)'),
            isWholeLine: true,
        });

        this.unknownDecorationType = vscode.window.createTextEditorDecorationType({
            isWholeLine: true,
        });

        // Update decorations on editor change
        vscode.window.onDidChangeActiveTextEditor(
            editor => this.updateDecorations(editor),
            this,
            this.disposables
        );

        // Initial
        if (vscode.window.activeTextEditor) {
            this.updateDecorations(vscode.window.activeTextEditor);
        }
    }

    private async updateDecorations(editor: vscode.TextEditor | undefined): Promise<void> {
        if (!editor || editor.document.uri.scheme !== 'file') return;

        const filePath = editor.document.uri.fsPath;
        const attr = await this.client.blame(filePath);
        if (!attr) return;

        const aiRanges: vscode.Range[] = [];
        const humanRanges: vscode.Range[] = [];

        for (const range of attr.ranges) {
            const vscodeRange = new vscode.Range(
                range.start_line - 1, 0,
                range.end_line - 1, 0
            );

            switch (range.origin) {
                case 'ai':
                case 'mixed':
                    aiRanges.push(vscodeRange);
                    break;
                case 'human':
                    humanRanges.push(vscodeRange);
                    break;
            }
        }

        editor.setDecorations(this.aiDecorationType, aiRanges);
        editor.setDecorations(this.humanDecorationType, humanRanges);
    }

    dispose(): void {
        this.aiDecorationType.dispose();
        this.humanDecorationType.dispose();
        this.unknownDecorationType.dispose();
        this.disposables.forEach(d => d.dispose());
    }
}
