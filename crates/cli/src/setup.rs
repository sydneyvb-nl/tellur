//! `tellur setup` — install, uninstall, and inspect the global editor & agent
//! integrations (Claude Code, Codex, Cursor, VS Code, Windsurf, Gemini CLI,
//! Antigravity): hook configs, MCP server entries, and the single-owner Codex
//! plugin hook surface.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::util::{shell_quote, tellur_executable_path};

const TELLUR_CODEX_HOOK_SOURCE: &str = "codex";
const TELLUR_CLAUDE_HOOK_SOURCE: &str = "claude-code";
const TELLUR_CURSOR_HOOK_SOURCE: &str = "cursor";
const TELLUR_VSCODE_HOOK_SOURCE: &str = "vscode";
const TELLUR_WINDSURF_HOOK_SOURCE: &str = "windsurf";
const TELLUR_GEMINI_HOOK_SOURCE: &str = "gemini-cli";
const TELLUR_ANTIGRAVITY_HOOK_SOURCE: &str = "antigravity";

fn home_dir_override(home: Option<&Path>) -> Result<PathBuf> {
    if let Some(home) = home {
        return Ok(home.to_path_buf());
    }
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; pass --home explicitly")
}

pub(crate) fn cmd_setup_agents(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    let codex_command = tellur_hook_command(TELLUR_CODEX_HOOK_SOURCE)?;
    let claude_command = tellur_hook_command(TELLUR_CLAUDE_HOOK_SOURCE)?;
    install_claude_global_hooks(&home, &claude_command)?;
    // Codex loads plugin hooks and user-level hooks together on supported
    // versions. Installing the same handler in both places duplicates every
    // event, so the personal plugin is the single hook delivery surface.
    remove_hook_command_from_json(&home.join(".codex/hooks.json"), TELLUR_CODEX_HOOK_SOURCE)?;
    install_codex_personal_plugin(&home, &codex_command)?;
    install_cursor_integration(&home, &tellur_exe)?;
    install_vscode_integration(&home, &tellur_exe)?;
    install_windsurf_integration(&home, &tellur_exe)?;
    install_gemini_cli_integration(&home)?;
    install_antigravity_integration(&home, &tellur_exe)?;
    println!(
        "✓ Configured Tellur capture for Claude Code, Codex, Cursor, VS Code, Windsurf, Gemini CLI, and Antigravity"
    );
    println!(
        "  Claude Code hooks: {}",
        home.join(".claude/settings.json").display()
    );
    println!(
        "  Codex hooks + plugin marketplace: {}",
        home.join(".agents/plugins/marketplace.json").display()
    );
    println!(
        "  Cursor MCP/settings: {}",
        cursor_mcp_path(&home).display()
    );
    println!(
        "  VS Code extension settings (extension distributed separately): {}",
        vscode_user_settings_path(&home).display()
    );
    println!(
        "  Windsurf MCP/settings: {}",
        windsurf_mcp_path(&home).display()
    );
    println!(
        "  Gemini CLI settings: {}",
        gemini_settings_path(&home).display()
    );
    println!(
        "  Antigravity hooks: {}",
        antigravity_hooks_path(&home).display()
    );
    println!("  Restart Codex/Claude Code and review/trust hooks once when prompted.");
    Ok(())
}

pub(crate) fn cmd_setup_codex(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let codex_command = tellur_hook_command(TELLUR_CODEX_HOOK_SOURCE)?;
    remove_hook_command_from_json(&home.join(".codex/hooks.json"), TELLUR_CODEX_HOOK_SOURCE)?;
    install_codex_personal_plugin(&home, &codex_command)?;
    println!("✓ Installed Tellur global Codex integration");
    println!(
        "  Hooks + plugin marketplace: {}",
        home.join(".agents/plugins/marketplace.json").display()
    );
    Ok(())
}

pub(crate) fn cmd_setup_claude_code(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let claude_command = tellur_hook_command(TELLUR_CLAUDE_HOOK_SOURCE)?;
    install_claude_global_hooks(&home, &claude_command)?;
    println!("✓ Installed Tellur global Claude Code integration");
    println!("  Hooks: {}", home.join(".claude/settings.json").display());
    Ok(())
}

pub(crate) fn cmd_setup_cursor(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_cursor_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global Cursor integration");
    println!("  MCP: {}", cursor_mcp_path(&home).display());
    println!("  Settings: {}", cursor_user_settings_path(&home).display());
    Ok(())
}

