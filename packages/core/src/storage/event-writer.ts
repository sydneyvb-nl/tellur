/**
 * JSONL event writer — append-only, tamper-evident event log
 */

import type { TraceEvent } from "../schemas/types.js";
import { hashEvent } from "../schemas/validation.js";
import * as fs from "node:fs";
import * as path from "node:path";

export class EventWriter {
  private stream: fs.WriteStream | null = null;
  private lastHash: string | null = null;

  constructor(private readonly logDir: string) {}

  async open(): Promise<void> {
    await fs.promises.mkdir(this.logDir, { recursive: true });
    const logFile = path.join(this.logDir, `events-${new Date().toISOString().split("T")[0]}.jsonl`);
    this.stream = fs.createWriteStream(logFile, { flags: "a" });

    // Read last hash from existing log for chain continuity
    try {
      const files = await fs.promises.readdir(this.logDir);
      const jsonlFiles = files.filter((f) => f.endsWith(".jsonl")).sort();
      if (jsonlFiles.length > 0) {
        const lastFile = path.join(this.logDir, jsonlFiles[jsonlFiles.length - 1]!);
        const content = await fs.promises.readFile(lastFile, "utf-8");
        const lines = content.trim().split("\n").filter(Boolean);
        if (lines.length > 0) {
          const lastEvent = JSON.parse(lines[lines.length - 1]!) as TraceEvent;
          this.lastHash = lastEvent.eventHash ?? null;
        }
      }
    } catch {
      // No existing log — start fresh
    }
  }

  async write(event: Omit<TraceEvent, "eventHash" | "prevHash">): Promise<TraceEvent> {
    if (!this.stream) throw new Error("EventWriter not open. Call open() first.");

    const eventWithPrev = { ...event, prevHash: this.lastHash } as TraceEvent;
    const eventHash = await hashEvent(eventWithPrev);
    const fullEvent: TraceEvent = { ...eventWithPrev, eventHash };

    const line = JSON.stringify(fullEvent) + "\n";
    this.stream.write(line);
    this.lastHash = eventHash;

    return fullEvent;
  }

  async close(): Promise<void> {
    return new Promise((resolve) => {
      if (this.stream) {
        this.stream.end(() => {
          this.stream = null;
          resolve();
        });
      } else {
        resolve();
      }
    });
  }
}

/**
 * Read events from a JSONL log directory
 */
export async function* readEvents(logDir: string): AsyncGenerator<TraceEvent> {
  const files = (await fs.promises.readdir(logDir))
    .filter((f) => f.endsWith(".jsonl"))
    .sort();

  for (const file of files) {
    const filePath = path.join(logDir, file);
    const content = await fs.promises.readFile(filePath, "utf-8");
    for (const line of content.split("\n")) {
      if (!line.trim()) continue;
      yield JSON.parse(line) as TraceEvent;
    }
  }
}
