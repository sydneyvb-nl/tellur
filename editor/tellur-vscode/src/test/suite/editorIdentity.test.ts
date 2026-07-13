import * as assert from 'assert';
import { configuredEditorIdentity, editorIdentity } from '../../editorIdentity';
import { suite, test } from './harness';

suite('Editor identity', () => {
    test('detects VS Code-compatible desktop hosts', () => {
        assert.deepStrictEqual(editorIdentity('Visual Studio Code'), {
            source: 'vscode',
            name: 'VS Code AI',
        });
        assert.deepStrictEqual(editorIdentity('Cursor'), {
            source: 'cursor',
            name: 'Cursor',
        });
        assert.deepStrictEqual(editorIdentity('Windsurf'), {
            source: 'windsurf',
            name: 'Windsurf / Cascade',
        });
    });

    test('keeps explicit setup overrides', () => {
        assert.deepStrictEqual(
            configuredEditorIdentity('Windsurf', 'team-agent', 'Team Agent'),
            { source: 'team-agent', name: 'Team Agent' },
        );
    });
});