pub(crate) fn cmd_setup_vscode(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_vscode_integration(&home, &tellur_exe)?;
    println!("✓ Prepared Tellur settings for the VS Code extension");
    println!("  Settings: {}", vscode_user_settings_path(&home).display());
    println!("  Note: this does not install the extension package.");
    Ok(())
}

pub(crate) fn cmd_setup_windsurf(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_windsurf_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global Windsurf integration");
    println!("  MCP: {}", windsurf_mcp_path(&home).display());
    println!(
        "  Settings: {}",
        windsurf_user_settings_path(&home).display()
    );
    Ok(())
}

pub(crate) fn cmd_setup_gemini_cli(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    install_gemini_cli_integration(&home)?;
    println!("✓ Installed Tellur global Gemini CLI integration");
    println!("  Settings: {}", gemini_settings_path(&home).display());
    Ok(())
}

pub(crate) fn cmd_setup_antigravity(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_antigravity_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global Antigravity integration");
    println!("  Hooks: {}", antigravity_hooks_path(&home).display());
    println!(
        "  MCP: {}, {}",
        antigravity_mcp_path(&home).display(),
        antigravity_cli_mcp_path(&home).display()
    );
    Ok(())
}

pub(crate) fn cmd_setup_status(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let claude = hook_config_has_tellur_source(
        &home.join(".claude/settings.json"),
        TELLUR_CLAUDE_HOOK_SOURCE,
    );
    let codex_legacy =
        hook_config_has_tellur_source(&home.join(".codex/hooks.json"), TELLUR_CODEX_HOOK_SOURCE);
    let plugin = codex_plugin_status(&home);
    let cursor = cursor_integration_status(&home);
    let vscode = vscode_integration_status(&home);
    let windsurf = windsurf_integration_status(&home);
    let gemini = gemini_integration_status(&home);
    let antigravity = antigravity_integration_status(&home);
    println!(
        "Claude Code global hooks: {}",
        if claude { "installed" } else { "missing" }
    );
    println!(
        "Codex hook delivery: {}",
        if plugin && !codex_legacy {
            "installed (personal plugin)"
        } else if plugin {
            "duplicate (plugin + legacy global hooks)"
        } else if codex_legacy {
            "legacy global hooks only"
        } else {
            "missing"
        }
    );
    println!(
        "Codex personal plugin: {}",
        if plugin { "installed" } else { "missing" }
    );
    println!(
        "Cursor global integration: {}",
        if cursor { "installed" } else { "missing" }
    );
    println!(
        "VS Code extension settings: {}",
        if vscode { "prepared" } else { "missing" }
    );
    println!(
        "Windsurf global integration: {}",
        if windsurf { "installed" } else { "missing" }
    );
    println!(
        "Gemini CLI global integration: {}",
        if gemini { "installed" } else { "missing" }
    );
    println!(
        "Antigravity global integration: {}",
        if antigravity { "installed" } else { "missing" }
    );
    Ok(())
}

pub(crate) fn cmd_setup_uninstall(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    remove_hook_command_from_json(
        &home.join(".claude/settings.json"),
        TELLUR_CLAUDE_HOOK_SOURCE,
    )?;
    remove_hook_command_from_json(&home.join(".codex/hooks.json"), TELLUR_CODEX_HOOK_SOURCE)?;
    let _ = std::fs::remove_dir_all(home.join(".codex/plugins/tellur-provenance"));
    remove_codex_marketplace_entry(&home)?;
    uninstall_cursor_integration(&home)?;
    uninstall_vscode_integration(&home)?;
    uninstall_windsurf_integration(&home)?;
    uninstall_gemini_cli_integration(&home)?;
    uninstall_antigravity_integration(&home)?;
    println!("✓ Removed Tellur global integrations where present");
    Ok(())
}

fn tellur_hook_command(source: &str) -> Result<String> {
    let exe = tellur_executable_path()?;
    Ok(format!(
        "{} hooks ingest --source {} --auto-init",
        shell_quote(&exe.to_string_lossy()),
        source
    ))
}

