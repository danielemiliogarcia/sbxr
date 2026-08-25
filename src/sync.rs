use crate::{process, sha256};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const CODEX_PATHS: &[&str] = &[
    ".codex/AGENTS.md",
    ".codex/hooks.json",
    ".codex/herdr-agent-state.sh",
    ".codex/prompts",
    ".codex/rules",
    ".codex/skills",
    ".codex/plugins/cache",
];

const CLAUDE_PATHS: &[&str] = &[
    ".claude/CLAUDE.md",
    ".claude/settings.local.json",
    ".claude/statusline.sh",
    ".claude/statusline-command.sh",
    ".claude/hooks",
    ".claude/commands",
    ".claude/agents",
    ".claude/skills",
    ".claude/output-styles",
    ".claude/plugins/cache",
    ".claude/plugins/marketplaces",
    ".claude/plugins/installed_plugins.json",
    ".claude/plugins/known_marketplaces.json",
    ".claude/plugins/plugin-catalog-cache.json",
    ".claude/.caveman-active",
    ".claude/.ponytail-active",
];

const PI_PATHS: &[&str] = &[
    ".pi/agent/models.json",
    ".pi/agent/keybindings.json",
    ".pi/agent/provider-failover.json",
    ".pi/agent/skills",
    ".pi/agent/prompts",
    ".pi/agent/themes",
    ".pi/agent/npm/package.json",
    ".pi/agent/npm/package-lock.json",
];

const PI_EXTENSION_PATHS: &[&str] = &[".pi/agent/extensions"];
const REMOTE_RTK: &str = "/usr/local/bin/rtk";
const PI_NPM_LOCK_MARKER: &str = "/home/agent/.pi/agent/npm/.sbxr-lock-sha256";
const BOOTSTRAP_STATE_PATH: &str = "/home/agent/.local/state/sbxr/bootstrap-v1.json";
const BOOTSTRAP_STATE_DIRECTORY: &str = "/home/agent/.local/state/sbxr";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BootstrapState {
    pub(crate) agent: bool,
    pub(crate) vscode: bool,
}

pub(crate) fn bootstrap_state(
    sandbox_name: &str,
    preserve_xdg_state: bool,
) -> Result<BootstrapState, String> {
    let Some(bytes) = remote_file(sandbox_name, preserve_xdg_state, BOOTSTRAP_STATE_PATH)? else {
        return Ok(BootstrapState::default());
    };
    Ok(parse_bootstrap_state(&bytes))
}

fn parse_bootstrap_state(bytes: &[u8]) -> BootstrapState {
    let value: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    BootstrapState {
        agent: value.get("agent").and_then(Value::as_bool).unwrap_or(false),
        vscode: value
            .get("vscode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

pub(crate) fn write_bootstrap_state(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    state: BootstrapState,
) -> Result<(), String> {
    process::run_checked(
        process::sbx_command(preserve_xdg_state).args([
            "exec",
            sandbox_name,
            "mkdir",
            "-p",
            BOOTSTRAP_STATE_DIRECTORY,
        ]),
        "creating sandbox bootstrap-state directory",
    )?;
    let bytes = serialize_bootstrap_state(state)?;
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        BOOTSTRAP_STATE_PATH,
        &bytes,
    )
}

fn serialize_bootstrap_state(state: BootstrapState) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "agent": state.agent,
        "vscode": state.vscode,
    }))
    .map_err(|error| format!("could not serialize sandbox bootstrap state: {error}"))
}

