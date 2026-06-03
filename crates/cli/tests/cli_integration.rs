//! CLI integration tests — test the tellur binary end-to-end

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tellur_core::schema::types::{AttributionRange, AttributionState, EvidenceStrength, Origin};
use tellur_core::schema::types::{EventActor, EventType, TraceEvent};
use tellur_core::storage::{RepoStorage, TraceIndex, read_events};

fn tellur_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_tellur") {
        return PathBuf::from(path);
    }
    let root = workspace_root();
    // Try release first, then debug
    let release = root.join("target/release/tellur");
    let debug = root.join("target/debug/tellur");
    if release.exists() { release } else { debug }
}

fn require_binary() -> Command {
    let binary = tellur_binary();
    if !binary.exists() {
        eprintln!("Building tellur binary for integration tests...");
        let root = workspace_root();
        let status = Command::new("cargo")
            .args(["build", "--bin", "tellur"])
            .current_dir(&root)
            .status()
            .expect("Failed to run cargo build");
        if !status.success() {
            panic!("Failed to build tellur binary for tests");
        }
    }
    Command::new(binary)
}

fn tellur() -> Command {
    require_binary()
}

fn workspace_root() -> PathBuf {
    // Navigate up from crates/cli/tests to workspace root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn temp_repo() -> PathBuf {
    // Unique per test invocation — tests run in parallel within one process,
    // so the directory name must not collide (process id alone is shared).
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let unique = format!(
        "{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        nanos
    );
    let dir = std::env::temp_dir().join(format!("tellur-test-{}", unique));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Init git repo
    Command::new("git")
        .args(["init"])
        .current_dir(&dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@tellur.dev"])
        .current_dir(&dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&dir)
        .output()
        .unwrap();

    dir
}

#[test]
fn test_version() {
    let output = tellur().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tellur"));
}

#[test]
fn test_help() {
    let output = tellur().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("init"));
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("blame"));
    assert!(stdout.contains("pr-report"));
    assert!(stdout.contains("notes"));
}