fn hook_config_has_tellur_source(path: &Path, source: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value
        .get("hooks")
        .and_then(|hooks| hooks.as_object())
        .is_some_and(|hooks| {
            hooks.values().any(|entries| {
                entries.as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry
                            .get("hooks")
                            .and_then(|hooks| hooks.as_array())
                            .is_some_and(|handlers| {
                                handlers.iter().any(|handler| {
                                    hook_command_matches_source(handler, source)
                                        && hook_command_executable_exists(handler)
                                })
                            })
                    })
                })
            })
        })
}

fn hook_command_matches_source(handler: &serde_json::Value, source: &str) -> bool {
    handler
        .get("command")
        .and_then(|command| command.as_str())
        .is_some_and(|command| {
            command.contains("hooks ingest")
                && command.contains("--auto-init")
                && command.contains(&format!("--source {}", source))
        })
}

fn hook_command_executable_exists(handler: &serde_json::Value) -> bool {
    let Some(command) = handler.get("command").and_then(|command| command.as_str()) else {
        return false;
    };
    command_executable_path(command).is_some_and(|path| path.exists())
}

fn command_executable_path(command: &str) -> Option<PathBuf> {
    let command = command.trim_start();
    if let Some(rest) = command.strip_prefix('\'') {
        let mut parsed = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\'' {
                break;
            }
            if ch == '\\' && chars.peek() == Some(&'\'') {
                let _ = chars.next();
                parsed.push('\'');
            } else {
                parsed.push(ch);
            }
        }
        return Some(PathBuf::from(parsed));
    }
    command
        .split_whitespace()
        .next()
        .filter(|part| part.starts_with('/'))
        .map(PathBuf::from)
}

fn codex_plugin_status(home: &Path) -> bool {
    let plugin_manifest = home.join(".codex/plugins/tellur-provenance/.codex-plugin/plugin.json");
    let hooks = home.join(".codex/plugins/tellur-provenance/hooks/hooks.json");
    let marketplace = home.join(".agents/plugins/marketplace.json");
    plugin_manifest.exists()
        && hooks.exists()
        && marketplace_plugin_path(&marketplace)
            .as_deref()
            .is_some_and(|path| path == "./.codex/plugins/tellur-provenance")
        && codex_config_plugin_enabled(home)
}

fn codex_config_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml")
}

fn codex_config_plugin_enabled(home: &Path) -> bool {
    std::fs::read_to_string(codex_config_path(home)).is_ok_and(|content| {
        content
            .lines()
            .position(|line| line.trim() == r#"[plugins."tellur-provenance@tellur-local"]"#)
            .is_some_and(|idx| {
                content
                    .lines()
                    .skip(idx + 1)
                    .take_while(|line| !line.trim_start().starts_with('['))
                    .any(|line| line.trim() == "enabled = true")
            })
    })
}

fn marketplace_plugin_path(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    value
        .get("plugins")?
        .as_array()?
        .iter()
        .find(|plugin| {
            plugin.get("name").and_then(|name| name.as_str()) == Some("tellur-provenance")
        })
        .and_then(|plugin| plugin.get("source"))
        .and_then(|source| source.get("path"))
        .and_then(|path| path.as_str())
        .map(ToString::to_string)
}

fn cursor_mcp_path(home: &Path) -> PathBuf {
    home.join(".cursor/mcp.json")
}

fn cursor_user_settings_path(home: &Path) -> PathBuf {
    editor_user_settings_path(home, "Cursor")
}

fn vscode_user_settings_path(home: &Path) -> PathBuf {
    editor_user_settings_path(home, "Code")
}

fn windsurf_user_settings_path(home: &Path) -> PathBuf {
    editor_user_settings_path(home, "Windsurf")
}

fn windsurf_mcp_path(home: &Path) -> PathBuf {
    home.join(".codeium/windsurf/mcp_config.json")
}

fn gemini_settings_path(home: &Path) -> PathBuf {
    home.join(".gemini/settings.json")
}

fn antigravity_hooks_path(home: &Path) -> PathBuf {
    home.join(".gemini/config/hooks.json")
}

fn antigravity_mcp_path(home: &Path) -> PathBuf {
    home.join(".gemini/antigravity/mcp_config.json")
}

fn antigravity_cli_mcp_path(home: &Path) -> PathBuf {
    home.join(".gemini/antigravity-cli/mcp_config.json")
}

fn editor_user_settings_path(home: &Path, app_name: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join(app_name)
            .join("User/settings.json")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join("AppData/Roaming"))
            .join(app_name)
            .join("User/settings.json")
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        // On Linux, VS Code-family editors store user settings under
        // ~/.config/<AppName>/User/settings.json (Code, Cursor, Windsurf, ...).
        home.join(".config")
            .join(app_name)
            .join("User/settings.json")
    }
}

