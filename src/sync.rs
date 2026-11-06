use crate::profile::{PromptStyle, ShellConfig, ShellProfile, ShellProgram};
use crate::{process, sbx, sha256};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

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
    ".claude/statusline.sh",
    ".claude/statusline-command.sh",
    ".claude/hooks",
    ".claude/commands",
    ".claude/agents",
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
    ".pi/agent/keybindings.json",
    ".pi/agent/provider-failover.json",
    ".pi/agent/skills",
    ".pi/agent/prompts",
    ".pi/agent/themes",
    ".pi/agent/npm/package.json",
];

const PI_EXTENSION_PATHS: &[&str] = &[".pi/agent/extensions"];

const HOST_SHELL_PROFILE_MARKER: &str = "/home/agent/.config/sbxr/host-shell-profile.sha256";
const HOST_SHELL_MIRROR_RELATIVE: &str = ".local/share/sbxr/host-shell";
const HOST_SHELL_MIRROR_ROOT: &str = "/home/agent/.local/share/sbxr/host-shell";

const REMOTE_RTK: &str = "/usr/local/bin/rtk";
const PI_NPM_MANIFEST_MARKER: &str = "/home/agent/.pi/agent/npm/.sbxr-manifest-sha256";
const PI_NPM_ROOT: &str = "/home/agent/.pi/agent/npm";
const BOOTSTRAP_STATE_PATH: &str = "/home/agent/.local/state/sbxr/bootstrap-v1.json";
const BOOTSTRAP_STATE_DIRECTORY: &str = "/home/agent/.local/state/sbxr";
const INSTALL_HOST_SHELL_MIRROR: &str = r#"
set -eu
root=$1
staging=$2
backup=$3
had_old=0
rm -rf -- "$backup"
if [ -e "$root" ] || [ -L "$root" ]; then
  chmod -R u+w -- "$root" 2>/dev/null || true
  mv -- "$root" "$backup"
  had_old=1
fi
if mv -- "$staging" "$root"; then
  if [ "$had_old" = 1 ]; then
    rm -rf -- "$backup"
  fi
else
  rm -rf -- "$root"
  if [ "$had_old" = 1 ]; then
    mv -- "$backup" "$root" || true
  fi
  exit 1
fi
"#;
static HOST_SHELL_STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const HOST_SHELL_LOCK_INITIALIZATION_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostShellLockState {
    Active,
    Initializing,
    Stale,
}

struct HostShellLock {
    path: PathBuf,
    pid: u32,
}

impl HostShellLock {
    fn acquire(state_root: &Path, sandbox_name: &str) -> Result<Self, String> {
        let directory = state_root.join("locks");
        fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "could not create the host-shell lock directory {}: {error}",
                directory.display()
            )
        })?;
        let metadata = fs::symlink_metadata(&directory).map_err(|error| {
            format!(
                "could not inspect the host-shell lock directory {}: {error}",
                directory.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "refusing unsafe host-shell lock directory {}",
                directory.display()
            ));
        }
        #[cfg(unix)]
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "could not protect the host-shell lock directory {}: {error}",
                directory.display()
            )
        })?;

        let key = sha256::hex(sandbox_name.as_bytes());
        let path = directory.join(format!("host-shell-{key}.lock"));
        let pid = std::process::id();
        for attempt in 0..600 {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(&path) {
                Ok(mut file) => {
                    if let Err(error) = writeln!(file, "{pid}").and_then(|()| file.flush()) {
                        let _ = fs::remove_file(&path);
                        return Err(format!(
                            "could not initialize host-shell lock {}: {error}",
                            path.display()
                        ));
                    }
                    return Ok(Self { path, pid });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    match host_shell_lock_state(&path, HOST_SHELL_LOCK_INITIALIZATION_GRACE)? {
                        HostShellLockState::Stale => match fs::remove_file(&path) {
                            Ok(()) => continue,
                            Err(error) if error.kind() == ErrorKind::NotFound => continue,
                            Err(error) => {
                                return Err(format!(
                                    "could not remove stale host-shell lock {}: {error}",
                                    path.display()
                                ));
                            }
                        },
                        HostShellLockState::Active | HostShellLockState::Initializing => {}
                    }
                    if attempt == 599 {
                        return Err(format!(
                            "timed out waiting for another sbxr process to finish shell reconciliation for {sandbox_name}"
                        ));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    return Err(format!(
                        "could not acquire host-shell lock {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        unreachable!("bounded host-shell lock loop always returns")
    }
}

impl Drop for HostShellLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
            == Some(self.pid)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn host_shell_lock_state(
    path: &Path,
    initialization_grace: Duration,
) -> Result<HostShellLockState, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(HostShellLockState::Stale),
        Err(error) => {
            return Err(format!(
                "could not read host-shell lock {}: {error}",
                path.display()
            ));
        }
    };
    if contents.trim().is_empty() {
        let age = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .unwrap_or_default();
        return Ok(if age < initialization_grace {
            HostShellLockState::Initializing
        } else {
            HostShellLockState::Stale
        });
    }
    let pid = contents.trim().parse::<u32>().map_err(|_| {
        format!(
            "refusing malformed host-shell lock {}; remove it after confirming that no sbxr process is running",
            path.display()
        )
    })?;
    if pid == std::process::id() {
        return Ok(HostShellLockState::Active);
    }
    Ok(if host_process_is_sbxr(pid)? {
        HostShellLockState::Active
    } else {
        HostShellLockState::Stale
    })
}

#[cfg(unix)]
fn host_process_is_sbxr(pid: u32) -> Result<bool, String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not inspect host-shell lock owner {pid}: {error}"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let command = String::from_utf8_lossy(&output.stdout);
    Ok(command
        .trim()
        .rsplit('/')
        .next()
        .is_some_and(|command| command.starts_with("sbxr")))
}

#[cfg(windows)]
fn host_process_is_sbxr(pid: u32) -> Result<bool, String> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not inspect host-shell lock owner {pid}: {error}"))?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .to_ascii_lowercase()
        .contains("sbxr"))
}

#[cfg(not(any(unix, windows)))]
fn host_process_is_sbxr(pid: u32) -> Result<bool, String> {
    Err(format!(
        "cannot inspect host-shell lock owner {pid} on this platform"
    ))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BootstrapState {
    pub(crate) locale: bool,
    pub(crate) git: bool,
    pub(crate) shell: bool,
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
        locale: value
            .get("locale")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        git: value.get("git").and_then(Value::as_bool).unwrap_or(false),
        shell: value.get("shell").and_then(Value::as_bool).unwrap_or(false),
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
        sbx::exec_command(preserve_xdg_state).args([
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
        "locale": state.locale,
        "git": state.git,
        "shell": state.shell,
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
        mirror_sanitized_json_config(
            sandbox_name,
            preserve_xdg_state,
            &home,
            &home_text,
            ".claude/settings.local.json",
        )?;
    }
    if pi {
        mirror_pi_models(sandbox_name, preserve_xdg_state, &home, &home_text)?;
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
                filter_credentials: true,
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
                filter_credentials: true,
            },
        )?;
    }
    Ok(())
}

fn mirror_sanitized_json_config(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
    relative: &str,
) -> Result<(), String> {
    let path = home.join(relative);
    if !path.is_file() {
        return Ok(());
    }
    let mut value: Value = serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    let removed = remove_json_credentials(&mut value);
    if removed > 0 {
        eprintln!(
            "warning: omitted {removed} credential-shaped field(s) while mirroring {relative}"
        );
    }
    replace_json_paths(&mut value, home_text);
    let bytes = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("could not serialize {}: {error}", path.display()))?;
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        &format!("/home/agent/{relative}"),
        &bytes,
    )
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

fn mirror_pi_models(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
) -> Result<(), String> {
    let path = home.join(".pi/agent/models.json");
    if !path.is_file() {
        return Ok(());
    }
    let (bytes, removed) = sanitized_pi_models(&path, home_text)?;
    if removed > 0 {
        eprintln!(
            "warning: omitted {removed} literal Pi model credential field(s) while mirroring host configuration"
        );
    }
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        "/home/agent/.pi/agent/models.json",
        &bytes,
    )
}

fn sanitized_pi_models(path: &Path, host_home: &str) -> Result<(Vec<u8>, usize), String> {
    let mut value: Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!(
            "could not mirror {}: expected a JSON object",
            path.display()
        ));
    }
    let mut removed = 0;
    remove_pi_model_credentials(&mut value, &mut removed);
    replace_json_paths(&mut value, host_home);
    let bytes = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("could not serialize {}: {error}", path.display()))?;
    Ok((bytes, removed))
}

