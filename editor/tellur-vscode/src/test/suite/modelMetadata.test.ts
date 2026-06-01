import * as assert from 'assert';
import {
    formatModelDiagnostics,
    modelKey,
    resolveModelSelection,
} from '../../modelMetadata';
import { suite, test } from './harness';

suite('model metadata', () => {
    test('builds stable vendor scoped model ids', () => {
        assert.strictEqual(modelKey({ vendor: 'openai', id: 'gpt-5', name: 'GPT-5' }), 'openai:gpt-5');
    });

    test('uses explicit configured model before discovered models', () => {
        const selection = resolveModelSelection(
            [{ vendor: 'copilot', id: 'gpt-4.1', name: 'GPT 4.1' }],
            'openai:gpt-5',
        );

        assert.strictEqual(selection.status, 'configured');
        assert.strictEqual(selection.modelId, 'openai:gpt-5');
    });

    test('auto-selects a single discovered model', () => {
        const selection = resolveModelSelection(
            [{ vendor: 'openai', id: 'gpt-5', name: 'GPT-5' }],
            '',
        );

        assert.strictEqual(selection.status, 'auto');
        assert.strictEqual(selection.modelId, 'openai:gpt-5');
    });

    test('requires explicit selection when multiple models are available', () => {
        const selection = resolveModelSelection(
            [
                { vendor: 'openai', id: 'gpt-5', name: 'GPT-5' },
                { vendor: 'ollama', id: 'qwen3-coder', name: 'Qwen3 Coder' },
            ],
            undefined,
        );

        assert.strictEqual(selection.status, 'ambiguous');
        assert.strictEqual(selection.modelId, undefined);
    });

    test('diagnostics disclose model visibility and capture limits', () => {
        const diagnostics = formatModelDiagnostics(
            [{ vendor: 'openai', id: 'gpt-5', name: 'GPT-5', family: 'gpt' }],
            'openai:gpt-5',
            true,
        );

        assert.match(diagnostics, /Language Model API: available/);
        assert.match(diagnostics, /openai:gpt-5/);
        assert.match(diagnostics, /does not expose a public API/);
        assert.match(diagnostics, /Record AI Prompt/);
    });
});