fn install_cursor_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_editor_settings(
        &cursor_user_settings_path(home),
        tellur_exe,
        TELLUR_CURSOR_HOOK_SOURCE,
        "Cursor",
    )?;
    install_cursor_mcp(home, tellur_exe)?;
    Ok(())
}

fn install_vscode_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_editor_settings(
        &vscode_user_settings_path(home),
        tellur_exe,
        TELLUR_VSCODE_HOOK_SOURCE,
        "VS Code AI",
    )
}

fn install_editor_settings(
    path: &Path,
    tellur_exe: &Path,
    agent_id: &str,
    agent_name: &str,
) -> Result<()> {
    let mut settings = read_json_object_or_empty(path)?;
    settings.insert(
        "tellur.tellurPath".to_string(),
        serde_json::Value::String(tellur_exe.to_string_lossy().to_string()),
    );
    settings.insert("tellur.autoInit".to_string(), serde_json::json!(true));
    settings.insert("tellur.autoWatch".to_string(), serde_json::json!(true));
    settings.insert("tellur.captureOnSave".to_string(), serde_json::json!(true));
    settings.insert(
        "tellur.vscodeAgentId".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    settings.insert(
        "tellur.vscodeAgentName".to_string(),
        serde_json::Value::String(agent_name.to_string()),
    );
    write_json_object(path, settings)
}

fn install_cursor_mcp(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_tellur_mcp_server(&cursor_mcp_path(home), tellur_exe)
}

/// Write a `tellur mcp` server entry into an `mcpServers` JSON config, preserving
/// any other servers already configured. Shared by Cursor and Windsurf, which
/// both use the standard `mcpServers` config shape.
fn install_tellur_mcp_server(path: &Path, tellur_exe: &Path) -> Result<()> {
    let mut config = read_json_object_or_empty(path)?;
    let servers = config
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    servers.as_object_mut().unwrap().insert(
        "tellur".to_string(),
        serde_json::json!({
            "command": tellur_exe.to_string_lossy(),
            "args": ["mcp"]
        }),
    );
    write_json_object(path, config)
}

fn read_json_object_or_empty(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    let value = serde_json::from_str::<serde_json::Value>(&content)
        .with_context(|| format!("invalid JSON in {}; refusing to overwrite", path.display()))?;
    value
        .as_object()
        .cloned()
        .with_context(|| format!("{} must contain a JSON object", path.display()))
}

fn write_json_object(
    path: &Path,
    object: serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&serde_json::Value::Object(object))?,
    )?;
    Ok(())
}

fn cursor_integration_status(home: &Path) -> bool {
    editor_settings_status(&cursor_user_settings_path(home), TELLUR_CURSOR_HOOK_SOURCE)
        && cursor_mcp_status(home)
}

fn vscode_integration_status(home: &Path) -> bool {
    editor_settings_status(&vscode_user_settings_path(home), TELLUR_VSCODE_HOOK_SOURCE)
}

fn editor_settings_status(path: &Path, agent_id: &str) -> bool {
    let Ok(settings) = read_json_object_or_empty(path) else {
        return false;
    };
    let Some(tellur_path) = settings.get("tellur.tellurPath").and_then(|v| v.as_str()) else {
        return false;
    };
    Path::new(tellur_path).exists()
        && settings
            .get("tellur.autoInit")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && settings
            .get("tellur.captureOnSave")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && settings
            .get("tellur.vscodeAgentId")
            .and_then(|v| v.as_str())
            == Some(agent_id)
}

fn cursor_mcp_status(home: &Path) -> bool {
    tellur_mcp_server_status(&cursor_mcp_path(home))
}

fn tellur_mcp_server_status(path: &Path) -> bool {
    let Ok(config) = read_json_object_or_empty(path) else {
        return false;
    };
    let Some(server) = config
        .get("mcpServers")
        .and_then(|v| v.get("tellur"))
        .and_then(|v| v.as_object())
    else {
        return false;
    };
    let Some(command) = server.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    Path::new(command).exists()
        && server
            .get("args")
            .and_then(|v| v.as_array())
            .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("mcp")))
}

