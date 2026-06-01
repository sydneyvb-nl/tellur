export interface LanguageModelMetadata {
    id: string;
    vendor: string;
    name: string;
    family?: string;
    version?: string;
}

export interface ModelSelection {
    modelId?: string;
    status: 'configured' | 'auto' | 'none' | 'ambiguous';
    message: string;
}

export function modelKey(model: LanguageModelMetadata): string {
    return `${model.vendor}:${model.id}`;
}

export function resolveModelSelection(
    models: LanguageModelMetadata[],
    configuredModelId: string | undefined,
): ModelSelection {
    const configured = configuredModelId?.trim();
    if (configured) {
        return {
            modelId: configured,
            status: 'configured',
            message: `Using configured VS Code AI model ${configured}`,
        };
    }

    if (models.length === 1) {
        const selected = modelKey(models[0]);
        return {
            modelId: selected,
            status: 'auto',
            message: `Using the only available VS Code AI model ${selected}`,
        };
    }

    if (models.length > 1) {
        return {
            status: 'ambiguous',
            message: 'Multiple VS Code AI models are available; choose one with Tellur: Select VS Code AI Model.',
        };
    }

    return {
        status: 'none',
        message: 'No VS Code AI models are available to Tellur.',
    };
}

export function formatModelDiagnostics(
    models: LanguageModelMetadata[],
    configuredModelId: string | undefined,
    lmApiAvailable: boolean,
): string {
    const selection = resolveModelSelection(models, configuredModelId);
    const lines = [
        '# Tellur VS Code AI Diagnostics',
        '',
        `Language Model API: ${lmApiAvailable ? 'available' : 'not available'}`,
        `Selection: ${selection.message}`,
        '',
        'Available models:',
    ];

    if (models.length === 0) {
        lines.push('- none reported by VS Code');
    } else {
        for (const model of models) {
            const details = [model.family, model.version].filter(Boolean).join(', ');
            lines.push(`- ${model.name} (${modelKey(model)}${details ? `; ${details}` : ''})`);
        }
    }

    lines.push(
        '',
        'Capture limits:',
        '- Tellur can attach selected VS Code/BYOK model metadata to watch sessions.',
        '- VS Code does not expose a public API for this extension to intercept arbitrary Copilot/BYOK chat prompts from other chat participants.',
        '- Use Tellur: Record AI Prompt when you need prompt provenance for a VS Code chat you are about to run.',
        '- BYOK model availability is controlled by VS Code, the configured provider, and any GitHub Copilot organization policy.',
    );

    return `${lines.join('\n')}\n`;
}
