// Attribution tree view provider

import * as vscode from 'vscode';
import { TellurClient, FileAttribution, AttributionRange } from '../client';

export class AttributionProvider implements vscode.TreeDataProvider<AttributionItem> {
    private _onDidChangeTreeData = new vscode.EventEmitter<AttributionItem | undefined | null>();
    readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

    private attribution: FileAttribution | null = null;

    constructor(private client: TellurClient) {}

    setAttribution(attr: FileAttribution | null): void {
        this.attribution = attr;
        this._onDidChangeTreeData.fire(undefined);
    }

    getTreeItem(element: AttributionItem): vscode.TreeItem {
        return element;
    }

    getChildren(element?: AttributionItem): Thenable<AttributionItem[]> {
        if (!this.attribution) {
            return Promise.resolve([
                new AttributionItem('No attribution data', '', vscode.TreeItemCollapsibleState.None)
            ]);
        }

        if (element) {
            return Promise.resolve([]);
        }

        return Promise.resolve(
            this.attribution.ranges.map(r => {
                const originIcon = r.origin === 'ai' ? '🤖' : r.origin === 'human' ? '👤' : '❓';
                const label = `${originIcon} L${r.start_line}-${r.end_line}`;
                const item = new AttributionItem(
                    label,
                    `${r.agent_id} · ${(r.confidence * 100).toFixed(0)}%`,
                    vscode.TreeItemCollapsibleState.None
                );
                item.tooltip = [
                    `Range: ${r.start_line}-${r.end_line}`,
                    `Origin: ${r.origin}`,
                    `Confidence: ${(r.confidence * 100).toFixed(1)}%`,
                    `Agent: ${r.agent_id}`,
                    r.model_id ? `Model: ${r.model_id}` : '',
                    r.risk_level ? `Risk: ${r.risk_level}` : '',
                    ...r.risk_tags.map(t => `Tag: ${t}`),
                ].filter(Boolean).join('\n');

                // Click to navigate to line
                item.command = {
                    command: 'tellur.gotoLine',
                    title: 'Go to Line',
                    arguments: [this.attribution!.file_path, r.start_line],
                };

                return item;
            })
        );
    }
}

class AttributionItem extends vscode.TreeItem {
    constructor(label: string, description: string, collapsible: vscode.TreeItemCollapsibleState) {
        super(label, collapsible);
        this.description = description;
    }
}