pub fn capabilities(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    codex: bool,
    claude: bool,
    pi: bool,
) -> Result<(), String> {
    let home = process::home_dir()?;
    let home_text = home.to_string_lossy();
    let mut selected = Vec::new();
    let pi_extensions = pi && pi_extensions_enabled()?;
    if codex {
        selected.extend(existing(&home, CODEX_PATHS));
    }
    if claude {
        selected.extend(existing(&home, CLAUDE_PATHS));
        if let Some(statusline) = claude_statusline_script(&home)? {
            if !selected.contains(&statusline) {
                selected.push(statusline);
            }
        }
    }
    if pi {
        selected.extend(existing(&home, PI_PATHS));
        if pi_extensions {
            selected.extend(existing(&home, PI_EXTENSION_PATHS));
        }
    }

    if !selected.is_empty() {
        stream_tree(
            sandbox_name,
            preserve_xdg_state,
            &home,
            &home_text,
            &selected,
        )?;
    }
    if claude {
        mirror_claude_hook_tools(sandbox_name, preserve_xdg_state, &home)?;
    }
    if pi {
        sync_pi_npm_packages(sandbox_name, preserve_xdg_state, &home)?;
    }
    if codex {
        merge_codex(sandbox_name, preserve_xdg_state, &home, &home_text)?;
    }
    if claude {
        merge_json_settings(
            sandbox_name,
            preserve_xdg_state,
            &home.join(".claude/settings.json"),
            "/home/agent/.claude/settings.json",
            &home_text,
            JsonMergePolicy {
                managed_keys: &[
                    "defaultMode",
                    "bypassPermissionsModeAccepted",
                    "skipDangerousModePermissionPrompt",
                ],
                excluded_host_keys: &[],
                filter_unavailable_hooks: true,
            },
        )?;
    }
    if pi {
        merge_json_settings(
            sandbox_name,
            preserve_xdg_state,
            &home.join(".pi/agent/settings.json"),
            "/home/agent/.pi/agent/settings.json",
            &home_text,
            JsonMergePolicy {
                managed_keys: &["defaultProjectTrust", "enableInstallTelemetry"],
                excluded_host_keys: &[],
                filter_unavailable_hooks: false,
            },
        )?;
    }
    Ok(())
}

fn pi_extensions_enabled() -> Result<bool, String> {
    if let Ok(value) = env::var("SBXR_SYNC_PI_EXTENSIONS") {
        return match value.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err("SBXR_SYNC_PI_EXTENSIONS must be 0 or 1".to_owned()),
        };
    }
    Ok(true)
}

fn mirror_claude_hook_tools(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
) -> Result<(), String> {
    let settings_path = home.join(".claude/settings.json");
    if !claude_settings_uses_program(&settings_path, "rtk")? {
        return Ok(());
    }
    let Some(host_rtk) = process::command_path("rtk") else {
        return Ok(());
    };
    let host_version = command_version(Command::new(&host_rtk).arg("--version"));
    let remote_version = command_version(process::sbx_command(preserve_xdg_state).args([
        "exec",
        sandbox_name,
        "--",
        REMOTE_RTK,
        "--version",
    ]));
    if host_version.is_some() && host_version == remote_version {
        return Ok(());
    }

    let contents = fs::read(&host_rtk)
        .map_err(|error| format!("could not read host rtk at {}: {error}", host_rtk.display()))?;
    upload_root_executable(sandbox_name, preserve_xdg_state, REMOTE_RTK, &contents)?;
    let installed_version = command_version(process::sbx_command(preserve_xdg_state).args([
        "exec",
        sandbox_name,
        "--",
        REMOTE_RTK,
        "--version",
    ]));
    if host_version.is_none() || installed_version != host_version {
        let _ = process::sbx_command(preserve_xdg_state)
            .args(["exec", "-u", "root", sandbox_name, "rm", "-f", REMOTE_RTK])
            .status();
    }
    Ok(())
}

fn claude_settings_uses_program(path: &Path, wanted: &str) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    let settings: Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    Ok(settings_uses_hook_program(&settings, wanted))
}

fn settings_uses_hook_program(settings: &Value, wanted: &str) -> bool {
    let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    events.values().any(|groups| {
        groups.as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hooks| {
                        hooks.iter().any(|hook| {
                            hook.get("command")
                                .and_then(Value::as_str)
                                .and_then(hook_program)
                                == Some(wanted)
                        })
                    })
            })
        })
    })
}

fn command_version(command: &mut Command) -> Option<String> {
    command.output().ok().and_then(|output| {
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    })
}

