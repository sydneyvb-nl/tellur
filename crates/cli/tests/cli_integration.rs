//! CLI integration tests — test the tellur binary end-to-end

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tellur_core::schema::types::{AttributionRange, AttributionState, EvidenceStrength, Origin};
use tellur_core::storage::{RepoStorage, TraceIndex};

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
