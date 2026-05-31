#!/usr/bin/env node
/**
 * TraceGit CLI
 *
 * AI Code Provenance from the terminal.
 */

import { Command } from "commander";
import * as fs from "node:fs";
import * as path from "node:path";

const VERSION = "0.0.1";

const program = new Command();

program
  .name("tracegit")
  .description("AI Code Provenance — line-level attribution, session replay, PR risk reports")
  .version(VERSION);

// ─── init ────────────────────────────────────────────────────────────────────

program
  .command("init")
  .description("Initialize TraceGit in the current repository")
  .option("--profile <profile>", "Setup profile: default | team | oss-maintainer", "default")
  .action(async (options: { profile: string }) => {
    const gitRoot = findGitRoot();
    if (!gitRoot) {
      console.error("Error: not inside a Git repository");
      process.exit(1);
    }

    const tracegitDir = path.join(gitRoot, ".tracegit");
    const configPath = path.join(tracegitDir, "config.yml");
    const policiesDir = path.join(tracegitDir, "policies");
    const tracesDir = path.join(tracegitDir, "traces");

    if (fs.existsSync(configPath)) {
      console.log("TraceGit already initialized. Run `tracegit doctor` to check setup.");
      return;
    }

    fs.mkdirSync(tracegitDir, { recursive: true });
    fs.mkdirSync(policiesDir, { recursive: true });
    fs.mkdirSync(tracesDir, { recursive: true });

    const config = generateConfig(options.profile);
    fs.writeFileSync(configPath, config);

    const defaultPolicy = generateDefaultPolicy();
    fs.writeFileSync(path.join(policiesDir, "default.yml"), defaultPolicy);

    // Add .tracegit/traces/ to .gitignore
    const gitignorePath = path.join(gitRoot, ".gitignore");
    const gitignoreEntry = "\n# TraceGit event traces (local data)\n.tracegit/traces/\n.tracegit/index/\n";
    if (fs.existsSync(gitignorePath)) {
      const existing = fs.readFileSync(gitignorePath, "utf-8");
      if (!existing.includes(".tracegit/traces/")) {
        fs.appendFileSync(gitignorePath, gitignoreEntry);
      }
    } else {
      fs.writeFileSync(gitignorePath, gitignoreEntry.trim() + "\n");
    }

    console.log(`✓ TraceGit initialized (profile: ${options.profile})`);
    console.log(`  Config: ${configPath}`);
    console.log(`  Policies: ${policiesDir}`);
    console.log(`  Traces: ${tracesDir}`);
    console.log("");
    console.log("Next: run `tracegit doctor` to verify setup");
  });

// ─── doctor ──────────────────────────────────────────────────────────────────

program
  .command("doctor")
  .description("Check TraceGit setup and detect AI tools")
  .action(async () => {
    const gitRoot = findGitRoot();
    if (!gitRoot) {
      console.error("✗ Not inside a Git repository");
      process.exit(1);
    }
    console.log("TraceGit Doctor");
    console.log("═══════════════");
    console.log("");

    // Check config
    const configPath = path.join(gitRoot, ".tracegit/config.yml");
    if (fs.existsSync(configPath)) {
      console.log("✓ Config found");
    } else {
      console.log("✗ Config not found — run `tracegit init` first");
    }

    // Check policies
    const policiesDir = path.join(gitRoot, ".tracegit/policies");
    if (fs.existsSync(policiesDir)) {
      const policies = fs.readdirSync(policiesDir).filter((f) => f.endsWith(".yml"));
      console.log(`✓ ${policies.length} polic${policies.length === 1 ? "y" : "ies"} found`);
      for (const p of policies) {
        console.log(`  - ${p}`);
      }
    } else {
      console.log("⚠ No policies directory");
    }

    // Check traces
    const tracesDir = path.join(gitRoot, ".tracegit/traces");
    if (fs.existsSync(tracesDir)) {
      const sessions = countSessions(tracesDir);
      console.log(`✓ Traces directory (${sessions} session${sessions === 1 ? "" : "s"})`);
    }

    // Detect AI tools
    console.log("");
    console.log("AI Tool Detection:");
    const tools = detectAITools(gitRoot);
    if (tools.length === 0) {
      console.log("  No AI coding tools detected");
    }
    for (const tool of tools) {
      console.log(`  ✓ ${tool.name}${tool.version ? ` v${tool.version}` : ""} (${tool.source})`);
    }

    console.log("");
    console.log("Setup looks good. Run `tracegit watch` to start capturing.");
  });

// ─── status ──────────────────────────────────────────────────────────────────

