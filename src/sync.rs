use crate::process;
use serde_json::{Map, Value};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
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
];

const PI_EXTENSION_PATHS: &[&str] = &[".pi/agent/extensions", ".pi/agent/npm"];

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
    let pi_extensions = if pi {
        pi_extensions_compatible(sandbox_name, preserve_xdg_state)?
    } else {
        false
    };
    if codex {
        selected.extend(existing(&home, CODEX_PATHS));
    }
    if claude {
        selected.extend(existing(&home, CLAUDE_PATHS));
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
            &[
                "defaultMode",
                "bypassPermissionsModeAccepted",
                "skipDangerousModePermissionPrompt",
            ],
            &[],
        )?;
    }
    if pi {
        merge_json_settings(
            sandbox_name,
            preserve_xdg_state,
            &home.join(".pi/agent/settings.json"),
            "/home/agent/.pi/agent/settings.json",
            &home_text,
            &["defaultProjectTrust", "enableInstallTelemetry"],
            if pi_extensions { &[] } else { &["packages"] },
        )?;
    }
    Ok(())
}

fn pi_extensions_compatible(sandbox_name: &str, preserve_xdg_state: bool) -> Result<bool, String> {
    if let Ok(value) = env::var("SBXR_SYNC_PI_EXTENSIONS") {
        return match value.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err("SBXR_SYNC_PI_EXTENSIONS must be 0 or 1".to_owned()),
        };
    }
    let host = Command::new("pi").arg("--version").output().ok();
    let remote = process::sbx_command(preserve_xdg_state)
        .args(["exec", sandbox_name, "--", "pi", "--version"])
        .output()
        .ok();
    let version = |output: Option<&std::process::Output>| {
        output
            .filter(|output| output.status.success())
            .and_then(|output| std::str::from_utf8(&output.stdout).ok())
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_owned)
    };
    let host_version = version(host.as_ref());
    let remote_version = version(remote.as_ref());
    let compatible = host_version.is_some() && host_version == remote_version;
    if !compatible {
        eprintln!(
            "warning: Pi extensions/packages were not mirrored (host {}, sandbox {}); prompts, themes, and safe settings still sync",
            host_version.as_deref().unwrap_or("missing"),
            remote_version.as_deref().unwrap_or("missing")
        );
    }
    Ok(compatible)
}

fn existing(home: &Path, paths: &'static [&'static str]) -> impl Iterator<Item = &'static str> {
    paths
        .iter()
        .copied()
        .filter(|relative| home.join(relative).exists())
}

fn stream_tree(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
    selected: &[&str],
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
            archive.append(&home.join(relative), Path::new(relative), home_text)?;
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

fn merge_json_settings(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    host_path: &Path,
    remote_path: &str,
    host_home: &str,
    managed_keys: &[&str],
    excluded_host_keys: &[&str],
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
    let managed = remote_file(sandbox_name, preserve_xdg_state, remote_path)?
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| Value::Object(Map::new()));
    let mut merged = managed.clone();
    if let (Some(target), Some(source)) = (merged.as_object_mut(), host.as_object()) {
        for (key, value) in source {
            if !excluded_host_keys.contains(&key.as_str()) {
                target.insert(key.clone(), value.clone());
            }
        }
        if let Some(original) = managed.as_object() {
            for key in managed_keys {
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

fn remote_file(
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

fn upload_file(
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
}