fn uninstall_cursor_integration(home: &Path) -> Result<()> {
    remove_editor_settings(&cursor_user_settings_path(home))?;
    remove_cursor_mcp(home)
}

fn uninstall_vscode_integration(home: &Path) -> Result<()> {
    remove_editor_settings(&vscode_user_settings_path(home))
}

fn remove_editor_settings(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_json_object_or_empty(path)?;
    for key in [
        "tellur.tellurPath",
        "tellur.autoInit",
        "tellur.autoWatch",
        "tellur.captureOnSave",
        "tellur.vscodeAgentId",
        "tellur.vscodeAgentName",
        "tellur.vscodeModelId",
        "tellur.vscodePromptSessionId",
    ] {
        settings.remove(key);
    }
    write_json_object(path, settings)
}

fn remove_cursor_mcp(home: &Path) -> Result<()> {
    remove_tellur_mcp_server(&cursor_mcp_path(home))
}

fn remove_tellur_mcp_server(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json_object_or_empty(path)?;
    if let Some(servers) = config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove("tellur");
    }
    write_json_object(path, config)
}

fn install_windsurf_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_editor_settings(
        &windsurf_user_settings_path(home),
        tellur_exe,
        TELLUR_WINDSURF_HOOK_SOURCE,
        "Windsurf / Cascade",
    )?;
    install_tellur_mcp_server(&windsurf_mcp_path(home), tellur_exe)?;
    Ok(())
}

fn windsurf_integration_status(home: &Path) -> bool {
    editor_settings_status(
        &windsurf_user_settings_path(home),
        TELLUR_WINDSURF_HOOK_SOURCE,
    ) && tellur_mcp_server_status(&windsurf_mcp_path(home))
}

fn uninstall_windsurf_integration(home: &Path) -> Result<()> {
    remove_editor_settings(&windsurf_user_settings_path(home))?;
    remove_tellur_mcp_server(&windsurf_mcp_path(home))
}

fn install_gemini_cli_integration(home: &Path) -> Result<()> {
    let command = tellur_hook_command_with_json_response(TELLUR_GEMINI_HOOK_SOURCE)?;
    install_gemini_hooks_json(&gemini_settings_path(home), &command)
}

fn tellur_hook_command_with_json_response(source: &str) -> Result<String> {
    let exe = tellur_executable_path()?;
    Ok(format!(
        "{} hooks ingest --source {} --auto-init --json-response",
        shell_quote(&exe.to_string_lossy()),
        source
    ))
}

