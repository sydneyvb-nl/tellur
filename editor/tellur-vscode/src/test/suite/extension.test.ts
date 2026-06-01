import * as assert from 'assert';
import * as vscode from 'vscode';
import { suite, test } from './harness';

suite('Tellur extension', () => {
    test('registers VS Code AI model commands', async () => {
        await vscode.extensions.getExtension('tellur.tellur-vscode')?.activate();
        const commands = await vscode.commands.getCommands(true);

        assert.ok(commands.includes('tellur.selectVSCodeModel'));
        assert.ok(commands.includes('tellur.diagnoseVSCodeModels'));
        assert.ok(commands.includes('tellur.recordPrompt'));
    });

    test('opens diagnostics document without requiring a live BYOK provider', async () => {
        await vscode.commands.executeCommand('tellur.diagnoseVSCodeModels');

        const editor = vscode.window.activeTextEditor;
        assert.ok(editor);
        assert.strictEqual(editor.document.languageId, 'markdown');
        assert.match(editor.document.getText(), /Tellur VS Code AI Diagnostics/);
        assert.match(editor.document.getText(), /Capture limits/);
    });
});