fn upload_root_executable(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    path: &str,
    contents: &[u8],
) -> Result<(), String> {
    let mut child = process::sbx_command(preserve_xdg_state)
        .args(["exec", "-u", "root", "-i", sandbox_name, "tee", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not write remote executable {path}: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("remote executable stdin was unavailable")?
        .write_all(contents)
        .map_err(|error| format!("could not stream remote executable {path}: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("could not finish remote executable {path}: {error}"))?;
    if !status.success() {
        return Err(format!(
            "writing remote executable {path} failed with {status}"
        ));
    }
    process::run_checked(
        process::sbx_command(preserve_xdg_state).args([
            "exec",
            "-u",
            "root",
            sandbox_name,
            "chmod",
            "755",
            path,
        ]),
        "setting mirrored hook-tool permissions",
    )
}

fn sync_pi_npm_packages(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
) -> Result<(), String> {
    let lock_path = home.join(".pi/agent/npm/package-lock.json");
    let package_path = home.join(".pi/agent/npm/package.json");
    if !lock_path.is_file() || !package_path.is_file() {
        return Ok(());
    }
    let lock = fs::read(&lock_path)
        .map_err(|error| format!("could not read {}: {error}", lock_path.display()))?;
    let wanted = sha256::hex_prefix(&lock, 32);
    let current = remote_file(sandbox_name, preserve_xdg_state, PI_NPM_LOCK_MARKER)?
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned());
    if current.as_deref() == Some(&wanted) {
        return Ok(());
    }
    process::run_checked(
        process::sbx_command(preserve_xdg_state).args([
            "exec",
            sandbox_name,
            "--",
            "npm",
            "ci",
            "--legacy-peer-deps",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--prefix",
            "/home/agent/.pi/agent/npm",
        ]),
        "installing exact host Pi extension packages",
    )?;
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        PI_NPM_LOCK_MARKER,
        wanted.as_bytes(),
    )
}

fn existing(home: &Path, paths: &'static [&'static str]) -> impl Iterator<Item = PathBuf> {
    paths
        .iter()
        .copied()
        .filter(|relative| home.join(relative).exists())
        .map(PathBuf::from)
}

fn claude_statusline_script(home: &Path) -> Result<Option<PathBuf>, String> {
    let settings_path = home.join(".claude/settings.json");
    if !settings_path.is_file() {
        return Ok(None);
    }
    let settings: Value = serde_json::from_slice(
        &fs::read(&settings_path)
            .map_err(|error| format!("could not read {}: {error}", settings_path.display()))?,
    )
    .map_err(|error| format!("could not parse {}: {error}", settings_path.display()))?;
    let Some(command) = settings
        .get("statusLine")
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let home_text = home.to_string_lossy();
    let Some(relative) = statusline_relative_path(command, &home_text) else {
        return Ok(None);
    };
    Ok(home.join(&relative).is_file().then_some(relative))
}

fn statusline_relative_path(command: &str, home: &str) -> Option<PathBuf> {
    let prefixes = [
        (format!("{home}/.claude/"), ".claude/"),
        ("$HOME/.claude/".to_owned(), ".claude/"),
        ("~/.claude/".to_owned(), ".claude/"),
    ];
    for (prefix, relative_prefix) in prefixes {
        let Some(start) = command.find(&prefix) else {
            continue;
        };
        let tail = &command[start + prefix.len()..];
        let end = tail
            .find(|character: char| character.is_whitespace() || "'\";&|<>()".contains(character))
            .unwrap_or(tail.len());
        if end == 0 {
            continue;
        }
        let relative = PathBuf::from(relative_prefix).join(&tail[..end]);
        if relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Some(relative);
        }
    }
    None
}