program
  .command("status")
  .description("Show current TraceGit status")
  .action(async () => {
    const gitRoot = findGitRoot();
    if (!gitRoot) {
      console.error("Error: not inside a Git repository");
      process.exit(1);
    }

    const tracesDir = path.join(gitRoot, ".tracegit/traces");
    if (!fs.existsSync(tracesDir)) {
      console.log("No traces yet. Run `tracegit watch` to start capturing.");
      return;
    }

    const sessions = countSessions(tracesDir);
    const events = countEvents(tracesDir);

    console.log(`Sessions: ${sessions}`);
    console.log(`Events: ${events}`);
  });

// ─── Helpers ─────────────────────────────────────────────────────────────────

function findGitRoot(): string | null {
  let dir = process.cwd();
  while (dir !== "/") {
    if (fs.existsSync(path.join(dir, ".git"))) {
      return dir;
    }
    dir = path.dirname(dir);
  }
  return null;
}

interface DetectedTool {
  name: string;
  version?: string;
  source: string;
}

function detectAITools(gitRoot: string): DetectedTool[] {
  const tools: DetectedTool[] = [];

  // Claude Code
  const claudeSettings = path.join(gitRoot, ".claude/settings.json");
  if (fs.existsSync(claudeSettings)) {
    tools.push({ name: "Claude Code", source: ".claude/settings.json" });
  }

  // Cursor
  const cursorDir = path.join(gitRoot, ".cursor");
  if (fs.existsSync(cursorDir)) {
    tools.push({ name: "Cursor", source: ".cursor/" });
  }

  // Aider
  const aiderConf = path.join(gitRoot, ".aider.conf.yml");
  if (fs.existsSync(aiderConf)) {
    tools.push({ name: "Aider", source: ".aider.conf.yml" });
  }

  // Continue
  const continueDir = path.join(gitRoot, ".continue");
  if (fs.existsSync(continueDir)) {
    tools.push({ name: "Continue", source: ".continue/" });
  }

  // Copilot
  const copilotConf = path.join(gitRoot, ".github/copilot");
  if (fs.existsSync(copilotConf)) {
    tools.push({ name: "GitHub Copilot", source: ".github/copilot" });
  }

  // OpenClaw
  const openclawDir = path.join(gitRoot, ".openclaw");
  if (fs.existsSync(openclawDir)) {
    tools.push({ name: "OpenClaw", source: ".openclaw/" });
  }

  return tools;
}

function countSessions(tracesDir: string): number {
  let count = 0;
  const walk = (dir: string) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        walk(path.join(dir, entry.name));
      } else if (entry.name.endsWith(".jsonl")) {
        count++;
      }
    }
  };
  walk(tracesDir);
  return count;
}

function countEvents(tracesDir: string): number {
  let count = 0;
  const walk = (dir: string) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(fullPath);
      } else if (entry.name.endsWith(".jsonl")) {
        const content = fs.readFileSync(fullPath, "utf-8");
        count += content.split("\n").filter((l) => l.trim()).length;
      }
    }
  };
  walk(tracesDir);
  return count;
}

function generateConfig(profile: string): string {
  return `# TraceGit Configuration
# Profile: ${profile}

version: 1

storage:
  mode: local
  traces_dir: traces
  index_type: sqlite

redaction:
  mode: automatic
  hash_prompts: true
  store_prompt_excerpt: false
  redact_patterns:
    - "(?i)api[_-]?key\\\\s*=\\\\s*.+"
    - "(?i)password\\\\s*=\\\\s*.+"
    - "(?i)token\\\\s*=\\\\s*.+"

retention:
  keep_days: 90
  keep_release_related: true
  delete_prompts_after_days: 30

attribution:
  confidence_threshold: 0.7
  range_fingerprint_window: 5
  semantic_anchors: true
`;
}

function generateDefaultPolicy(): string {
  return `# TraceGit Default Policy
version: 1

sensitive_paths:
  - path: "src/auth/**"
    tags: ["auth", "security-sensitive"]
    require_human_review: true
    require_tests: true

  - path: "**/.env*"
    tags: ["secrets"]
    block_ai_read: true

  - path: "infra/**"
    tags: ["infrastructure"]
    block_ai_automerge: true

rules: []
  # Add custom rules here. Example:
  # - id: require-tests-for-ai-code
  #   description: "AI code changes require test evidence"
  #   when:
  #     attribution.origin: ai
  #     changed_lines.greater_than: 20
  #   action: warn
  #   require:
  #     tests_run: true
`;
}

program.parse();
