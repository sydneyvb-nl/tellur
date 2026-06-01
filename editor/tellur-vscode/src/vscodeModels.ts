import * as vscode from 'vscode';
import {
    LanguageModelMetadata,
    modelKey,
    resolveModelSelection,
} from './modelMetadata';

export async function listVSCodeLanguageModels(): Promise<LanguageModelMetadata[]> {
    const lm = (vscode as any).lm;
    if (!lm?.selectChatModels) {
        return [];
    }

    try {
        const models = await lm.selectChatModels();
        return models.map((model: any) => ({
            id: model.id,
            vendor: model.vendor,
            name: model.name,
            family: model.family,
            version: model.version,
        }));
    } catch {
        return [];
    }
}

export function hasLanguageModelApi(): boolean {
    return Boolean((vscode as any).lm?.selectChatModels);
}

export async function resolveConfiguredVSCodeModel(): Promise<string | undefined> {
    const config = vscode.workspace.getConfiguration('tellur');
    const models = await listVSCodeLanguageModels();
    return resolveModelSelection(models, config.get('vscodeModelId', '')).modelId;
}

export { modelKey };
