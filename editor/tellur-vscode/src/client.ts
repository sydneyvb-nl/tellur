// Tellur CLI client — bridges VS Code to the tellur binary

import * as vscode from 'vscode';
import { execFile } from 'child_process';
import * as path from 'path';

export interface AttributionRange {
    range_id: string;
    start_line: number;
    end_line: number;
    origin: string;
    confidence: number;
    agent_id: string;
    model_id?: string;
    risk_level?: string;
    risk_tags: string[];
}

export interface FileAttribution {
    file_path: string;
    ranges: AttributionRange[];
}

export interface Session {
    id: string;
    started_at: string;
    ended_at?: string;
    agent_id: string;
    agent_name: string;
    model_name?: string;
    status: string;
    event_count: number;
}

export interface ExplainResult {
    file_path: string;
    line: number;
    origin: string;
    confidence: number;
    agent_id?: string;
    model_id?: string;
    session_id?: string;
    prompt_hash?: string;
    risk_level?: string;
}

export class TellurClient {
    private binaryPath: string;
    private watchProcess: ReturnType<typeof execFile> | null = null;
    private outputChannel: vscode.OutputChannel;

    constructor(binaryPath: string) {
        this.binaryPath = binaryPath;
        this.outputChannel = vscode.window.createOutputChannel('Tellur');
    }

    /** Execute a tellur CLI command */
    async exec(args: string[], cwd?: string): Promise<string> {
        const workDir = cwd || vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
        return new Promise((resolve, reject) => {
            execFile(this.binaryPath, args, { cwd: workDir, maxBuffer: 10 * 1024 * 1024 }, (err, stdout, stderr) => {
                if (err) {
                    this.outputChannel.appendLine(`ERROR: ${this.binaryPath} ${args.join(' ')}`);
                    this.outputChannel.appendLine(stderr || err.message);
                    reject(new Error(`tellur ${args[0]} failed: ${stderr || err.message}`));
                } else {
                    resolve(stdout);
                }
            });
        });
    }

    /** Initialize Tellur in the workspace */
    async init(): Promise<string> {
        return this.exec(['init']);
    }

    /** Explain who changed a specific line */
    async explain(filePath: string, line: number): Promise<ExplainResult | null> {
        try {
            const output = await this.exec(['explain', `${filePath}:${line}`, '--json']);
            return JSON.parse(output);
        } catch {
            return null;
        }
    }

    /** Get attribution for an entire file */
    async blame(filePath: string): Promise<FileAttribution | null> {
        try {
            const output = await this.exec(['blame', filePath, '--json']);
            return JSON.parse(output);
        } catch {
            return null;
        }
    }

    /** Generate a PR report */
    async prReport(base: string, head: string): Promise<string> {
        return this.exec(['pr-report', '--base', base, '--head', head]);
    }

    /** List sessions */
    async sessions(): Promise<Session[]> {
        try {
            const output = await this.exec(['sessions', '--json']);
            return JSON.parse(output);
        } catch {
            return [];
        }
    }

    /** Check policy */
    async policyCheck(): Promise<string> {
        return this.exec(['policy', 'check']);
    }

    /** Record structured event payload */
    async event(eventType: string, session: string, payload: Record<string, unknown>): Promise<string> {
        return this.exec([
            'event',
            '--event-type',
            eventType,
            '--session',
            session,
            '--payload-json',
            JSON.stringify(payload),
        ]);
    }

    /** Start watching for changes */
    startWatch(options?: { agentId?: string; agentName?: string; modelId?: string }): void {
        const workDir = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
        if (!workDir) return;
        if (this.watchProcess) return;

        const args = ['watch'];
        if (options?.agentId) {
            args.push('--agent-id', options.agentId);
        }
        if (options?.agentName) {
            args.push('--agent-name', options.agentName);
        }
        if (options?.modelId) {
            args.push('--model-id', options.modelId);
        }

        this.watchProcess = execFile(this.binaryPath, args, { cwd: workDir });
        this.watchProcess.stdout?.on('data', (data: Buffer) => {
            this.outputChannel.append(data.toString());
        });
        this.watchProcess.stderr?.on('data', (data: Buffer) => {
            this.outputChannel.append(data.toString());
        });
        this.watchProcess.on('exit', () => {
            this.watchProcess = null;
        });
    }

    /** Stop watching */
    stopWatch(): void {
        this.watchProcess?.kill();
        this.watchProcess = null;
    }

    /** Check if tellur is installed */
    async isInstalled(): Promise<boolean> {
        try {
            await this.exec(['--version']);
            return true;
        } catch {
            return false;
        }
    }
}