fn install_gemini_hooks_json(path: &Path, command: &str) -> Result<()> {
    let mut settings = read_json_object_or_empty(path)?;
    let hooks = settings
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();
    for (event, matcher) in [
        ("SessionStart", "startup|resume"),
        ("BeforeAgent", "*"),
        (
            "BeforeTool",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        (
            "AfterTool",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        ("SessionEnd", "exit|shutdown"),
    ] {
        merge_named_setup_hook(
            hooks,
            event,
            matcher,
            "tellur-provenance",
            command,
            TELLUR_GEMINI_HOOK_SOURCE,
        );
    }
    let hooks_config = settings
        .entry("hooksConfig".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !hooks_config.is_object() {
        *hooks_config = serde_json::json!({});
    }
    hooks_config
        .as_object_mut()
        .unwrap()
        .insert("enabled".to_string(), serde_json::Value::Bool(true));
    write_json_object(path, settings)
}

fn merge_named_setup_hook(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    matcher: &str,
    name: &str,
    command: &str,
    source: &str,
) {
    let arr = hooks
        .entry(event.to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !arr.is_array() {
        *arr = serde_json::json!([]);
    }
    if let Some(entries) = arr.as_array_mut() {
        for entry in entries {
            if let Some(handlers) = entry
                .get_mut("hooks")
                .and_then(|hooks| hooks.as_array_mut())
            {
                for handler in handlers {
                    let name_matches =
                        handler.get("name").and_then(|value| value.as_str()) == Some(name);
                    if name_matches || hook_command_matches_source(handler, source) {
                        *handler = serde_json::json!({
                            "name": name,
                            "type": "command",
                            "command": command,
                            "timeout": 30
                        });
                        return;
                    }
                }
            }
        }
    }
    arr.as_array_mut().unwrap().push(serde_json::json!({
        "matcher": matcher,
        "hooks": [
            {
                "name": name,
                "type": "command",
                "command": command,
                "timeout": 30
            }
        ]
    }));
}

fn install_antigravity_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    let command = tellur_hook_command_with_json_response(TELLUR_ANTIGRAVITY_HOOK_SOURCE)?;
    install_antigravity_hooks_json(&antigravity_hooks_path(home), &command)?;
    install_antigravity_mcp(&antigravity_mcp_path(home), tellur_exe)?;
    install_antigravity_mcp(&antigravity_cli_mcp_path(home), tellur_exe)?;
    Ok(())
}

fn install_antigravity_hooks_json(path: &Path, command: &str) -> Result<()> {
    let mut root = read_json_object_or_empty(path)?;
    let hook = root
        .entry("tellur-provenance".to_string())
        .or_insert_with(|| serde_json::json!({ "enabled": true }));
    if !hook.is_object() {
        *hook = serde_json::json!({ "enabled": true });
    }
    let hook = hook.as_object_mut().unwrap();
    hook.insert("enabled".to_string(), serde_json::Value::Bool(true));
    for (event, matcher) in [
        ("SessionStart", "startup|resume"),
        (
            "PreToolUse",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        (
            "PostToolUse",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        ("SessionEnd", "exit|shutdown"),
    ] {
        merge_named_setup_hook(
            hook,
            event,
            matcher,
            "tellur-provenance",
            command,
            TELLUR_ANTIGRAVITY_HOOK_SOURCE,
        );
    }
    write_json_object(path, root)
}

fn install_antigravity_mcp(path: &Path, tellur_exe: &Path) -> Result<()> {
    let mut config = read_json_object_or_empty(path)?;
    let servers = config
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    servers.as_object_mut().unwrap().insert(
        "tellur".to_string(),
        serde_json::json!({
            "command": tellur_exe.to_string_lossy(),
            "args": ["mcp"]
        }),
    );
    write_json_object(path, config)
}

fn gemini_integration_status(home: &Path) -> bool {
    hook_config_has_tellur_source(&gemini_settings_path(home), TELLUR_GEMINI_HOOK_SOURCE)
}

fn antigravity_integration_status(home: &Path) -> bool {
    antigravity_hook_status(home)
        && antigravity_mcp_status(&antigravity_mcp_path(home))
        && antigravity_mcp_status(&antigravity_cli_mcp_path(home))
}

fn antigravity_hook_status(home: &Path) -> bool {
    let Ok(root) = read_json_object_or_empty(&antigravity_hooks_path(home)) else {
        return false;
    };
    root.get("tellur-provenance")
        .and_then(|hook| hook.as_object())
        .is_some_and(|hook| {
            hook.values().any(|entries| {
                entries.as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry
                            .get("hooks")
                            .and_then(|hooks| hooks.as_array())
                            .is_some_and(|handlers| {
                                handlers.iter().any(|handler| {
                                    hook_command_matches_source(
                                        handler,
                                        TELLUR_ANTIGRAVITY_HOOK_SOURCE,
                                    ) && hook_command_executable_exists(handler)
                                })
                            })
                    })
                })
            })
        })
}

fn antigravity_mcp_status(path: &Path) -> bool {
    let Ok(config) = read_json_object_or_empty(path) else {
        return false;
    };
    let Some(server) = config
        .get("mcpServers")
        .and_then(|v| v.get("tellur"))
        .and_then(|v| v.as_object())
    else {
        return false;
    };
    let Some(command) = server.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    Path::new(command).exists()
        && server
            .get("args")
            .and_then(|v| v.as_array())
            .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("mcp")))
}

fn uninstall_gemini_cli_integration(home: &Path) -> Result<()> {
    remove_gemini_hooks(&gemini_settings_path(home), TELLUR_GEMINI_HOOK_SOURCE)
}

fn uninstall_antigravity_integration(home: &Path) -> Result<()> {
    remove_antigravity_hooks(home)?;
    remove_antigravity_mcp(&antigravity_mcp_path(home))?;
    remove_antigravity_mcp(&antigravity_cli_mcp_path(home))
}

fn remove_gemini_hooks(path: &Path, source: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_json_object_or_empty(path)?;
    if let Some(hooks) = settings.get_mut("hooks").and_then(|v| v.as_object_mut()) {
        remove_matching_named_hooks(hooks, source);
    }
    write_json_object(path, settings)
}

