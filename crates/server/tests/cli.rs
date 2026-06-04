//! Process-level tests for the `tellur-server` admin CLI.

use std::path::PathBuf;
use std::process::Command;

use tellur_server::storage::{SqliteStore, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn server_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_tellur-server") {
        return PathBuf::from(path);
    }
    let exe = format!("tellur-server{}", std::env::consts::EXE_SUFFIX);
    workspace_root().join("target/debug").join(exe)
}

fn require_server_binary() -> Command {
    let binary = server_binary();
    if !binary.exists() {
        let status = Command::new("cargo")
            .args(["build", "--bin", "tellur-server"])
            .current_dir(workspace_root())
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "failed to build tellur-server");
    }
    Command::new(binary)
}

fn temp_db() -> PathBuf {
    let unique = format!(
        "tellur-server-cli-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::env::temp_dir().join(unique)
}

fn parse_org_id(stdout: &str) -> String {
    stdout
        .split("id=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .expect("create-org output should include id=")
        .to_string()
}

fn parse_member_id(stdout: &str) -> String {
    stdout
        .split("Created member id=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .expect("create-token output should include member id")
        .to_string()
}

#[test]
fn help_lists_admin_commands() {
    let output = require_server_binary()
        .args(["admin", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("create-org"));
    assert!(stdout.contains("create-token"));
    assert!(stdout.contains("set-policy"));
}

#[test]
fn create_token_requires_explicit_role() {
    let output = require_server_binary()
        .args(["admin", "create-token", "--org", "org_test"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--role"), "{stderr}");
}

#[test]
fn create_token_records_admin_cli_actor_not_new_member() {
    let db = temp_db();
    let create_org = require_server_binary()
        .args(["admin", "create-org", "--name", "CLI Org"])
        .env("TELLUR_SERVER_DB", &db)
        .output()
        .unwrap();
    assert!(create_org.status.success());
    let org_id = parse_org_id(&String::from_utf8_lossy(&create_org.stdout));

    let create_token = require_server_binary()
        .args([
            "admin",
            "create-token",
            "--org",
            &org_id,
            "--role",
            "viewer",
            "--name",
            "viewer-token",
        ])
        .env("TELLUR_SERVER_DB", &db)
        .output()
        .unwrap();
    assert!(create_token.status.success());
    let stdout = String::from_utf8_lossy(&create_token.stdout);
    let member_id = parse_member_id(&stdout);
    assert!(stdout.contains("role=viewer"));

    let store = SqliteStore::open(&db).unwrap();
    let audit = store.export_audit(&org_id).unwrap();
    let token_create = audit
        .iter()
        .find(|entry| entry.action == "token.create")
        .expect("token.create audit entry");
    assert_ne!(
        token_create.actor_member_id.as_deref(),
        Some(member_id.as_str())
    );
    assert!(token_create.actor_member_id.is_none());
    assert!(token_create.detail.contains("via=admin-cli"));

    let _ = std::fs::remove_file(db);
}