fn stream_tree(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
    selected: &[PathBuf],
) -> Result<(), String> {
    let mut child = process::sbx_command(preserve_xdg_state)
        .arg("exec")
        .arg("-i")
        .arg(sandbox_name)
        .args(["tar", "-xf", "-", "-C", "/home/agent"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start remote capability extraction: {error}"))?;
    {
        let output = child
            .stdin
            .as_mut()
            .ok_or("remote capability extraction stdin was unavailable")?;
        let mut archive = TarWriter::new(output);
        for relative in selected {
            archive.append(&home.join(relative), relative, home_text)?;
        }
        archive.finish()?;
    }
    drop(child.stdin.take());
    let status = child
        .wait()
        .map_err(|error| format!("could not finish remote capability extraction: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("remote capability extraction failed with {status}"))
    }
}

fn merge_codex(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
) -> Result<(), String> {
    let host_path = home.join(".codex/config.toml");
    if !host_path.is_file() {
        return Ok(());
    }
    let host = fs::read_to_string(&host_path)
        .map_err(|error| format!("could not read {}: {error}", host_path.display()))?
        .replace(home_text, "/home/agent");
    let Some(managed) = remote_file(
        sandbox_name,
        preserve_xdg_state,
        "/home/agent/.codex/config.toml",
    )?
    else {
        return Ok(());
    };
    let managed = String::from_utf8(managed)
        .map_err(|error| format!("managed Codex configuration is not UTF-8: {error}"))?;
    let merged = merge_codex_toml(&managed, &host);
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        "/home/agent/.codex/config.toml",
        merged.as_bytes(),
    )
}

fn merge_codex_toml(managed: &str, host: &str) -> String {
    let mut output = String::new();
    output.push_str(&toml_root(
        host,
        &[
            "model",
            "model_reasoning_effort",
            "model_verbosity",
            "personality",
            "approvals_reviewer",
            "web_search",
            "hide_agent_reasoning",
            "show_raw_agent_reasoning",
        ],
    ));
    output.push_str(&toml_root(
        managed,
        &[
            "approval_policy",
            "sandbox_mode",
            "mcp_oauth_credentials_store",
            "forced_login_method",
            "model_provider",
        ],
    ));
    output.push_str(&toml_tables(
        host,
        &[
            "tui",
            "features",
            "notice",
            "marketplaces",
            "plugins",
            "hooks.state",
        ],
    ));
    output.push_str(&toml_tables(
        managed,
        &["model_providers.sandboxd", "mcp_servers.mcp-gateway"],
    ));
    output
}

fn toml_root(input: &str, allowed: &[&str]) -> String {
    input
        .lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .filter(|line| {
            line.split_once('=')
                .map(|(key, _)| allowed.contains(&key.trim()))
                .unwrap_or(false)
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

fn toml_tables(input: &str, allowed: &[&str]) -> String {
    let mut keep = false;
    let mut output = String::new();
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            let name = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim_matches('"');
            keep = allowed
                .iter()
                .any(|allowed| name == *allowed || name.starts_with(&format!("{allowed}.")));
        }
        if keep {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

struct JsonMergePolicy<'a> {
    managed_keys: &'a [&'a str],
    excluded_host_keys: &'a [&'a str],
    filter_unavailable_hooks: bool,
}

fn merge_json_settings(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    host_path: &Path,
    remote_path: &str,
    host_home: &str,
    policy: JsonMergePolicy<'_>,
) -> Result<(), String> {
    if !host_path.is_file() {
        return Ok(());
    }
    let mut host: Value = serde_json::from_slice(
        &fs::read(host_path)
            .map_err(|error| format!("could not read {}: {error}", host_path.display()))?,
    )
    .map_err(|error| format!("could not parse {}: {error}", host_path.display()))?;
    replace_json_paths(&mut host, host_home);
    if policy.filter_unavailable_hooks {
        prune_unavailable_claude_hooks(&mut host, |program| {
            remote_program_exists(sandbox_name, preserve_xdg_state, program)
        })?;
    }
    let managed = remote_file(sandbox_name, preserve_xdg_state, remote_path)?
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| Value::Object(Map::new()));
    let mut merged = managed.clone();
    if let (Some(target), Some(source)) = (merged.as_object_mut(), host.as_object()) {
        for (key, value) in source {
            if !policy.excluded_host_keys.contains(&key.as_str()) {
                target.insert(key.clone(), value.clone());
            }
        }
        if let Some(original) = managed.as_object() {
            for key in policy.managed_keys {
                if let Some(value) = original.get(*key) {
                    target.insert((*key).to_owned(), value.clone());
                }
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&merged)
        .map_err(|error| format!("could not serialize merged settings: {error}"))?;
    upload_file(sandbox_name, preserve_xdg_state, remote_path, &bytes)
}

fn prune_unavailable_claude_hooks<F>(
    settings: &mut Value,
    mut program_exists: F,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<bool, String>,
{
    let Some(events) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let mut availability = HashMap::new();
    for groups in events.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        let mut kept_groups = Vec::new();
        for mut group in std::mem::take(groups) {
            let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                kept_groups.push(group);
                continue;
            };
            let mut kept_entries = Vec::new();
            for entry in std::mem::take(entries) {
                let command = entry.get("command").and_then(Value::as_str);
                let program = command.and_then(hook_program);
                let available = match program {
                    Some(program) => {
                        if let Some(available) = availability.get(program) {
                            *available
                        } else {
                            let available = program_exists(program)?;
                            availability.insert(program.to_owned(), available);
                            available
                        }
                    }
                    None => true,
                };
                if available {
                    kept_entries.push(entry);
                } else if let (Some(program), Some(command)) = (program, command) {
                    if !quietly_omitted_hook_program(program) {
                        eprintln!(
                            "warning: skipped host Claude hook because '{program}' is unavailable in the sandbox: {command}"
                        );
                    }
                }
            }
            *entries = kept_entries;
            if !entries.is_empty() {
                kept_groups.push(group);
            }
        }
        *groups = kept_groups;
    }
    events.retain(|_, groups| groups.as_array().is_none_or(|groups| !groups.is_empty()));
    Ok(())
}

fn quietly_omitted_hook_program(program: &str) -> bool {
    matches!(program, "paplay" | "afplay")
}

fn hook_program(command: &str) -> Option<&str> {
    let program = command.split_whitespace().next()?;
    if program.is_empty()
        || program.contains('=')
        || program
            .chars()
            .any(|character| "'\"$;&|<>(){}!".contains(character))
        || matches!(program, "if" | "for" | "while" | "until" | "case")
    {
        None
    } else {
        Some(program)
    }
}

fn remote_program_exists(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    program: &str,
) -> Result<bool, String> {
    let status = process::sbx_command(preserve_xdg_state)
        .args([
            "exec",
            sandbox_name,
            "--",
            "sh",
            "-lc",
            "command -v -- \"$1\" >/dev/null 2>&1",
            "sh",
            program,
        ])
        .status()
        .map_err(|error| {
            format!("could not inspect sandbox hook dependency '{program}': {error}")
        })?;
    Ok(status.success())
}

fn replace_json_paths(value: &mut Value, host_home: &str) {
    match value {
        Value::String(text) => *text = text.replace(host_home, "/home/agent"),
        Value::Array(values) => {
            for value in values {
                replace_json_paths(value, host_home);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace_json_paths(value, host_home);
            }
        }
        _ => {}
    }
}

pub(crate) fn remote_file(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    path: &str,
) -> Result<Option<Vec<u8>>, String> {
    let output = process::sbx_command(preserve_xdg_state)
        .args(["exec", sandbox_name, "cat", path])
        .output()
        .map_err(|error| format!("could not read remote {path}: {error}"))?;
    Ok(output.status.success().then_some(output.stdout))
}

pub(crate) fn upload_file(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    path: &str,
    contents: &[u8],
) -> Result<(), String> {
    let mut child = process::sbx_command(preserve_xdg_state)
        .args(["exec", "-i", sandbox_name, "tee", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not write remote {path}: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("remote file stdin was unavailable")?
        .write_all(contents)
        .map_err(|error| format!("could not stream remote {path}: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("could not finish remote {path}: {error}"))?;
    if !status.success() {
        return Err(format!("writing remote {path} failed with {status}"));
    }
    process::run_checked(
        process::sbx_command(preserve_xdg_state).args(["exec", sandbox_name, "chmod", "600", path]),
        "setting remote configuration permissions",
    )
}

struct TarWriter<'a, W: Write> {
    output: &'a mut W,
}

impl<'a, W: Write> TarWriter<'a, W> {
    fn new(output: &'a mut W) -> Self {
        Self { output }
    }

    fn append(&mut self, source: &Path, archive: &Path, host_home: &str) -> Result<(), String> {
        let metadata = fs::symlink_metadata(source)
            .map_err(|error| format!("could not inspect {}: {error}", source.display()))?;
        if should_skip(archive) {
            return Ok(());
        }
        if metadata.file_type().is_symlink() {
            if fs::metadata(source).is_err() {
                return Ok(());
            }
            let target = fs::read_link(source)
                .map_err(|error| format!("could not read symlink {}: {error}", source.display()))?;
            let target = target.to_string_lossy().replace(host_home, "/home/agent");
            self.header(archive, &metadata, 0, b'2', Some(&target))?;
        } else if metadata.is_dir() {
            self.header(archive, &metadata, 0, b'5', None)?;
            let mut entries: Vec<_> = fs::read_dir(source)
                .map_err(|error| format!("could not read {}: {error}", source.display()))?
                .filter_map(Result::ok)
                .collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                self.append(&entry.path(), &archive.join(entry.file_name()), host_home)?;
            }
        } else if metadata.is_file() {
            let mut contents = Vec::new();
            File::open(source)
                .and_then(|mut file| file.read_to_end(&mut contents))
                .map_err(|error| format!("could not read {}: {error}", source.display()))?;
            if let Ok(text) = std::str::from_utf8(&contents) {
                contents = text.replace(host_home, "/home/agent").into_bytes();
            }
            self.header(archive, &metadata, contents.len() as u64, b'0', None)?;
            self.output
                .write_all(&contents)
                .map_err(|error| error.to_string())?;
            self.padding(contents.len())?;
        }
        Ok(())
    }

    fn header(
        &mut self,
        path: &Path,
        metadata: &fs::Metadata,
        size: u64,
        kind: u8,
        link: Option<&str>,
    ) -> Result<(), String> {
        let name = path.to_string_lossy().replace('\\', "/");
        let (prefix, short) = split_ustar_name(&name)?;
        let mut header = [0_u8; 512];
        field(&mut header[0..100], short.as_bytes())?;
        octal(&mut header[100..108], mode(metadata));
        octal(&mut header[108..116], 1000);
        octal(&mut header[116..124], 1000);
        octal(&mut header[124..136], size);
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs());
        octal(&mut header[136..148], modified);
        header[148..156].fill(b' ');
        header[156] = kind;
        if let Some(link) = link {
            field(&mut header[157..257], link.as_bytes())?;
        }
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        field(&mut header[265..297], b"agent")?;
        field(&mut header[297..329], b"agent")?;
        field(&mut header[345..500], prefix.as_bytes())?;
        let checksum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
        let encoded = format!("{checksum:06o}\0 ");
        header[148..156].copy_from_slice(encoded.as_bytes());
        self.output
            .write_all(&header)
            .map_err(|error| error.to_string())
    }

    fn padding(&mut self, size: usize) -> Result<(), String> {
        let padding = (512 - size % 512) % 512;
        self.output
            .write_all(&vec![0; padding])
            .map_err(|error| error.to_string())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.output
            .write_all(&[0; 1024])
            .map_err(|error| error.to_string())
    }
}

fn should_skip(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    text == ".codex/skills/.system"
        || text.starts_with(".codex/skills/.system/")
        || text.contains("/.remote-plugin-install-staging")
}

fn split_ustar_name(path: &str) -> Result<(String, String), String> {
    if path.len() <= 100 {
        return Ok((String::new(), path.to_owned()));
    }
    for (index, character) in path.char_indices().rev() {
        if character == '/' && index <= 155 && path.len() - index - 1 <= 100 {
            return Ok((path[..index].to_owned(), path[index + 1..].to_owned()));
        }
    }
    Err(format!(
        "capability path is too long for portable tar: {path}"
    ))
}

fn field(destination: &mut [u8], value: &[u8]) -> Result<(), String> {
    if value.len() > destination.len() {
        return Err("tar header field is too long".to_owned());
    }
    destination[..value.len()].copy_from_slice(value);
    Ok(())
}

fn octal(destination: &mut [u8], value: u64) {
    let width = destination.len() - 1;
    let encoded = format!("{value:0width$o}");
    destination[..width].copy_from_slice(encoded.as_bytes());
}

#[cfg(unix)]
fn mode(metadata: &fs::Metadata) -> u64 {
    u64::from(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode(metadata: &fs::Metadata) -> u64 {
    if metadata.is_dir() { 0o700 } else { 0o600 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_versioned_bootstrap_state() {
        let state = BootstrapState {
            agent: true,
            vscode: false,
        };
        let bytes = serialize_bootstrap_state(state).unwrap();
        assert_eq!(parse_bootstrap_state(&bytes), state);
        assert_eq!(
            parse_bootstrap_state(br#"{"agent":true}"#),
            BootstrapState {
                agent: true,
                vscode: false,
            }
        );
        assert_eq!(
            parse_bootstrap_state(b"not json"),
            BootstrapState::default()
        );
    }

    #[test]
    fn preserves_docker_codex_sections_and_host_preferences() {
        let managed = "forced_login_method = \"api\"\nmodel_provider = \"sandboxd\"\n[model_providers.sandboxd]\nbase_url = \"proxy\"\n";
        let host = "model = \"gpt-test\"\n[projects.\"/\"]\ntrust_level = \"trusted\"\n[tui]\nstatus_line_use_colors = true\n";
        let merged = merge_codex_toml(managed, host);
        assert!(merged.contains("model = \"gpt-test\""));
        assert!(merged.contains("model_provider = \"sandboxd\""));
        assert!(merged.contains("[tui]"));
        assert!(!merged.contains("[projects"));
    }

    #[test]
    fn splits_portable_tar_paths() {
        let path = format!("{}/file.txt", "directory/".repeat(12));
        let (prefix, name) = split_ustar_name(&path).unwrap();
        assert!(prefix.len() <= 155);
        assert!(name.len() <= 100);
    }

    #[test]
    fn finds_allowlisted_claude_statusline_commands() {
        assert_eq!(
            statusline_relative_path("bash '/home/dev/.claude/status/token-line.sh'", "/home/dev"),
            Some(PathBuf::from(".claude/status/token-line.sh"))
        );
        assert_eq!(
            statusline_relative_path("$HOME/.claude/statusline.sh", "/home/dev"),
            Some(PathBuf::from(".claude/statusline.sh"))
        );
        assert_eq!(
            statusline_relative_path("bash /tmp/untrusted.sh", "/home/dev"),
            None
        );
    }

    #[test]
    fn removes_claude_hooks_with_missing_sandbox_commands() {
        let mut settings = serde_json::json!({
            "hooks": {
                "Stop": [{"hooks": [
                    {"type": "command", "command": "paplay complete.oga"},
                    {"type": "command", "command": "bash /home/agent/.claude/hooks/done.sh"}
                ]}],
                "PreToolUse": [{"hooks": [
                    {"type": "command", "command": "rtk hook claude"}
                ]}]
            }
        });
        prune_unavailable_claude_hooks(&mut settings, |program| Ok(program == "bash")).unwrap();
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("bash /home/agent/.claude/hooks/done.sh"));
        assert!(!text.contains("paplay"));
        assert!(!text.contains("rtk"));
        assert!(settings["hooks"].get("PreToolUse").is_none());
    }

    #[test]
    fn recognizes_only_simple_hook_programs() {
        assert_eq!(hook_program("paplay sound.oga"), Some("paplay"));
        assert_eq!(hook_program("bash '/home/agent/hook.sh'"), Some("bash"));
        assert_eq!(hook_program("FLAG=1 command"), None);
        assert_eq!(hook_program("if command -v tool; then tool; fi"), None);
        assert!(quietly_omitted_hook_program("paplay"));
        assert!(quietly_omitted_hook_program("afplay"));
        assert!(!quietly_omitted_hook_program("rtk"));
    }

    #[test]
    fn detects_claude_hook_tool_usage() {
        let settings = serde_json::json!({
            "hooks": {
                "PreToolUse": [{"hooks": [
                    {"type": "command", "command": "rtk hook claude"}
                ]}]
            }
        });
        assert!(settings_uses_hook_program(&settings, "rtk"));
        assert!(!settings_uses_hook_program(&settings, "paplay"));
    }
}
