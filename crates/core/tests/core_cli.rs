//! Process-level smoke tests for the `tellur-core` binary.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn core_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_tellur-core") {
        return PathBuf::from(path);
    }
    let exe = format!("tellur-core{}", std::env::consts::EXE_SUFFIX);
    workspace_root().join("target/debug").join(exe)
}

fn require_core_binary() -> Command {
    let binary = core_binary();
    if !binary.exists() {
        let status = Command::new("cargo")
            .args(["build", "--bin", "tellur-core"])
            .current_dir(workspace_root())
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "failed to build tellur-core");
    }
    Command::new(binary)
}

#[test]
fn help_exposes_internal_contract() {
    let output = require_core_binary().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Internal diagnostics for the Tellur core library."));
    assert!(stdout.contains("doctor"));
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    let output = require_core_binary()
        .arg("definitely-not-real")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand"), "{stderr}");
}