fn remove_pi_model_credentials(value: &mut Value, removed: &mut usize) {
    match value {
        Value::Object(object) => {
            if object
                .get("apiKey")
                .is_some_and(|value| !safe_pi_config_reference(value))
            {
                object.remove("apiKey");
                *removed += 1;
            }
            if let Some(Value::Object(headers)) = object.get_mut("headers") {
                let before = headers.len();
                headers.retain(|_, value| safe_pi_config_reference(value));
                *removed += before - headers.len();
            }
            for child in object.values_mut() {
                remove_pi_model_credentials(child, removed);
            }
        }
        Value::Array(array) => {
            for child in array {
                remove_pi_model_credentials(child, removed);
            }
        }
        _ => {}
    }
}

fn safe_pi_config_reference(value: &Value) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let name = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'));
    name.is_some_and(|name| {
        !name.is_empty()
            && name.chars().enumerate().all(|(index, character)| {
                character == '_'
                    || character.is_ascii_alphanumeric()
                        && (index > 0 || !character.is_ascii_digit())
            })
    })
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
    let remote_version = command_version(sbx::exec_command(preserve_xdg_state).args([
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
    let installed_version = command_version(sbx::exec_command(preserve_xdg_state).args([
        sandbox_name,
        "--",
        REMOTE_RTK,
        "--version",
    ]));
    if host_version.is_none() || installed_version != host_version {
        let _ = sbx::exec_command(preserve_xdg_state)
            .args(["-u", "root", sandbox_name, "rm", "-f", REMOTE_RTK])
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
    let mut child = sbx::exec_command(preserve_xdg_state)
        .args(["-u", "root", "-i", sandbox_name, "tee", path])
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
        sbx::exec_command(preserve_xdg_state).args([
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
    let package_path = home.join(".pi/agent/npm/package.json");
    if !package_path.is_file() {
        return Ok(());
    }
    let package = fs::read(&package_path)
        .map_err(|error| format!("could not read {}: {error}", package_path.display()))?;
    let wanted = sha256::hex_prefix(&package, 32);
    let current = remote_file(sandbox_name, preserve_xdg_state, PI_NPM_MANIFEST_MARKER)?
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned());
    if current.as_deref() == Some(&wanted) {
        return Ok(());
    }
    // The host lockfile pins extension builds against the host's own Pi release.
    // The sandbox runs its kit-pinned Pi, so those exact versions can reference
    // APIs that release no longer exports and every `pi` start then fails to load
    // the extension. Resolve the host's declared semver ranges against the
    // sandbox Pi instead, and drop any lockfile left by an earlier sbxr version.
    process::run_checked(
        sbx::exec_command(preserve_xdg_state).args([
            sandbox_name,
            "--",
            "rm",
            "-f",
            "/home/agent/.pi/agent/npm/package-lock.json",
        ]),
        "clearing the stale sandbox Pi extension lockfile",
    )?;
    process::run_checked(
        sbx::exec_command(preserve_xdg_state).args([
            sandbox_name,
            "--",
            "npm",
            "install",
            "--legacy-peer-deps",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--prefix",
            PI_NPM_ROOT,
        ]),
        "installing host-declared Pi extension packages",
    )?;
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        PI_NPM_MANIFEST_MARKER,
        wanted.as_bytes(),
    )
}

pub(crate) fn host_shell_profile_current(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    shell: &ShellConfig,
    state_root: &Path,
) -> Result<bool, String> {
    let home = process::home_dir()?;
    let home_text = home.to_string_lossy();
    let inputs = host_shell_inputs(shell, &home, &home_text, state_root)?;
    let runtime = host_shell_runtime(shell, &inputs);
    let expected = host_shell_profile_digest(shell, &inputs, &runtime, &home)?;
    Ok(
        remote_file(sandbox_name, preserve_xdg_state, HOST_SHELL_PROFILE_MARKER)?
            .is_some_and(|value| value == expected.as_bytes()),
    )
}

pub(crate) fn host_shell(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    shell: &ShellConfig,
    state_root: &Path,
) -> Result<(), String> {
    let _lock = HostShellLock::acquire(state_root, sandbox_name)?;
    let home = process::home_dir()?;
    let home_text = home.to_string_lossy();
    let inputs = host_shell_inputs(shell, &home, &home_text, state_root)?;
    for warning in &inputs.warnings {
        eprintln!("warning: {warning}");
    }
    let runtime = host_shell_runtime(shell, &inputs);
    let digest = host_shell_profile_digest(shell, &inputs, &runtime, &home)?;
    replace_host_shell_mirror(
        sandbox_name,
        preserve_xdg_state,
        &home,
        &home_text,
        &inputs.selected,
    )?;
    remove_extracted_aliases(sandbox_name, preserve_xdg_state)?;
    if let (Some(contents), Some(path)) = (
        inputs.extracted_aliases.as_ref(),
        extracted_alias_path(shell, &inputs),
    ) {
        let file = format!(
            "# Managed by sbxr: alias definitions extracted from the trusted host shell RC.\n{contents}"
        );
        upload_file(
            sandbox_name,
            preserve_xdg_state,
            &format!("/home/agent/{path}"),
            file.as_bytes(),
        )?;
    }
    let extension = shell.program.name();
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        &format!("/home/agent/.config/sbxr/host-shell-runtime.{extension}"),
        runtime.as_bytes(),
    )?;
    upload_file(
        sandbox_name,
        preserve_xdg_state,
        HOST_SHELL_PROFILE_MARKER,
        digest.as_bytes(),
    )
}

fn extracted_alias_path(shell: &ShellConfig, inputs: &HostShellInputs) -> Option<&'static str> {
    inputs
        .extracted_aliases
        .as_ref()
        .map(|_| match shell.program {
            ShellProgram::Bash => ".config/sbxr/host-aliases.bash",
            ShellProgram::Zsh => ".config/sbxr/host-aliases.zsh",
            ShellProgram::Fish => ".config/sbxr/host-aliases.fish",
        })
}

fn host_shell_runtime(shell: &ShellConfig, inputs: &HostShellInputs) -> String {
    render_host_shell_runtime(
        shell,
        &inputs.alias_files,
        &inputs.bin_dirs,
        extracted_alias_path(shell, inputs),
    )
}

struct HostShellInputs {
    alias_files: Vec<String>,
    bin_dirs: Vec<String>,
    extracted_aliases: Option<String>,
    selected: Vec<PathBuf>,
    warnings: Vec<String>,
}

fn host_shell_inputs(
    shell: &ShellConfig,
    home: &Path,
    home_text: &str,
    state_root: &Path,
) -> Result<HostShellInputs, String> {
    let mut alias_files = if shell.profile == ShellProfile::Custom {
        shell.alias_files.clone()
    } else {
        Vec::new()
    };
    let mut bin_dirs = if shell.profile == ShellProfile::Custom {
        shell.bin_dirs.clone()
    } else {
        Vec::new()
    };
    let mut implicit_alias_files = BTreeSet::new();
    let mut implicit_bin_dirs = BTreeSet::new();
    let mut extracted_aliases = None;
    if shell.profile == ShellProfile::Familiar {
        let (rc, aliases) = match shell.program {
            ShellProgram::Bash => (".bashrc", ".bash_aliases"),
            ShellProgram::Zsh => (".zshrc", ".zsh_aliases"),
            ShellProgram::Fish => (
                ".config/fish/config.fish",
                ".config/fish/conf.d/aliases.fish",
            ),
        };
        if home.join(aliases).exists() && !alias_files.iter().any(|value| value == aliases) {
            alias_files.insert(0, aliases.to_owned());
            implicit_alias_files.insert(aliases.to_owned());
        }
        if home.join("bin").exists() && !bin_dirs.iter().any(|value| value == "bin") {
            bin_dirs.insert(0, "bin".to_owned());
            implicit_bin_dirs.insert("bin".to_owned());
        }
        extracted_aliases = extract_host_aliases(&home.join(rc), shell.program)?;
    }

    // Top-level selections may be symlinks (stow, chezmoi and dotfiles repos
    // link `~/.bash_aliases` or `~/bin`); the mirror copies what they point
    // at. Both the name and the resolved target are checked against the
    // protected stores so a link cannot smuggle one in.
    let mut selected = BTreeSet::new();
    let mut warnings = Vec::new();
    for (relative, want_dir, implicit) in alias_files
        .iter()
        .map(|relative| (relative, false, implicit_alias_files.contains(relative)))
        .chain(
            shell
                .prompt_file
                .iter()
                .map(|relative| (relative, false, false)),
        )
        .chain(
            bin_dirs
                .iter()
                .map(|relative| (relative, true, implicit_bin_dirs.contains(relative))),
        )
    {
        let source = home.join(relative);
        let kind = if want_dir {
            "executable directory"
        } else {
            "shell file"
        };
        let validation = (|| {
            crate::profile::validate_shell_mirror_path(relative, home, state_root)?;
            let metadata = fs::metadata(&source).map_err(|error| {
                format!(
                    "selected host {kind} is unavailable: {}: {error}",
                    source.display()
                )
            })?;
            if (want_dir && !metadata.is_dir()) || (!want_dir && !metadata.is_file()) {
                return Err(format!(
                    "selected host {kind} must resolve to a {}: {}",
                    if want_dir {
                        "directory"
                    } else {
                        "regular file"
                    },
                    source.display()
                ));
            }
            crate::profile::validate_resolved_shell_path(relative, home, state_root)
        })();
        if let Err(error) = validation {
            if implicit {
                warnings.push(format!(
                    "skipping automatically discovered host {kind} '{}': {error}",
                    source.display()
                ));
                continue;
            }
            return Err(error);
        }
        selected.insert(PathBuf::from(relative));
    }
    alias_files.retain(|relative| selected.contains(Path::new(relative)));
    bin_dirs.retain(|relative| selected.contains(Path::new(relative)));
    let selected = selected.into_iter().collect::<Vec<_>>();
    let extracted_aliases =
        extracted_aliases.map(|aliases| rewrite_shell_paths(&aliases, home_text, &selected));
    Ok(HostShellInputs {
        alias_files,
        bin_dirs,
        extracted_aliases,
        selected,
        warnings,
    })
}

/// Copies alias definitions out of a host shell RC without adopting the rest
/// of it. Toolchain setup, host paths, and host-only integrations do not belong
/// in a sandbox unless the user selects a dedicated file explicitly.
fn extract_host_aliases(rc: &Path, program: ShellProgram) -> Result<Option<String>, String> {
    if !rc.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(rc)
        .map_err(|error| format!("could not read {}: {error}", rc.display()))?;
    let aliases = match program {
        ShellProgram::Fish => fish_alias_lines(&contents),
        ShellProgram::Bash | ShellProgram::Zsh => shell_alias_lines(&contents),
    };
    if aliases.is_empty() {
        return Ok(None);
    }
    Ok(Some(aliases))
}

/// Rewrites host paths in mirrored shell text. References to a mirrored path
/// (`/host/home/bin/x`, `~/bin/x`, `$HOME/bin/x`, `${HOME}/bin/x`) point at the
/// launcher-owned mirror root; any other absolute host-home path maps to
/// `/home/agent`, as for every other mirrored capability.
fn rewrite_shell_paths(text: &str, host_home: &str, mirrored: &[PathBuf]) -> String {
    let mut relatives = mirrored
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    relatives.sort_by_key(|relative| std::cmp::Reverse(relative.len()));
    // Quoted forms first: `"$HOME"/bin/x` keeps its quotes and gains the
    // mirror segment after them.
    let prefixes = [
        host_home,
        "\"${HOME}\"",
        "\"$HOME\"",
        "${HOME}",
        "$HOME",
        "~",
    ];
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    'scan: while index < text.len() {
        for prefix in prefixes {
            if prefix.is_empty()
                || !text[index..].starts_with(prefix)
                || !prefix_boundary_before(text, index, prefix, host_home)
            {
                continue;
            }
            let after = index + prefix.len();
            for relative in &relatives {
                let needle = format!("/{relative}");
                let end = after + needle.len();
                if text[after..].starts_with(&needle) && path_boundary_after(text, end) {
                    output.push_str(if prefix == host_home {
                        "/home/agent"
                    } else {
                        prefix
                    });
                    output.push('/');
                    output.push_str(HOST_SHELL_MIRROR_RELATIVE);
                    output.push_str(&needle);
                    index = end;
                    continue 'scan;
                }
            }
            if prefix == host_home && path_boundary_after(text, after) {
                output.push_str("/home/agent");
                index = after;
                continue 'scan;
            }
        }
        let character = text[index..].chars().next().unwrap_or_default();
        output.push(character);
        index += character.len_utf8().max(1);
    }
    output
}