fn remove_matching_named_hooks(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    source: &str,
) {
    for entries in hooks.values_mut() {
        if let Some(arr) = entries.as_array_mut() {
            arr.retain(|entry| {
                !entry
                    .get("hooks")
                    .and_then(|hooks| hooks.as_array())
                    .is_some_and(|handlers| {
                        handlers
                            .iter()
                            .any(|handler| hook_command_matches_source(handler, source))
                    })
            });
        }
    }
}

fn remove_antigravity_hooks(home: &Path) -> Result<()> {
    let path = antigravity_hooks_path(home);
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_json_object_or_empty(&path)?;
    root.remove("tellur-provenance");
    write_json_object(&path, root)
}

fn remove_antigravity_mcp(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json_object_or_empty(path)?;
    if let Some(servers) = config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove("tellur");
    }
    write_json_object(path, config)
}

fn install_claude_global_hooks(home: &Path, command: &str) -> Result<()> {
    let path = home.join(".claude/settings.json");
    install_hooks_json(&path, command, false)
}

fn install_hooks_json(path: &Path, command: &str, include_codex_matchers: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut settings = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str::<serde_json::Value>(&content)
            .with_context(|| format!("invalid JSON in {}; refusing to overwrite", path.display()))?
    } else {
        serde_json::json!({})
    };
    if !settings
        .get("hooks")
        .map(|hooks| hooks.is_object())
        .unwrap_or(false)
    {
        settings["hooks"] = serde_json::json!({});
    }
    let hooks = settings["hooks"].as_object_mut().unwrap();
    merge_setup_hook(
        hooks,
        "SessionStart",
        Some("startup|resume|clear|compact"),
        command,
    );
    merge_setup_hook(hooks, "UserPromptSubmit", None, command);
    merge_setup_hook(hooks, "Stop", None, command);
    if include_codex_matchers {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
    } else {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
    }
    std::fs::write(path, serde_json::to_string_pretty(&settings)?)?;
    Ok(())
}

fn merge_setup_hook(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) {
    let arr = hooks
        .entry(event.to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !arr.is_array() {
        *arr = serde_json::json!([]);
    }
    let already = arr.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|hs| {
                    hs.iter()
                        .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(command))
                })
        })
    });
    if already {
        return;
    }
    let mut entry = serde_json::json!({
        "hooks": [
            {
                "type": "command",
                "command": command,
                "timeout": 30,
                "statusMessage": "Recording Tellur provenance"
            }
        ]
    });
    if let Some(matcher) = matcher {
        entry["matcher"] = serde_json::Value::String(matcher.to_string());
    }
    arr.as_array_mut().unwrap().push(entry);
}

