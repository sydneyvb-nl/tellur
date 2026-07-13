export interface EditorIdentity {
    source: 'vscode' | 'cursor' | 'windsurf';
    name: 'VS Code AI' | 'Cursor' | 'Windsurf / Cascade';
}

/** Resolve the VS Code-compatible host without relying on product-specific APIs. */
export function editorIdentity(appName: string): EditorIdentity {
    const normalized = appName.toLowerCase();
    if (normalized.includes('windsurf')) {
        return { source: 'windsurf', name: 'Windsurf / Cascade' };
    }
    if (normalized.includes('cursor')) {
        return { source: 'cursor', name: 'Cursor' };
    }
    return { source: 'vscode', name: 'VS Code AI' };
}

export function configuredEditorIdentity(
    appName: string,
    configuredSource: string,
    configuredName: string,
): { source: string; name: string } {
    const detected = editorIdentity(appName);
    return {
        source: configuredSource.trim() || detected.source,
        name: configuredName.trim() || detected.name,
    };
}