fn prefix_boundary_before(text: &str, index: usize, prefix: &str, host_home: &str) -> bool {
    let previous = text[..index].chars().next_back();
    if prefix == "~" {
        previous
            .is_none_or(|character| !(character.is_alphanumeric() || "._-/~$".contains(character)))
    } else if prefix == host_home {
        // `/data/home/emi` must not match a host home of `/home/emi`.
        previous
            .is_none_or(|character| !(character.is_alphanumeric() || "._-/".contains(character)))
    } else {
        // `\$HOME` is an escaped literal, not the variable.
        previous != Some('\\')
    }
}

fn path_boundary_after(text: &str, index: usize) -> bool {
    text[index..].chars().next().is_none_or(|character| {
        character == '/' || !(character.is_alphanumeric() || "._-".contains(character))
    })
}

fn fish_alias_lines(contents: &str) -> String {
    contents
        .lines()
        .filter(|line| line.starts_with("alias "))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn shell_alias_lines(contents: &str) -> String {
    let mut output = String::new();
    let mut continued = false;
    for line in contents.lines() {
        let starts_alias = line.trim_start().starts_with("alias ");
        if !continued && !starts_alias {
            continue;
        }
        output.push_str(line);
        output.push('\n');
        continued = line.ends_with('\\');
    }
    output
}

fn host_shell_profile_digest(
    shell: &ShellConfig,
    inputs: &HostShellInputs,
    runtime: &str,
    home: &Path,
) -> Result<String, String> {
    let mut input = b"sbxr-host-shell-v4\0".to_vec();
    for part in [
        shell.profile.name(),
        shell.program.name(),
        shell.prompt.name(),
        shell.prompt_file.as_deref().unwrap_or(""),
    ] {
        input.extend_from_slice(part.as_bytes());
        input.push(0);
    }
    for values in [&shell.alias_files, &shell.bin_dirs] {
        for value in values {
            input.extend_from_slice(value.as_bytes());
            input.push(0);
        }
        input.push(0xff);
    }
    if let Some(aliases) = &inputs.extracted_aliases {
        input.extend_from_slice(aliases.as_bytes());
    }
    input.push(0xff);
    // The rendered overlay is part of the digest, so a launcher release that
    // changes it refreshes existing sandboxes without a manual tag bump.
    input.extend_from_slice(runtime.as_bytes());
    input.push(0xff);
    for relative in &inputs.selected {
        fingerprint_shell_tree(&home.join(relative), relative, &mut input)?;
    }
    Ok(sha256::hex(&input))
}

fn replace_host_shell_mirror(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
    selected: &[PathBuf],
) -> Result<(), String> {
    // Never delete a user-selected guest path. A host selection such as
    // `.cargo/bin` or `.local/bin` may collide with sandbox-managed tools.
    // All mirrored shell inputs therefore live below one launcher-owned root.
    // They are staged next to it and swapped in, so a failed copy leaves the
    // previous mirror in place.
    let token = format!(
        "{}-{}",
        std::process::id(),
        HOST_SHELL_STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let staging = format!("{HOST_SHELL_MIRROR_ROOT}.staging-{token}");
    let backup = format!("{HOST_SHELL_MIRROR_ROOT}.backup-{token}");
    process::run_checked(
        sbx::exec_command(preserve_xdg_state).args([
            sandbox_name,
            "--",
            "sh",
            "-c",
            "set -eu; for path in \"$2\".staging-* \"$2\".backup-*; do if [ -e \"$path\" ] || [ -L \"$path\" ]; then chmod -R u+w -- \"$path\" 2>/dev/null || true; rm -rf -- \"$path\"; fi; done; mkdir -p -- \"$1\"",
            "sh",
            &staging,
            HOST_SHELL_MIRROR_ROOT,
        ]),
        "preparing the managed host shell mirror",
    )?;
    if !selected.is_empty() {
        if let Err(error) = stream_shell_tree(
            sandbox_name,
            preserve_xdg_state,
            home,
            home_text,
            selected,
            &staging,
        ) {
            let _ = process::run_checked(
                sbx::exec_command(preserve_xdg_state).args([
                    sandbox_name,
                    "--",
                    "rm",
                    "-rf",
                    "--",
                    &staging,
                ]),
                "cleaning an incomplete host shell mirror",
            );
            return Err(error);
        }
    }
    process::run_checked(
        sbx::exec_command(preserve_xdg_state).args([
            sandbox_name,
            "--",
            "sh",
            "-c",
            INSTALL_HOST_SHELL_MIRROR,
            "sh",
            HOST_SHELL_MIRROR_ROOT,
            &staging,
            &backup,
        ]),
        "installing the managed host shell mirror",
    )
}

fn remove_extracted_aliases(sandbox_name: &str, preserve_xdg_state: bool) -> Result<(), String> {
    process::run_checked(
        sbx::exec_command(preserve_xdg_state)
            .arg(sandbox_name)
            .args([
                "--",
                "rm",
                "-f",
                "--",
                "/home/agent/.config/sbxr/host-aliases.bash",
                "/home/agent/.config/sbxr/host-aliases.zsh",
                "/home/agent/.config/sbxr/host-aliases.fish",
            ]),
        "clearing stale extracted host aliases",
    )
}

fn fingerprint_shell_tree(
    source: &Path,
    archive: &Path,
    input: &mut Vec<u8>,
) -> Result<(), String> {
    let root = fs::canonicalize(source)
        .map_err(|error| format!("could not resolve {}: {error}", source.display()))?;
    // Start from the resolved root: a symlinked selection is mirrored by
    // content, so its target's contents are what must be fingerprinted.
    fingerprint_shell_entry(&root, archive, &root, input)
}

fn fingerprint_shell_entry(
    source: &Path,
    archive: &Path,
    root: &Path,
    input: &mut Vec<u8>,
) -> Result<(), String> {
    if should_skip(archive, true) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("could not inspect {}: {error}", source.display()))?;
    input.extend_from_slice(archive.to_string_lossy().as_bytes());
    input.push(0);
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = u32::from(metadata.permissions().readonly());
    input.extend_from_slice(
        format!(
            "{}:{}:{}:{}:{mode:o}\0",
            metadata.is_dir(),
            metadata.is_file(),
            metadata.len(),
            modified.as_nanos()
        )
        .as_bytes(),
    );
    if metadata.file_type().is_symlink() {
        // Hash the link text whether or not it resolves. The archive writer
        // skips broken and escaping links, so neither may fail the launch.
        let target = fs::read_link(source)
            .map_err(|error| format!("could not read symlink {}: {error}", source.display()))?;
        input.extend_from_slice(target.to_string_lossy().as_bytes());
        input.push(0);
        if let Ok(resolved) = fs::canonicalize(source) {
            input.push(u8::from(resolved.starts_with(root)));
        }
    } else if metadata.is_dir() {
        let mut entries = fs::read_dir(source)
            .map_err(|error| format!("could not read {}: {error}", source.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not read {}: {error}", source.display()))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            fingerprint_shell_entry(&entry.path(), &archive.join(entry.file_name()), root, input)?;
        }
    }
    Ok(())
}

fn render_host_shell_runtime(
    shell: &ShellConfig,
    aliases: &[String],
    bins: &[String],
    extracted: Option<&str>,
) -> String {
    let mut output = String::from("# Managed by sbxr from the protected shell profile.\n");
    match shell.program {
        ShellProgram::Bash | ShellProgram::Zsh => {
            for directory in bins {
                let path = mirrored_shell_expression(directory);
                output.push_str(&format!(
                    "__sbxr_path={path}; if [ -d \"$__sbxr_path\" ]; then case \":$PATH:\" in *\":$__sbxr_path:\"*) ;; *) PATH=\"$PATH:$__sbxr_path\"; export PATH ;; esac; fi; unset __sbxr_path\n"
                ));
            }
            if let Some(path) = extracted {
                output.push_str(&format!(
                    "[ -f {} ] && . {} || true\n",
                    shell_home_expression(path),
                    shell_home_expression(path)
                ));
            }
            for path in aliases {
                output.push_str(&format!(
                    "[ -f {} ] && . {} || true\n",
                    mirrored_shell_expression(path),
                    mirrored_shell_expression(path)
                ));
            }
            if shell.prompt == PromptStyle::Plain {
                match shell.program {
                    ShellProgram::Bash => output.push_str("PS1='\\u@\\h:\\w\\$ '\n"),
                    ShellProgram::Zsh => output.push_str("PROMPT='%n@%m:%~%# '\n"),
                    ShellProgram::Fish => unreachable!(),
                }
            } else if let Some(path) = shell.prompt_file.as_deref() {
                output.push_str(&format!(
                    "[ -f {} ] && . {} || true\n",
                    mirrored_shell_expression(path),
                    mirrored_shell_expression(path)
                ));
            }
        }
        ShellProgram::Fish => {
            for directory in bins {
                let path = mirrored_shell_expression(directory);
                output.push_str(&format!(
                    "set -l __sbxr_path {path}; if test -d \"$__sbxr_path\"; if not contains -- \"$__sbxr_path\" $PATH; set -gx PATH $PATH \"$__sbxr_path\"; end; end\n"
                ));
            }
            if let Some(path) = extracted {
                output.push_str(&format!(
                    "if test -f {}; source {}; end\n",
                    shell_home_expression(path),
                    shell_home_expression(path)
                ));
            }
            for path in aliases {
                output.push_str(&format!(
                    "if test -f {}; source {}; end\n",
                    mirrored_shell_expression(path),
                    mirrored_shell_expression(path)
                ));
            }
            if shell.prompt == PromptStyle::Plain {
                output.push_str("function fish_prompt; printf '%s@%s:%s> ' $USER (prompt_hostname) (prompt_pwd); end\n");
            } else if let Some(path) = shell.prompt_file.as_deref() {
                output.push_str(&format!(
                    "if test -f {}; source {}; end\n",
                    mirrored_shell_expression(path),
                    mirrored_shell_expression(path)
                ));
            }
        }
    }
    output
}

fn shell_home_expression(relative: &str) -> String {
    format!("\"$HOME\"/{}", shell_single_quote(relative))
}

fn mirrored_shell_expression(relative: &str) -> String {
    shell_home_expression(&format!("{HOST_SHELL_MIRROR_RELATIVE}/{relative}"))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
    stream_tree_with_policy(
        sandbox_name,
        preserve_xdg_state,
        home,
        home_text,
        selected,
        MirrorPolicy::capabilities(),
        "/home/agent",
    )
}

fn stream_shell_tree(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
    selected: &[PathBuf],
    destination: &str,
) -> Result<(), String> {
    stream_tree_with_policy(
        sandbox_name,
        preserve_xdg_state,
        home,
        home_text,
        selected,
        MirrorPolicy::host_shell(),
        destination,
    )
}

fn stream_tree_with_policy(
    sandbox_name: &str,
    preserve_xdg_state: bool,
    home: &Path,
    home_text: &str,
    selected: &[PathBuf],
    policy: MirrorPolicy,
    destination: &str,
) -> Result<(), String> {
    let mut child = sbx::exec_command(preserve_xdg_state)
        .arg("-i")
        .arg(sandbox_name)
        .args(["tar", "-xf", "-", "-C", destination])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start remote capability extraction: {error}"))?;
    {
        let output = child
            .stdin
            .as_mut()
            .ok_or("remote capability extraction stdin was unavailable")?;
        let mut archive = TarWriter::with_policy(output, policy);
        if policy.rewrite_into_shell_mirror {
            archive.shell_mirror = selected.to_vec();
        }
        for relative in selected {
            archive.append_tree(&home.join(relative), relative, home_text)?;
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
    filter_credentials: bool,
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
    if policy.filter_credentials {
        let removed = remove_json_credentials(&mut host);
        if removed > 0 {
            eprintln!(
                "warning: omitted {removed} credential-shaped field(s) while mirroring {}",
                host_path.display()
            );
        }
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

fn remove_json_credentials(value: &mut Value) -> usize {
    match value {
        Value::Object(object) => {
            let sensitive = object
                .iter()
                .filter(|(key, value)| credential_field(key, value))
                .map(|(key, _)| key)
                .cloned()
                .collect::<Vec<_>>();
            let mut removed = sensitive.len();
            for key in sensitive {
                object.remove(&key);
            }
            for child in object.values_mut() {
                removed += remove_json_credentials(child);
            }
            removed
        }
        Value::Array(array) => array.iter_mut().map(remove_json_credentials).sum(),
        _ => 0,
    }
}

fn credential_field(key: &str, value: &Value) -> bool {
    let compact = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let exact = matches!(
        compact.as_str(),
        "access"
            | "refresh"
            | "authorization"
            | "cookie"
            | "token"
            | "tokens"
            | "secret"
            | "secrets"
            | "password"
            | "passwords"
            | "apikey"
            | "apikeys"
            | "privatekey"
            | "privatekeys"
            | "credential"
            | "credentials"
    );
    let singular = [
        "token",
        "secret",
        "password",
        "apikey",
        "privatekey",
        "cookie",
    ]
    .iter()
    .any(|suffix| compact.ends_with(suffix) && !compact.ends_with(&format!("{suffix}s")));
    let plural = [
        "secrets",
        "passwords",
        "apikeys",
        "privatekeys",
        "credentials",
    ]
    .iter()
    .any(|suffix| compact.ends_with(suffix) && compact.len() > suffix.len())
        || ["tokens", "cookies"].iter().any(|suffix| {
            compact
                .strip_suffix(suffix)
                .is_some_and(credential_qualifier)
        });
    exact
        || singular
        || (plural && matches!(value, Value::String(_) | Value::Array(_) | Value::Object(_)))
}

fn credential_qualifier(prefix: &str) -> bool {
    [
        "access", "api", "auth", "bearer", "client", "id", "identity", "oauth", "refresh",
        "session",
    ]
    .iter()
    .any(|qualifier| prefix.ends_with(qualifier))
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
    let status = sbx::exec_command(preserve_xdg_state)
        .args([
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
    let output = sbx::exec_command(preserve_xdg_state)
        .args([sandbox_name, "cat", path])
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
    let mut child = sbx::exec_command(preserve_xdg_state)
        .args(["-i", sandbox_name, "tee", path])
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
        sbx::exec_command(preserve_xdg_state).args([sandbox_name, "chmod", "600", path]),
        "setting remote configuration permissions",
    )
}

const MAX_SYMLINK_DEREFERENCE_DEPTH: u32 = 32;

#[derive(Clone, Copy)]
struct MirrorPolicy {
    follow_external_symlinks: bool,
    skip_any_git: bool,
    rewrite_into_shell_mirror: bool,
}

impl MirrorPolicy {
    fn capabilities() -> Self {
        Self {
            follow_external_symlinks: true,
            skip_any_git: false,
            rewrite_into_shell_mirror: false,
        }
    }

    fn host_shell() -> Self {
        Self {
            follow_external_symlinks: false,
            skip_any_git: true,
            rewrite_into_shell_mirror: true,
        }
    }
}

struct TarWriter<'a, W: Write> {
    output: &'a mut W,
    root: PathBuf,
    policy: MirrorPolicy,
    /// Selections that live under the host-shell mirror root; host paths that
    /// reference them are rewritten to the mirror instead of `/home/agent`.
    shell_mirror: Vec<PathBuf>,
    archive_root: PathBuf,
}

impl<'a, W: Write> TarWriter<'a, W> {
    #[cfg(test)]
    fn new(output: &'a mut W) -> Self {
        Self::with_policy(output, MirrorPolicy::capabilities())
    }

    fn with_policy(output: &'a mut W, policy: MirrorPolicy) -> Self {
        Self {
            output,
            root: PathBuf::new(),
            policy,
            shell_mirror: Vec::new(),
            archive_root: PathBuf::new(),
        }
    }

    fn rewrite(&self, text: &str, host_home: &str) -> String {
        if self.policy.rewrite_into_shell_mirror {
            rewrite_shell_paths(text, host_home, &self.shell_mirror)
        } else {
            text.replace(host_home, "/home/agent")
        }
    }

    /// Appends one mirrored top-level selection. Symlinks are preserved only
    /// while they resolve inside this selection; the sandbox has no copy of
    /// anything outside it, so those links are followed instead.
    fn append_tree(
        &mut self,
        source: &Path,
        archive: &Path,
        host_home: &str,
    ) -> Result<(), String> {
        self.root = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
        self.archive_root = archive.to_path_buf();
        if self.policy.rewrite_into_shell_mirror {
            // A symlinked shell selection is mirrored by content: the guest has
            // no copy of the host path the link names.
            let root = self.root.clone();
            return self.append(&root, archive, host_home, 0);
        }
        self.append(source, archive, host_home, 0)
    }

    fn append(
        &mut self,
        source: &Path,
        archive: &Path,
        host_home: &str,
        depth: u32,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(source)
            .map_err(|error| format!("could not inspect {}: {error}", source.display()))?;
        if should_skip(archive, self.policy.skip_any_git) {
            return Ok(());
        }
        if metadata.file_type().is_symlink() {
            let Ok(resolved) = fs::canonicalize(source) else {
                // The host link is already broken; there is nothing to mirror.
                return Ok(());
            };
            if !resolved.starts_with(&self.root) {
                if !self.policy.follow_external_symlinks {
                    eprintln!(
                        "warning: skipped host shell symlink that leaves its selected tree: {} -> {}",
                        source.display(),
                        resolved.display()
                    );
                    return Ok(());
                }
                // A link that leaves this mirrored tree has no valid sandbox-side
                // target, so copy what it points at under the link's own name.
                if depth >= MAX_SYMLINK_DEREFERENCE_DEPTH {
                    return Err(format!(
                        "refusing to follow deeply nested mirrored symlinks at {}",
                        source.display()
                    ));
                }
                return self.append(&resolved, archive, host_home, depth + 1);
            }
            let target = if self.policy.rewrite_into_shell_mirror {
                // Emit in-tree links relative to the link itself, computed from
                // the resolved target. That keeps them valid at any mirror
                // root, short enough for ustar's 100-byte link field, and
                // correct even when the host home is itself reached via a link.
                let inside = resolved.strip_prefix(&self.root).unwrap_or(Path::new(""));
                relative_link(
                    archive.parent().unwrap_or(Path::new("")),
                    &self.archive_root.join(inside),
                )
            } else {
                let target = fs::read_link(source).map_err(|error| {
                    format!("could not read symlink {}: {error}", source.display())
                })?;
                self.rewrite(&target.to_string_lossy(), host_home)
            };
            self.header(archive, &metadata, 0, b'2', Some(&target))?;
        } else if metadata.is_dir() {
            self.header(archive, &metadata, 0, b'5', None)?;
            let mut entries: Vec<_> = fs::read_dir(source)
                .map_err(|error| format!("could not read {}: {error}", source.display()))?
                .filter_map(Result::ok)
                .collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                self.append(
                    &entry.path(),
                    &archive.join(entry.file_name()),
                    host_home,
                    depth,
                )?;
            }
        } else if metadata.is_file() {
            let mut contents = Vec::new();
            File::open(source)
                .and_then(|mut file| file.read_to_end(&mut contents))
                .map_err(|error| format!("could not read {}: {error}", source.display()))?;
            if let Ok(text) = std::str::from_utf8(&contents) {
                contents = self.rewrite(text, host_home).into_bytes();
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

/// Relative symlink text that reaches `target` from directory `from`; both are
/// archive paths below the same extraction root.
fn relative_link(from: &Path, target: &Path) -> String {
    let from = from.components().collect::<Vec<_>>();
    let target = target.components().collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec![".."; from.len() - common];
    let rest = target[common..]
        .iter()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    parts.extend(rest.iter().map(String::as_str));
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

fn should_skip(path: &Path, skip_any_git: bool) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    text == ".codex/skills/.system"
        || text.starts_with(".codex/skills/.system/")
        || text == "bin/.git"
        || text.starts_with("bin/.git/")
        || (skip_any_git && text.split('/').any(|component| component == ".git"))
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
    fn host_shell_lock_serializes_reconciliation_and_cleans_up() {
        use std::sync::mpsc;
        use std::time::{Duration, SystemTime};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-host-shell-lock-{}-{unique}",
            std::process::id()
        ));
        let first = HostShellLock::acquire(&root, "sandbox").unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let contender_root = root.clone();
        let contender = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _lock = HostShellLock::acquire(&contender_root, "sandbox").unwrap();
            acquired_tx.send(()).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        contender.join().unwrap();
        assert_eq!(fs::read_dir(root.join("locks")).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_shell_lock_retries_an_empty_initializing_file() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-host-shell-empty-lock-{}-{unique}",
            std::process::id()
        ));
        let directory = root.join("locks");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("host-shell-{}.lock", sha256::hex(b"sandbox")));
        fs::write(&path, b"").unwrap();
        assert_eq!(
            host_shell_lock_state(&path, HOST_SHELL_LOCK_INITIALIZATION_GRACE).unwrap(),
            HostShellLockState::Initializing
        );
        assert_eq!(
            host_shell_lock_state(&path, Duration::ZERO).unwrap(),
            HostShellLockState::Stale
        );

        let contender_path = path.clone();
        let remover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            fs::remove_file(contender_path).unwrap();
        });
        let lock = HostShellLock::acquire(&root, "sandbox").unwrap();
        remover.join().unwrap();
        drop(lock);

        fs::write(&path, b"not-a-pid\n").unwrap();
        assert!(host_shell_lock_state(&path, HOST_SHELL_LOCK_INITIALIZATION_GRACE).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_shell_install_restores_the_previous_mirror_on_failure() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "sbxr-host-shell-install-{}-{unique}",
            std::process::id()
        ));
        let root = directory.join("mirror");
        let staging = directory.join("staging");
        let backup = directory.join("backup");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("content"), b"previous").unwrap();

        let failed = Command::new("sh")
            .args(["-c", INSTALL_HOST_SHELL_MIRROR, "sh"])
            .arg(&root)
            .arg(&staging)
            .arg(&backup)
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!failed.success());
        assert_eq!(fs::read(root.join("content")).unwrap(), b"previous");
        assert!(!backup.exists());

        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("content"), b"replacement").unwrap();
        let installed = Command::new("sh")
            .args(["-c", INSTALL_HOST_SHELL_MIRROR, "sh"])
            .arg(&root)
            .arg(&staging)
            .arg(&backup)
            .status()
            .unwrap();
        assert!(installed.success());
        assert_eq!(fs::read(root.join("content")).unwrap(), b"replacement");
        assert!(!backup.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dereferences_mirrored_links_that_leave_their_tree() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("sbxr-sync-symlink-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let tree = root.join("tree");
        fs::create_dir_all(&tree).unwrap();
        fs::write(root.join("outside.txt"), b"outside payload").unwrap();
        fs::write(tree.join("real.txt"), b"inside payload").unwrap();
        symlink("real.txt", tree.join("inside-link")).unwrap();
        symlink(root.join("outside.txt"), tree.join("escaping-link")).unwrap();
        symlink(root.join("missing.txt"), tree.join("broken-link")).unwrap();

        let mut buffer = Vec::new();
        {
            let mut archive = TarWriter::new(&mut buffer);
            archive
                .append_tree(&tree, Path::new("tree"), &root.to_string_lossy())
                .unwrap();
            archive.finish().unwrap();
        }
        let _ = fs::remove_dir_all(&root);

        let mut entries = HashMap::new();
        let mut offset = 0;
        while offset + 512 <= buffer.len() {
            let header = &buffer[offset..offset + 512];
            let name = String::from_utf8_lossy(&header[0..100])
                .trim_end_matches('\0')
                .to_owned();
            if name.is_empty() {
                break;
            }
            let size = usize::from_str_radix(
                String::from_utf8_lossy(&header[124..135])
                    .trim_end_matches('\0')
                    .trim(),
                8,
            )
            .unwrap();
            entries.insert(name, header[156]);
            offset += 512 + size.div_ceil(512) * 512;
        }

        // A link that stays inside the mirrored tree still resolves in the sandbox.
        assert_eq!(entries.get("tree/inside-link"), Some(&b'2'));
        // One that escapes it does not, so its content is copied under the link name.
        assert_eq!(entries.get("tree/escaping-link"), Some(&b'0'));
        // A link the host itself cannot resolve is skipped entirely.
        assert_eq!(entries.get("tree/broken-link"), None);
    }

    #[cfg(unix)]
    #[test]
    fn host_shell_mirror_skips_links_that_leave_the_selected_tree() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-secure-shell-symlink-{}-{unique}",
            std::process::id()
        ));
        let tree = root.join("bin");
        fs::create_dir_all(&tree).unwrap();
        fs::write(root.join("secret"), b"do not copy").unwrap();
        symlink(root.join("secret"), tree.join("escaping-link")).unwrap();

        let mut buffer = Vec::new();
        {
            let mut archive = TarWriter::with_policy(&mut buffer, MirrorPolicy::host_shell());
            archive
                .append_tree(&tree, Path::new("bin"), &root.to_string_lossy())
                .unwrap();
            archive.finish().unwrap();
        }
        assert!(
            !buffer
                .windows(b"do not copy".len())
                .any(|value| value == b"do not copy")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keeps_only_alias_definitions_from_the_host_bashrc() {
        let bashrc = concat!(
            "export PATH=$PATH:/host/only/bin\n",
            "# some more ls aliases\n",
            "alias ll='ls -alF'\n",
            "  alias grep='grep --color=auto'\n",
            "alias wrapped='echo one \\\n",
            "  two'\n",
            "source /host/only/tool.sh\n",
        );
        assert_eq!(
            shell_alias_lines(bashrc),
            concat!(
                "alias ll='ls -alF'\n",
                "  alias grep='grep --color=auto'\n",
                "alias wrapped='echo one \\\n",
                "  two'\n",
            )
        );
        assert!(shell_alias_lines("export PATH=/host\n").is_empty());
    }

    #[test]
    fn fish_alias_extraction_keeps_only_top_level_one_line_aliases() {
        let config = concat!(
            "alias ll 'ls -la'\n",
            "function nested\n",
            "  alias private 'echo no'\n",
            "end\n",
            "set -gx PATH /host/bin $PATH\n",
        );
        assert_eq!(fish_alias_lines(config), "alias ll 'ls -la'\n");
    }

    #[test]
    fn renders_shell_specific_runtime_overlays() {
        let bash = ShellConfig {
            alias_files: vec![".config/custom aliases".to_owned()],
            bin_dirs: vec!["scripts".to_owned()],
            prompt: PromptStyle::Plain,
            ..ShellConfig::default()
        };
        let rendered = render_host_shell_runtime(&bash, &bash.alias_files, &bash.bin_dirs, None);
        assert!(rendered.contains("$HOME\"/'.local/share/sbxr/host-shell/scripts'"));
        assert!(rendered.contains(".config/custom aliases"));
        assert!(rendered.contains("PS1="));

        let fish = ShellConfig {
            program: ShellProgram::Fish,
            prompt: PromptStyle::Plain,
            ..ShellConfig::default()
        };
        let aliases = [
            ".config/fish/conf.d/aliases.fish".to_owned(),
            ".config/fish/manual-aliases.fish".to_owned(),
        ];
        let rendered = render_host_shell_runtime(&fish, &aliases, &[], None);
        assert!(rendered.contains("function fish_prompt"));
        assert!(!rendered.contains("PS1="));
        // Mirrored files live outside ~/.config/fish, so fish never
        // auto-sources them; the runtime overlay must source each one.
        assert!(rendered.contains("host-shell/.config/fish/conf.d/aliases.fish"));
        assert!(rendered.contains("manual-aliases.fish"));

        let minimal = ShellConfig {
            profile: ShellProfile::Minimal,
            prompt: PromptStyle::Plain,
            ..ShellConfig::default()
        };
        let rendered = render_host_shell_runtime(&minimal, &[], &[], None);
        assert!(!rendered.contains("__sbxr_path"));
        assert!(!rendered.contains("host-aliases"));
        assert!(!rendered.contains(".bash_aliases"));
    }

    #[test]
    fn familiar_inputs_are_shell_specific_and_content_sensitive() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("sbxr-shell-inputs-{}-{unique}", std::process::id()));
        fs::create_dir_all(home.join(".config/fish/conf.d")).unwrap();
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::write(home.join(".bashrc"), b"alias b='echo bash'\n").unwrap();
        fs::write(home.join(".bash_aliases"), b"alias bb='echo bash'\n").unwrap();
        fs::write(home.join(".zshrc"), b"alias z='echo zsh'\n").unwrap();
        fs::write(home.join(".zsh_aliases"), b"alias zz='echo zsh'\n").unwrap();
        fs::write(
            home.join(".config/fish/config.fish"),
            b"alias f 'echo fish'\n",
        )
        .unwrap();
        fs::write(
            home.join(".config/fish/conf.d/aliases.fish"),
            b"alias ff 'echo fish'\n",
        )
        .unwrap();
        fs::write(home.join("bin/tool"), b"one").unwrap();
        let state = std::env::temp_dir().join("sbxr-state-outside-test-home");

        for (program, expected) in [
            (ShellProgram::Bash, ".bash_aliases"),
            (ShellProgram::Zsh, ".zsh_aliases"),
            (ShellProgram::Fish, ".config/fish/conf.d/aliases.fish"),
        ] {
            let shell = ShellConfig {
                program,
                ..ShellConfig::default()
            };
            let inputs = host_shell_inputs(&shell, &home, &home.to_string_lossy(), &state).unwrap();
            assert_eq!(inputs.alias_files, [expected]);
            assert_eq!(inputs.bin_dirs, ["bin"]);
            assert!(inputs.extracted_aliases.is_some());
        }

        let shell = ShellConfig::default();
        let first_inputs =
            host_shell_inputs(&shell, &home, &home.to_string_lossy(), &state).unwrap();
        let first = host_shell_profile_digest(&shell, &first_inputs, "", &home).unwrap();
        fs::write(home.join("bin/tool"), b"changed-size").unwrap();
        let second_inputs =
            host_shell_inputs(&shell, &home, &home.to_string_lossy(), &state).unwrap();
        let second = host_shell_profile_digest(&shell, &second_inputs, "", &home).unwrap();
        assert_ne!(first, second);
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn familiar_external_symlinks_are_skipped_but_custom_selections_fail() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-shell-external-defaults-{}-{unique}",
            std::process::id()
        ));
        let home = root.join("home");
        let outside = root.join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(outside.join("bin")).unwrap();
        fs::write(outside.join("aliases"), b"alias safe='printf safe'\n").unwrap();
        fs::write(outside.join("bin/tool"), b"#!/bin/sh\n").unwrap();
        symlink(outside.join("aliases"), home.join(".bash_aliases")).unwrap();
        symlink(outside.join("bin"), home.join("bin")).unwrap();
        let state = root.join("state");

        let familiar = host_shell_inputs(
            &ShellConfig::default(),
            &home,
            &home.to_string_lossy(),
            &state,
        )
        .unwrap();
        assert!(familiar.alias_files.is_empty());
        assert!(familiar.bin_dirs.is_empty());
        assert!(familiar.selected.is_empty());
        assert_eq!(familiar.warnings.len(), 2);
        assert!(
            familiar
                .warnings
                .iter()
                .all(|warning| warning.contains("automatically discovered"))
        );

        let custom = ShellConfig {
            profile: ShellProfile::Custom,
            alias_files: vec![".bash_aliases".to_owned()],
            bin_dirs: vec!["bin".to_owned()],
            ..ShellConfig::default()
        };
        let error = host_shell_inputs(&custom, &home, &home.to_string_lossy(), &state)
            .err()
            .expect("an explicit external selection must fail");
        assert!(error.contains("outside the host home"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_inputs_reject_missing_paths() {
        let home = std::env::temp_dir().join(format!("sbxr-missing-shell-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let shell = ShellConfig {
            profile: ShellProfile::Custom,
            alias_files: vec!["missing.aliases".to_owned()],
            ..ShellConfig::default()
        };
        assert!(
            host_shell_inputs(
                &shell,
                &home,
                &home.to_string_lossy(),
                Path::new("/tmp/outside-state")
            )
            .is_err()
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn rewrites_mirrored_host_paths_into_the_shell_mirror() {
        let mirrored = [PathBuf::from("bin"), PathBuf::from(".bash_aliases")];
        let text = concat!(
            "alias t='/host/home/bin/tool'\n",
            "alias u='~/bin/other' v=\"$HOME/bin\" w=${HOME}/bin/x\n",
            "alias p='/host/home/projects/app'\n",
            "alias q='/host/home/binaries/z' r='~/binx' s=a~/bin\n",
            "/host/homeother/bin\n",
        );
        assert_eq!(
            rewrite_shell_paths(text, "/host/home", &mirrored),
            concat!(
                "alias t='/home/agent/.local/share/sbxr/host-shell/bin/tool'\n",
                "alias u='~/.local/share/sbxr/host-shell/bin/other' ",
                "v=\"$HOME/.local/share/sbxr/host-shell/bin\" ",
                "w=${HOME}/.local/share/sbxr/host-shell/bin/x\n",
                "alias p='/home/agent/projects/app'\n",
                "alias q='/home/agent/binaries/z' r='~/binx' s=a~/bin\n",
                "/host/homeother/bin\n",
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn host_shell_archive_rewrites_contents_and_links_into_the_mirror() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!(
            "sbxr-shell-mirror-rewrite-{}-{unique}",
            std::process::id()
        ));
        let bin = home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let home_text = home.to_string_lossy().into_owned();
        fs::write(bin.join("tool"), format!("exec {home_text}/bin/real\n")).unwrap();
        fs::write(bin.join("real"), b"#!/bin/sh\n").unwrap();
        symlink(bin.join("real"), bin.join("absolute-link")).unwrap();

        let mut buffer = Vec::new();
        {
            let mut archive = TarWriter::with_policy(&mut buffer, MirrorPolicy::host_shell());
            archive.shell_mirror = vec![PathBuf::from("bin")];
            archive
                .append_tree(&bin, Path::new("bin"), &home_text)
                .unwrap();
            archive.finish().unwrap();
        }
        let mirrored = b"/home/agent/.local/share/sbxr/host-shell/bin/real";
        let occurrences = buffer
            .windows(mirrored.len())
            .filter(|window| *window == mirrored)
            .count();
        assert_eq!(occurrences, 1, "file contents point at the mirror");
        // In-tree links are emitted relative to the link (see
        // deep_in_tree_links_are_relative_and_extract_correctly).
        assert!(buffer.windows(5).any(|window| window == b"real\0"));
        assert!(
            !buffer
                .windows(home_text.len())
                .any(|window| window == home_text.as_bytes())
        );
        fs::remove_dir_all(home).unwrap();
    }

    fn scratch_home(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("sbxr-{label}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        // Canonical, so resolved-path checks compare like with like.
        fs::canonicalize(home).unwrap()
    }

    fn shell_archive(home: &Path, selected: &[PathBuf]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut archive = TarWriter::with_policy(&mut buffer, MirrorPolicy::host_shell());
            archive.shell_mirror = selected.to_vec();
            for relative in selected {
                archive
                    .append_tree(&home.join(relative), relative, &home.to_string_lossy())
                    .unwrap();
            }
            archive.finish().unwrap();
        }
        buffer
    }

    /// Extracts with the system `tar`, as the guest does.
    fn extract(buffer: &[u8], destination: &Path) {
        use std::process::{Command, Stdio};
        fs::create_dir_all(destination).unwrap();
        let mut child = Command::new("tar")
            .args(["-xf", "-", "-C"])
            .arg(destination)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(buffer).unwrap();
        assert!(child.wait().unwrap().success(), "tar rejected the archive");
    }

    #[cfg(unix)]
    #[test]
    fn familiar_profile_mirrors_symlinked_dotfiles_by_content() {
        use std::os::unix::fs::symlink;
        let home = scratch_home("dotfile-links");
        fs::create_dir_all(home.join("dotfiles/bin")).unwrap();
        fs::write(home.join("dotfiles/aliases"), b"alias ll='ls -l'\n").unwrap();
        fs::write(home.join("dotfiles/bin/tool"), b"#!/bin/sh\n").unwrap();
        symlink(home.join("dotfiles/aliases"), home.join(".bash_aliases")).unwrap();
        symlink(home.join("dotfiles/bin"), home.join("bin")).unwrap();
        let state = home.join("state");

        let shell = ShellConfig::default();
        let inputs = host_shell_inputs(&shell, &home, &home.to_string_lossy(), &state).unwrap();
        assert_eq!(
            inputs.selected,
            [PathBuf::from(".bash_aliases"), PathBuf::from("bin")]
        );
        let digest = host_shell_profile_digest(&shell, &inputs, "", &home).unwrap();
        fs::write(home.join("dotfiles/bin/tool"), b"#!/bin/sh\necho changed\n").unwrap();
        let changed = host_shell_profile_digest(&shell, &inputs, "", &home).unwrap();
        assert_ne!(
            digest, changed,
            "a link target's contents must be fingerprinted"
        );

        let out = home.join("extracted");
        extract(&shell_archive(&home, &inputs.selected), &out);
        let aliases = out.join(".bash_aliases");
        assert!(
            !fs::symlink_metadata(&aliases)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(aliases).unwrap(), b"alias ll='ls -l'\n");
        assert!(out.join("bin/tool").is_file());
        assert!(
            !fs::symlink_metadata(out.join("bin"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_links_in_a_mirrored_directory_do_not_block_launch() {
        use std::os::unix::fs::symlink;
        let home = scratch_home("dangling-link");
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::write(home.join("bin/tool"), b"#!/bin/sh\n").unwrap();
        symlink(home.join("bin/removed"), home.join("bin/stale")).unwrap();
        let shell = ShellConfig::default();
        let inputs =
            host_shell_inputs(&shell, &home, &home.to_string_lossy(), &home.join("s")).unwrap();
        host_shell_profile_digest(&shell, &inputs, "", &home).unwrap();
        let out = home.join("extracted");
        extract(&shell_archive(&home, &inputs.selected), &out);
        assert!(out.join("bin/tool").is_file());
        assert!(fs::symlink_metadata(out.join("bin/stale")).is_err());
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn permission_only_changes_refresh_the_mirror() {
        use std::os::unix::fs::PermissionsExt;
        let home = scratch_home("chmod");
        fs::create_dir_all(home.join("bin")).unwrap();
        let tool = home.join("bin/tool");
        fs::write(&tool, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).unwrap();
        let shell = ShellConfig::default();
        let inputs =
            host_shell_inputs(&shell, &home, &home.to_string_lossy(), &home.join("s")).unwrap();
        let before = host_shell_profile_digest(&shell, &inputs, "", &home).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let after = host_shell_profile_digest(&shell, &inputs, "", &home).unwrap();
        assert_ne!(before, after);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn rendered_overlay_is_part_of_the_digest() {
        let home = scratch_home("runtime-digest");
        let shell = ShellConfig::default();
        let inputs =
            host_shell_inputs(&shell, &home, &home.to_string_lossy(), &home.join("s")).unwrap();
        assert_ne!(
            host_shell_profile_digest(&shell, &inputs, "PATH=a", &home).unwrap(),
            host_shell_profile_digest(&shell, &inputs, "PATH=b", &home).unwrap()
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_smuggle_credential_stores_into_the_mirror() {
        use std::os::unix::fs::symlink;
        let home = scratch_home("credential-links");
        let state = home.join("state");
        fs::create_dir_all(home.join("dotfiles/ssh")).unwrap();
        fs::write(home.join("dotfiles/ssh/id_ed25519"), b"secret").unwrap();
        symlink(home.join("dotfiles/ssh"), home.join(".ssh")).unwrap();
        fs::create_dir_all(home.join(".aws")).unwrap();
        fs::write(home.join(".aws/credentials"), b"secret").unwrap();
        symlink(home.join(".aws"), home.join("link")).unwrap();
        symlink(home.join(".aws"), home.join("aws-tools")).unwrap();

        let custom = |aliases: Vec<&str>, bins: Vec<&str>| ShellConfig {
            profile: ShellProfile::Custom,
            alias_files: aliases.into_iter().map(str::to_owned).collect(),
            bin_dirs: bins.into_iter().map(str::to_owned).collect(),
            ..ShellConfig::default()
        };
        let text = home.to_string_lossy();
        // A directory that contains the resolved target of ~/.ssh.
        assert!(
            host_shell_inputs(&custom(vec![], vec!["dotfiles"]), &home, &text, &state).is_err()
        );
        // A file reached through a symlinked intermediate component.
        assert!(
            host_shell_inputs(
                &custom(vec!["link/credentials"], vec![]),
                &home,
                &text,
                &state
            )
            .is_err()
        );
        // A top-level symlink straight to a store.
        assert!(
            host_shell_inputs(&custom(vec![], vec!["aws-tools"]), &home, &text, &state).is_err()
        );
        // An unrelated directory beside them stays allowed.
        fs::create_dir_all(home.join("dotfiles-bin")).unwrap();
        assert!(
            host_shell_inputs(&custom(vec![], vec!["dotfiles-bin"]), &home, &text, &state).is_ok()
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn deep_in_tree_links_are_relative_and_extract_correctly() {
        use std::os::unix::fs::symlink;
        let home = scratch_home("deep-links");
        let deep = "tools/some-long-vendor-name/versions/1.2.3-with-a-long-suffix/linux-x86_64/bin";
        fs::create_dir_all(home.join("bin").join(deep)).unwrap();
        let target = home.join("bin").join(deep).join("real-tool");
        fs::write(&target, b"#!/bin/sh\necho real\n").unwrap();
        // Absolute host link; rewritten with the 40-byte mirror prefix it
        // would exceed ustar's 100-byte link field.
        symlink(&target, home.join("bin/tool")).unwrap();
        symlink(
            Path::new(deep).join("real-tool"),
            home.join("bin/relative-tool"),
        )
        .unwrap();
        assert!(
            format!("/home/agent/.local/share/sbxr/host-shell/bin/{deep}/real-tool").len() > 100
        );

        let selected = [PathBuf::from("bin")];
        let out = home.join("extracted");
        extract(&shell_archive(&home, &selected), &out);
        for link in ["bin/tool", "bin/relative-tool"] {
            let text = fs::read_link(out.join(link)).unwrap();
            assert!(text.is_relative(), "{link} -> {}", text.display());
            assert_eq!(
                fs::read(out.join(link)).unwrap(),
                b"#!/bin/sh\necho real\n",
                "{link} must resolve inside the extracted mirror"
            );
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn computes_relative_link_text() {
        assert_eq!(relative_link(Path::new("bin"), Path::new("bin/a/b")), "a/b");
        assert_eq!(
            relative_link(Path::new("bin/a"), Path::new("bin/c")),
            "../c"
        );
        assert_eq!(relative_link(Path::new("bin/a"), Path::new("bin/a")), ".");
        assert_eq!(relative_link(Path::new(""), Path::new(".x")), ".x");
    }

    #[test]
    fn rewrites_quoted_home_and_respects_left_boundaries() {
        let mirrored = [PathBuf::from("bin")];
        let text = concat!(
            "a=\"$HOME\"/bin/x b=\"${HOME}\"/bin/y\n",
            "c=/data/home/u/bin/z d=\\$HOME/bin/w\n",
            "e=$HOME/bin\n",
        );
        assert_eq!(
            rewrite_shell_paths(text, "/home/u", &mirrored),
            concat!(
                "a=\"$HOME\"/.local/share/sbxr/host-shell/bin/x ",
                "b=\"${HOME}\"/.local/share/sbxr/host-shell/bin/y\n",
                "c=/data/home/u/bin/z d=\\$HOME/bin/w\n",
                "e=$HOME/.local/share/sbxr/host-shell/bin\n",
            )
        );
    }

    #[test]
    fn generated_overlays_parse_in_their_shells() {
        use std::process::Command;
        let aliases = vec![".config/it's aliases".to_owned()];
        let bins = vec!["bin".to_owned(), "my tools".to_owned()];
        for (program, binary) in [
            (ShellProgram::Bash, "bash"),
            (ShellProgram::Zsh, "zsh"),
            (ShellProgram::Fish, "fish"),
        ] {
            if Command::new(binary).arg("--version").output().is_err() {
                eprintln!("skipping {binary}: not installed");
                continue;
            }
            for prompt in [
                PromptStyle::Classic,
                PromptStyle::Plain,
                PromptStyle::Custom,
            ] {
                let shell = ShellConfig {
                    program,
                    prompt,
                    prompt_file: (prompt == PromptStyle::Custom)
                        .then(|| ".config/sbxr/prompt".to_owned()),
                    ..ShellConfig::default()
                };
                let rendered = render_host_shell_runtime(
                    &shell,
                    &aliases,
                    &bins,
                    Some(".config/sbxr/host-aliases.bash"),
                );
                let output = Command::new(binary)
                    .args(["-n", "-c", &rendered])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{binary} rejected the overlay:\n{rendered}\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }

    #[test]
    fn round_trips_versioned_bootstrap_state() {
        let state = BootstrapState {
            locale: true,
            git: true,
            shell: true,
            agent: true,
            vscode: false,
        };
        let bytes = serialize_bootstrap_state(state).unwrap();
        assert_eq!(parse_bootstrap_state(&bytes), state);
        assert_eq!(
            parse_bootstrap_state(br#"{"agent":true}"#),
            BootstrapState {
                locale: false,
                git: false,
                shell: false,
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
    fn removes_literal_pi_model_credentials_but_keeps_environment_references() {
        let root = std::env::temp_dir().join(format!("sbxr-pi-model-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("models.json");
        fs::write(
            &path,
            br#"{
              "providers": {
                "literal": {
                  "apiKey": "reusable-secret",
                  "headers": {"Authorization": "Bearer reusable-secret"}
                },
                "environment": {
                  "apiKey": "$SAFE_API_KEY",
                  "headers": {"Authorization": "${SAFE_HEADER}"}
                }
              }
            }"#,
        )
        .unwrap();
        let (bytes, removed) = sanitized_pi_models(&path, "/host/home").unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let providers = value["providers"].as_object().unwrap();
        assert_eq!(removed, 2);
        assert!(providers["literal"].get("apiKey").is_none());
        assert!(
            providers["literal"]["headers"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert_eq!(providers["environment"]["apiKey"], "$SAFE_API_KEY");
        assert_eq!(
            providers["environment"]["headers"]["Authorization"],
            "${SAFE_HEADER}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unsafe_pi_credential_references() {
        for value in [
            Value::String("literal".to_owned()),
            Value::String("!host-command".to_owned()),
            Value::String("Bearer $TOKEN".to_owned()),
            Value::String("$9INVALID".to_owned()),
            Value::Bool(true),
        ] {
            assert!(!safe_pi_config_reference(&value));
        }
        for value in ["$TOKEN", "${TOKEN}", "$_TOKEN_2"] {
            assert!(safe_pi_config_reference(&Value::String(value.to_owned())));
        }
    }

    #[test]
    fn removes_nested_agent_credentials_without_mistaking_model_limits() {
        let mut value = serde_json::json!({
            "env": {
                "ANTHROPIC_API_KEY": "reusable",
                "SAFE_COLOR": "blue"
            },
            "headers": {
                "Authorization": "Bearer reusable",
                "User-Agent": "sbxr"
            },
            "authTokens": ["reusable-one", "reusable-two"],
            "clientSecrets": {"production": "reusable"},
            "maxTokens": 4096,
            "outputTokens": 2048,
            "refreshTokenExpiresAt": 1234
        });
        assert_eq!(remove_json_credentials(&mut value), 4);
        assert!(value["env"].get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(value["env"]["SAFE_COLOR"], "blue");
        assert!(value["headers"].get("Authorization").is_none());
        assert_eq!(value["headers"]["User-Agent"], "sbxr");
        assert!(value.get("authTokens").is_none());
        assert!(value.get("clientSecrets").is_none());
        assert_eq!(value["maxTokens"], 4096);
        assert_eq!(value["outputTokens"], 2048);
        assert_eq!(value["refreshTokenExpiresAt"], 1234);
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

    #[test]
    fn leaves_docker_owned_claude_skills_mount_read_only() {
        assert!(!CLAUDE_PATHS.contains(&".claude/skills"));
    }
}