fn remove_hook_command_from_json(path: &Path, source: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path)?;
    let mut value = serde_json::from_str::<serde_json::Value>(&content)
        .with_context(|| format!("invalid JSON in {}; refusing to overwrite", path.display()))?;
    if let Some(hooks) = value.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for entries in hooks.values_mut() {
            if let Some(arr) = entries.as_array_mut() {
                arr.retain(|entry| {
                    !entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .is_some_and(|hs| hs.iter().any(|h| hook_command_matches_source(h, source)))
                });
            }
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

fn install_codex_personal_plugin(home: &Path, command: &str) -> Result<()> {
    let plugin_root = home.join(".codex/plugins/tellur-provenance");
    std::fs::create_dir_all(plugin_root.join(".codex-plugin"))?;
    std::fs::create_dir_all(plugin_root.join("skills/tellur-provenance"))?;
    std::fs::create_dir_all(plugin_root.join("hooks"))?;

    std::fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "tellur-provenance",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Tellur AI provenance workflows for Codex",
            "skills": "./skills/"
        }))?,
    )?;
    std::fs::write(
        plugin_root.join("skills/tellur-provenance/SKILL.md"),
        r#"---
name: tellur-provenance
description: Use Tellur to inspect AI provenance, verify event integrity, and generate PR provenance reports.
---

Use the local `tellur` CLI for provenance workflows:

- `tellur status`
- `tellur sessions`
- `tellur verify`
- `tellur pr-report --base main`

Do not store raw prompts. Tellur records prompt hashes and sanitized metadata.
"#,
    )?;
    let hooks = tellur_hooks_json(command, true);
    std::fs::write(
        plugin_root.join("hooks/hooks.json"),
        serde_json::to_string_pretty(&hooks)?,
    )?;

    let marketplace_path = home.join(".agents/plugins/marketplace.json");
    if let Some(parent) = marketplace_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut marketplace = if marketplace_path.exists() {
        let content = std::fs::read_to_string(&marketplace_path)?;
        serde_json::from_str::<serde_json::Value>(&content).with_context(|| {
            format!(
                "invalid JSON in {}; refusing to overwrite",
                marketplace_path.display()
            )
        })?
    } else {
        serde_json::json!({
            "name": "tellur-local",
            "interface": { "displayName": "Tellur Local" },
            "plugins": []
        })
    };
    marketplace["name"] = serde_json::json!("tellur-local");
    marketplace["interface"] = serde_json::json!({ "displayName": "Tellur Local" });
    if !marketplace
        .get("plugins")
        .map(|plugins| plugins.is_array())
        .unwrap_or(false)
    {
        marketplace["plugins"] = serde_json::json!([]);
    }
    let plugins = marketplace["plugins"].as_array_mut().unwrap();
    plugins.retain(|p| p.get("name").and_then(|n| n.as_str()) != Some("tellur-provenance"));
    plugins.push(serde_json::json!({
        "name": "tellur-provenance",
        "source": {
            "source": "local",
            "path": "./.codex/plugins/tellur-provenance"
        },
        "policy": {
            "installation": "AVAILABLE",
            "authentication": "ON_INSTALL"
        },
        "category": "Productivity"
    }));
    std::fs::write(
        marketplace_path,
        serde_json::to_string_pretty(&marketplace)?,
    )?;
    enable_codex_plugin_in_config(home)?;
    Ok(())
}

fn enable_codex_plugin_in_config(home: &Path) -> Result<()> {
    let path = codex_config_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let content = remove_toml_section(&content, r#"[plugins."tellur-provenance@tellur-local"]"#);
    let mut content = content.trim_end().to_string();
    if !content.is_empty() {
        content.push_str("\n\n");
    }
    content.push_str(
        r#"[plugins."tellur-provenance@tellur-local"]
enabled = true
"#,
    );
    std::fs::write(path, content)?;
    Ok(())
}

fn tellur_hooks_json(command: &str, codex: bool) -> serde_json::Value {
    let mut value = serde_json::json!({ "hooks": {} });
    let hooks = value["hooks"].as_object_mut().unwrap();
    merge_setup_hook(
        hooks,
        "SessionStart",
        Some("startup|resume|clear|compact"),
        command,
    );
    merge_setup_hook(hooks, "UserPromptSubmit", None, command);
    merge_setup_hook(hooks, "Stop", None, command);
    if codex {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
    } else {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
    }
    value
}

fn remove_codex_marketplace_entry(home: &Path) -> Result<()> {
    let marketplace_path = home.join(".agents/plugins/marketplace.json");
    if !marketplace_path.exists() {
        disable_codex_plugin_in_config(home)?;
        return Ok(());
    }
    let content = std::fs::read_to_string(&marketplace_path)?;
    let mut marketplace =
        serde_json::from_str::<serde_json::Value>(&content).with_context(|| {
            format!(
                "invalid JSON in {}; refusing to overwrite",
                marketplace_path.display()
            )
        })?;
    if let Some(plugins) = marketplace
        .get_mut("plugins")
        .and_then(|p| p.as_array_mut())
    {
        plugins.retain(|p| p.get("name").and_then(|n| n.as_str()) != Some("tellur-provenance"));
    }
    std::fs::write(
        marketplace_path,
        serde_json::to_string_pretty(&marketplace)?,
    )?;
    disable_codex_plugin_in_config(home)?;
    Ok(())
}

fn disable_codex_plugin_in_config(home: &Path) -> Result<()> {
    let path = codex_config_path(home);
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&path)?;
    let content = remove_toml_section(&content, r#"[plugins."tellur-provenance@tellur-local"]"#);
    std::fs::write(path, content.trim_end())?;
    Ok(())
}

fn remove_toml_section(content: &str, section: &str) -> String {
    let mut output = Vec::new();
    let mut skipping = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == section {
            skipping = true;
            continue;
        }
        if skipping && trimmed.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            output.push(line);
        }
    }
    output.join("\n")
}
