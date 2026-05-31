// Session tree view provider

import * as vscode from 'vscode';
import { TraceGitClient, Session } from '../client';

export class SessionProvider implements vscode.TreeDataProvider<SessionItem> {
    private _onDidChangeTreeData = new vscode.EventEmitter<SessionItem | undefined | null>();
    readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

    private sessions: Session[] = [];

    constructor(private client: TraceGitClient) {}

    refresh(): void {
        this.client.sessions().then(sessions => {
            this.sessions = sessions;
            this._onDidChangeTreeData.fire(undefined);
        });
    }

    getTreeItem(element: SessionItem): vscode.TreeItem {
        return element;
    }

    getChildren(element?: SessionItem): Thenable<SessionItem[]> {
        if (element) {
            return Promise.resolve([]);
        }

        if (this.sessions.length === 0) {
            return this.client.sessions().then(sessions => {
                this.sessions = sessions;
                return sessions.map(s => new SessionItem(s));
            });
        }

        return Promise.resolve(this.sessions.map(s => new SessionItem(s)));
    }
}

class SessionItem extends vscode.TreeItem {
    constructor(session: Session) {
        const label = `${session.agent_name || session.agent_id} — ${session.started_at.slice(0, 19)}`;
        super(label, vscode.TreeItemCollapsibleState.None);

        const origin = session.event_count > 0 ? '●' : '○';
        this.description = `${origin} ${session.event_count} events${session.model_name ? ` · ${session.model_name}` : ''}`;
        this.tooltip = `Session: ${session.id}\nAgent: ${session.agent_name || session.agent_id}\nStarted: ${session.started_at}${session.ended_at ? `\nEnded: ${session.ended_at}` : ''}`;

        this.contextValue = 'session';
    }
}