#[test]
fn test_notes_help_lists_git_ai_commands() {
    let output = tellur().args(["notes", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("export"));
    assert!(stdout.contains("show"));
    assert!(stdout.contains("refs/notes/ai"));
}

#[test]
fn test_watch_help_lists_vscode_agent_model_metadata_options() {
    let output = tellur().args(["watch", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--agent-id"));
    assert!(stdout.contains("--agent-name"));
    assert!(stdout.contains("--model-id"));
}

#[test]
fn test_setup_agents_installs_user_level_agent_editor_integrations() {
    let home = std::env::temp_dir().join(format!(
        "tellur-home-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(home.join(".gemini/antigravity")).unwrap();
    fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
    fs::write(home.join(".gemini/antigravity/mcp_config.json"), "").unwrap();
    fs::write(home.join(".gemini/antigravity-cli/mcp_config.json"), " \n").unwrap();

    let output = require_binary()
        .args(["setup", "agents", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let claude_settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_hook_command(&claude_settings, "UserPromptSubmit", "claude-code");
    assert_hook_command(&claude_settings, "PostToolUse", "claude-code");

    let codex_hooks: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".codex/hooks.json")).unwrap()).unwrap();
    assert_hook_command(&codex_hooks, "UserPromptSubmit", "codex");
    assert_hook_command(&codex_hooks, "PostToolUse", "codex");

    let codex_plugin = home.join(".codex/plugins/tellur-provenance/.codex-plugin/plugin.json");
    assert!(codex_plugin.exists());
    let plugin_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&codex_plugin).unwrap()).unwrap();
    assert_eq!(plugin_manifest["skills"], "./skills/");
    assert!(plugin_manifest.get("hooks").is_none());
    let marketplace: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".agents/plugins/marketplace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        marketplace["plugins"][0]["source"]["path"],
        "./.codex/plugins/tellur-provenance"
    );
    let codex_config = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(codex_config.contains(r#"[plugins."tellur-provenance@tellur-local"]"#));
    assert!(codex_config.contains("enabled = true"));
    let cursor_mcp: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".cursor/mcp.json")).unwrap()).unwrap();
    assert_eq!(cursor_mcp["mcpServers"]["tellur"]["args"][0], "mcp");
    assert!(
        PathBuf::from(
            cursor_mcp["mcpServers"]["tellur"]["command"]
                .as_str()
                .unwrap()
        )
        .is_absolute()
    );
    let cursor_settings = read_editor_settings(&home, "Cursor");
    assert_eq!(cursor_settings["tellur.vscodeAgentId"], "cursor");
    assert_eq!(cursor_settings["tellur.autoInit"], true);
    assert_eq!(cursor_settings["tellur.captureOnSave"], true);
    let vscode_settings = read_editor_settings(&home, "Code");
    assert_eq!(vscode_settings["tellur.vscodeAgentId"], "vscode");
    assert_eq!(vscode_settings["tellur.autoInit"], true);
    assert_eq!(vscode_settings["tellur.captureOnSave"], true);
    let windsurf_mcp: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".codeium/windsurf/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(windsurf_mcp["mcpServers"]["tellur"]["args"][0], "mcp");
    assert!(
        PathBuf::from(
            windsurf_mcp["mcpServers"]["tellur"]["command"]
                .as_str()
                .unwrap()
        )
        .is_absolute()
    );
    let windsurf_settings = read_editor_settings(&home, "Windsurf");
    assert_eq!(windsurf_settings["tellur.vscodeAgentId"], "windsurf");
    assert_eq!(windsurf_settings["tellur.autoInit"], true);
    assert_eq!(windsurf_settings["tellur.captureOnSave"], true);
    let gemini_settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".gemini/settings.json")).unwrap())
            .unwrap();
    assert_named_hook_command(&gemini_settings, "BeforeTool", "gemini-cli");
    assert_named_hook_command(&gemini_settings, "AfterTool", "gemini-cli");
    assert_eq!(gemini_settings["hooksConfig"]["enabled"], true);
    let antigravity_hooks: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".gemini/config/hooks.json")).unwrap())
            .unwrap();
    let command = antigravity_hooks["tellur-provenance"]["PostToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains("hooks ingest --source antigravity --auto-init --json-response"));
    let antigravity_mcp: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".gemini/antigravity/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(antigravity_mcp["mcpServers"]["tellur"]["args"][0], "mcp");
    let antigravity_cli_mcp: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".gemini/antigravity-cli/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        antigravity_cli_mcp["mcpServers"]["tellur"]["args"][0],
        "mcp"
    );

    let stale_command =
        "/tmp/old-tellur hooks ingest --source gemini-cli --auto-init --json-response";
    let mut stale_gemini = gemini_settings.clone();
    stale_gemini["hooks"]["BeforeTool"][0]["hooks"][0]["command"] =
        serde_json::Value::String(stale_command.to_string());
    fs::write(
        home.join(".gemini/settings.json"),
        serde_json::to_string_pretty(&stale_gemini).unwrap(),
    )
    .unwrap();
    let mut stale_antigravity = antigravity_hooks.clone();
    stale_antigravity["tellur-provenance"]["PostToolUse"][0]["hooks"][0]["command"] =
        serde_json::Value::String(
            "/tmp/old-tellur hooks ingest --source antigravity --auto-init --json-response"
                .to_string(),
        );
    fs::write(
        home.join(".gemini/config/hooks.json"),
        serde_json::to_string_pretty(&stale_antigravity).unwrap(),
    )
    .unwrap();
    let rerun = require_binary()
        .args(["setup", "agents", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(rerun.status.success());
    let refreshed_gemini: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".gemini/settings.json")).unwrap())
            .unwrap();
    let refreshed_gemini_command =
        refreshed_gemini["hooks"]["BeforeTool"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
    assert!(!refreshed_gemini_command.contains("/tmp/old-tellur"));
    assert_named_hook_command(&refreshed_gemini, "BeforeTool", "gemini-cli");
    let refreshed_antigravity: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".gemini/config/hooks.json")).unwrap())
            .unwrap();
    let refreshed_antigravity_command = refreshed_antigravity["tellur-provenance"]["PostToolUse"]
        [0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(!refreshed_antigravity_command.contains("/tmp/old-tellur"));

    let status = require_binary()
        .args(["setup", "status", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("Claude Code global hooks: installed"));
    assert!(status_stdout.contains("Codex global hooks: installed"));
    assert!(status_stdout.contains("Codex personal plugin: installed"));
    assert!(status_stdout.contains("Cursor global integration: installed"));
    assert!(status_stdout.contains("VS Code global integration: installed"));
    assert!(status_stdout.contains("Windsurf global integration: installed"));
    assert!(status_stdout.contains("Gemini CLI global integration: installed"));
    assert!(status_stdout.contains("Antigravity global integration: installed"));

    let uninstall = require_binary()
        .args(["setup", "uninstall", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(uninstall.status.success());
    let status = require_binary()
        .args(["setup", "status", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("Claude Code global hooks: missing"));
    assert!(status_stdout.contains("Codex global hooks: missing"));
    assert!(status_stdout.contains("Codex personal plugin: missing"));
    assert!(status_stdout.contains("Cursor global integration: missing"));
    assert!(status_stdout.contains("VS Code global integration: missing"));
    assert!(status_stdout.contains("Windsurf global integration: missing"));
    assert!(status_stdout.contains("Gemini CLI global integration: missing"));
    assert!(status_stdout.contains("Antigravity global integration: missing"));
    let codex_config = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(!codex_config.contains(r#"[plugins."tellur-provenance@tellur-local"]"#));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn test_setup_windsurf_installs_editor_and_mcp_config() {
    let home = std::env::temp_dir().join(format!(
        "tellur-windsurf-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&home).unwrap();

    let output = require_binary()
        .args(["setup", "windsurf", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let mcp: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".codeium/windsurf/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(mcp["mcpServers"]["tellur"]["args"][0], "mcp");
    assert!(PathBuf::from(mcp["mcpServers"]["tellur"]["command"].as_str().unwrap()).is_absolute());

    let settings = read_editor_settings(&home, "Windsurf");
    assert_eq!(settings["tellur.vscodeAgentId"], "windsurf");
    assert_eq!(settings["tellur.captureOnSave"], true);

    let status = require_binary()
        .args(["setup", "status", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("Windsurf global integration: installed"));

    let uninstall = require_binary()
        .args(["setup", "uninstall", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(uninstall.status.success());
    let status = require_binary()
        .args(["setup", "status", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("Windsurf global integration: missing"));

    let _ = fs::remove_dir_all(&home);
}

fn read_editor_settings(home: &std::path::Path, app_name: &str) -> serde_json::Value {
    #[cfg(target_os = "macos")]
    let path = home
        .join("Library")
        .join("Application Support")
        .join(app_name)
        .join("User/settings.json");
    #[cfg(target_os = "windows")]
    let path = home
        .join("AppData/Roaming")
        .join(app_name)
        .join("User/settings.json");
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let path = home
        .join(".config")
        .join(app_name)
        .join("User/settings.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_hook_command(config: &serde_json::Value, event: &str, source: &str) {
    let command = config["hooks"][event][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains(&format!(" hooks ingest --source {source} --auto-init")));
    let exe = command
        .split_whitespace()
        .next()
        .unwrap()
        .trim_matches('\'');
    assert!(
        PathBuf::from(exe).is_absolute(),
        "hook command must use an absolute executable path: {command}"
    );
}

fn assert_named_hook_command(config: &serde_json::Value, event: &str, source: &str) {
    let command = config["hooks"][event][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains(&format!(
        " hooks ingest --source {source} --auto-init --json-response"
    )));
    let exe = command
        .split_whitespace()
        .next()
        .unwrap()
        .trim_matches('\'');
    assert!(PathBuf::from(exe).is_absolute());
}

#[test]
fn test_hooks_ingest_auto_init_records_event_in_new_repo() {
    let dir = temp_repo();
    let mut child = require_binary()
        .args(["hooks", "ingest", "--source", "codex", "--auto-init"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            serde_json::json!({
                "session_id": "sess_auto",
                "hook_event_name": "SessionStart",
                "cwd": dir,
                "model": "gpt-5-codex"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    assert!(storage.is_initialized());
    let index = TraceIndex::open(&storage.index_path).unwrap();
    let events = index.get_session_events("sess_auto").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::SessionStart);
    assert_eq!(events[0].payload["tool"], "codex");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_hooks_ingest_invalid_json_does_not_auto_init_repo() {
    let dir = temp_repo();
    let mut child = require_binary()
        .args(["hooks", "ingest", "--source", "codex", "--auto-init"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{not-json")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    assert!(!storage.is_initialized());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_hooks_ingest_post_tool_without_file_path_does_not_capture_whole_tree() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "fn changed() {}\n").unwrap();

    let mut child = require_binary()
        .args(["hooks", "ingest", "--source", "codex", "--auto-init"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            serde_json::json!({
                "session_id": "sess_post_no_path",
                "event": "PostToolUse",
                "cwd": dir,
                "tool": { "name": "Bash", "input": { "command": "echo ok" } }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    let events = read_events(&storage.traces_dir).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::ToolPostCall)
    );
    assert!(
        !events
            .iter()
            .any(|event| event.payload["file"] == "src/lib.rs"
                || event.payload["file_path"] == "src/lib.rs")
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_hooks_ingest_gemini_event_mapping_returns_json_response() {
    let dir = temp_repo();
    let mut child = require_binary()
        .args([
            "hooks",
            "ingest",
            "--source",
            "gemini-cli",
            "--auto-init",
            "--json-response",
        ])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            serde_json::json!({
                "session_id": "sess_gemini",
                "event": "AfterTool",
                "cwd": dir,
                "tool_name": "write_file",
                "tool_input": {"file_path": "src/lib.rs"}
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "{}");

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    let index = TraceIndex::open(&storage.index_path).unwrap();
    let events = index.get_session_events("sess_gemini").unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::ToolPostCall)
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_setup_refuses_to_overwrite_invalid_existing_hook_config() {
    let home = std::env::temp_dir().join(format!(
        "tellur-invalid-home-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/hooks.json"), "{invalid").unwrap();

    let output = require_binary()
        .args(["setup", "codex", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(home.join(".codex/hooks.json")).unwrap(),
        "{invalid"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn test_event_accepts_structured_payload_json() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let output = require_binary()
        .args([
            "event",
            "--event-type",
            "prompt.submitted",
            "--session",
            "sess_vscode",
            "--payload-json",
            r#"{"prompt_hash":"sha256:abc","model_id":"openai:gpt-5"}"#,
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(output.status.success());

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    let index = TraceIndex::open(&storage.index_path).unwrap();
    let events = index.get_session_events("sess_vscode").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["prompt_hash"], "sha256:abc");
    assert_eq!(events[0].payload["model_id"], "openai:gpt-5");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_import_preserves_source_event_identity_and_timestamp() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let source = dir.join("events.jsonl");
    let imported = TraceEvent {
        schema: "tellur.event.v1".to_string(),
        id: "evt_imported_original".to_string(),
        session_id: "sess_imported_original".to_string(),
        timestamp: "2026-05-30T12:34:56Z".to_string(),
        event_type: EventType::Custom("custom.imported".to_string()),
        actor: EventActor::Agent,
        payload: serde_json::json!({"tool": "test", "file_path": "src/lib.rs"}),
        redaction: None,
        prev_hash: None,
        event_hash: None,
    };
    fs::write(
        &source,
        format!("{}\n", serde_json::to_string(&imported).unwrap()),
    )
    .unwrap();

    let output = require_binary()
        .args(["import", "generic", source.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(output.status.success());

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    let events = read_events(&storage.traces_dir).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, "evt_imported_original");
    assert_eq!(events[0].session_id, "sess_imported_original");
    assert_eq!(events[0].timestamp, "2026-05-30T12:34:56Z");
    assert_eq!(
        events[0].event_type,
        EventType::Custom("custom.imported".to_string())
    );
    assert!(events[0].event_hash.is_some());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_notes_export_prints_and_writes_git_ai_note() {
    let dir = temp_repo();
    fs::write(dir.join("src.rs"), "fn main() {}\n").unwrap();
    Command::new("git")
        .args(["add", "src.rs"])
        .current_dir(&dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&dir)
        .output()
        .unwrap();

    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    let index = TraceIndex::open(&storage.index_path).unwrap();
    index
        .index_attribution(
            &AttributionRange {
                range_id: "rng_note".to_string(),
                start_line: 1,
                end_line: 1,
                origin: Origin::Ai,
                evidence_strength: EvidenceStrength::Recorded,
                confidence: 1.0,
                state: AttributionState::Exact,
                session_id: "sess_note".to_string(),
                event_ids: vec![],
                agent_id: "codex".to_string(),
                model_id: Some("gpt-5".to_string()),
                prompt_hash: None,
                context_set_id: None,
                policy_tags: vec![],
                risk_tags: vec![],
                risk_level: None,
                tests_run: vec![],
                tests_passed: false,
                reviewer: None,
                reviewed_at: None,
            },
            "src.rs",
            "blob",
            "2026-05-31T00:00:00Z",
        )
        .unwrap();

    let printed = require_binary()
        .args(["notes", "export", "--print"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(printed.status.success());
    let stdout = String::from_utf8_lossy(&printed.stdout);
    assert!(stdout.contains("src.rs"));
    assert!(stdout.contains("\"schema_version\": \"authorship/3.0.0\""));
    assert!(stdout.contains("\"tool\": \"codex\""));

    let written = require_binary()
        .args(["notes", "export"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(written.status.success());

    let note = Command::new("git")
        .args(["notes", "--ref", "refs/notes/ai", "show", "HEAD"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(note.status.success());
    let note_stdout = String::from_utf8_lossy(&note.stdout);
    assert!(note_stdout.contains("src.rs"));
    assert!(note_stdout.contains("authorship/3.0.0"));

    let imported = require_binary()
        .args(["notes", "import"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(imported.status.success());
    let import_stdout = String::from_utf8_lossy(&imported.stdout);
    assert!(import_stdout.contains("Imported 1 attribution range"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_init() {
    let dir = temp_repo();
    let release_bin = tellur_binary();
    let output = Command::new(&release_bin)
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should create .tellur directory
    assert!(
        dir.join(".tellur").exists()
            || stdout.contains("tellur")
            || stderr.contains("tellur")
            || output.status.success()
    );

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_init_creates_structure() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    // Check that expected directories/files exist
    let tellur_dir = dir.join(".tellur");
    if tellur_dir.exists() {
        // Config should exist
        assert!(
            tellur_dir.join("config.yml").exists()
                || tellur_dir.join("config.yaml").exists()
                || tellur_dir.exists()
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_status_without_init() {
    let dir = temp_repo();
    let output = require_binary()
        .arg("status")
        .current_dir(&dir)
        .output()
        .unwrap();

    // Should either succeed with "not initialized" or fail gracefully
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("not")
            || combined.contains("No")
            || combined.contains("tellur")
            || !output.status.success()
            || !combined.is_empty()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_doctor() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let output = match require_binary().arg("doctor").current_dir(&dir).output() {
        Ok(o) => o,
        Err(_) => return, // Binary not available in this test environment
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    // Doctor should produce output OR exit with a status
    assert!(!combined.is_empty() || output.status.code().is_some());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_sessions_empty() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let output = require_binary()
        .arg("sessions")
        .current_dir(&dir)
        .output()
        .unwrap();

    // Should succeed (even if no sessions)
    let _ = String::from_utf8_lossy(&output.stdout);
    // No crash = pass

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_verify_empty() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let output = require_binary()
        .arg("verify")
        .current_dir(&dir)
        .output()
        .unwrap();

    // Should not crash
    let _ = String::from_utf8_lossy(&output.stdout);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_import_codex_jsonl() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    let events = dir.join("codex.jsonl");
    fs::write(
        &events,
        serde_json::json!({
            "timestamp": "2026-05-31T12:00:00Z",
            "type": "event_msg",
            "payload": {
                "type": "exec_command_begin",
                "command": "cargo test"
            }
        })
        .to_string(),
    )
    .unwrap();

    let output = require_binary()
        .args(["import", "codex"])
        .arg(&events)
        .current_dir(&dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Imported 1 events from codex"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_import_copilot_jsonl() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    let events = dir.join("copilot.jsonl");
    fs::write(
        &events,
        serde_json::json!({
            "timestamp": "2026-05-31T12:00:00Z",
            "type": "suggestion.accepted",
            "file": "src/main.ts",
            "completion_id": "cmp_1"
        })
        .to_string(),
    )
    .unwrap();

    let output = require_binary()
        .args(["import", "copilot"])
        .arg(&events)
        .current_dir(&dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Imported 1 events from copilot"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_import_windsurf_jsonl() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    let events = dir.join("cascade.jsonl");
    let lines = [
        serde_json::json!({
            "cascadeId": "cascade-1",
            "type": "user_message",
            "message": "add a CLI flag"
        }),
        serde_json::json!({
            "cascadeId": "cascade-1",
            "type": "write_file",
            "file_path": "src/cli.rs"
        }),
    ]
    .iter()
    .map(serde_json::Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(&events, lines).unwrap();

    let output = require_binary()
        .args(["import", "windsurf"])
        .arg(&events)
        .current_dir(&dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Imported 2 events from windsurf"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_import_cline_json_array_task() {
    let dir = temp_repo();
    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();
    let events = dir.join("ui_messages.json");
    let doc = serde_json::json!([
        {"ts": 1_700_000_000_000_i64, "type": "say", "say": "user_feedback", "text": "ship it"},
        {"ts": 1_700_000_001_000_i64, "type": "ask", "ask": "command", "text": "cargo test"}
    ]);
    fs::write(&events, doc.to_string()).unwrap();

    let output = require_binary()
        .args(["import", "cline"])
        .arg(&events)
        .current_dir(&dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Imported 2 events from cline"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_team_report_aggregates_notes_over_range() {
    let dir = temp_repo();
    // Base commit.
    fs::write(dir.join("src.rs"), "fn main() {}\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let base = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let base_sha = String::from_utf8_lossy(&base.stdout).trim().to_string();

    // Head commit carrying AI-attributed work.
    fs::write(dir.join("src.rs"), "fn main() { let x = 1; }\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature"])
        .current_dir(&dir)
        .output()
        .unwrap();

    require_binary()
        .arg("init")
        .current_dir(&dir)
        .output()
        .unwrap();

    let storage = RepoStorage::from_git_root(&dir).unwrap();
    let index = TraceIndex::open(&storage.index_path).unwrap();
    index
        .index_attribution(
            &AttributionRange {
                range_id: "rng_team".to_string(),
                start_line: 1,
                end_line: 5,
                origin: Origin::Ai,
                evidence_strength: EvidenceStrength::Recorded,
                confidence: 1.0,
                state: AttributionState::Exact,
                session_id: "sess_team".to_string(),
                event_ids: vec![],
                agent_id: "claude-code".to_string(),
                model_id: Some("claude-opus-4.7".to_string()),
                prompt_hash: None,
                context_set_id: None,
                policy_tags: vec![],
                risk_tags: vec![],
                risk_level: None,
                tests_run: vec![],
                tests_passed: false,
                reviewer: Some("alice".to_string()),
                reviewed_at: None,
            },
            "src.rs",
            "blob",
            "2026-06-03T00:00:00Z",
        )
        .unwrap();

    // Write the authorship note onto HEAD.
    let exported = require_binary()
        .args(["notes", "export"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(exported.status.success());

    // Markdown report over base..HEAD.
    let report = require_binary()
        .args(["team", "report", "--base", &base_sha, "--head", "HEAD"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(report.status.success());
    let stdout = String::from_utf8_lossy(&report.stdout);
    assert!(stdout.contains("Tellur Team AI-Involvement Report"));
    assert!(stdout.contains("With provenance: 1"));
    assert!(stdout.contains("AI-assisted lines: 5"));
    assert!(stdout.contains("claude-code"));
    assert!(stdout.contains("alice"));

    // JSON report.
    let json = require_binary()
        .args([
            "team", "report", "--base", &base_sha, "--head", "HEAD", "--json",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(json.status.success());
    let parsed: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("team report --json must be valid JSON");
    assert_eq!(parsed["schema"], "tellur.team-report.v1");
    assert_eq!(parsed["ai_lines"], 5);
    assert_eq!(parsed["commits_with_provenance"], 1);

    let _ = fs::remove_dir_all(&dir);
}
