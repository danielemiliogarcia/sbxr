use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost, crate_name};
use crate::profile::{NetworkProfile, Profile, WorkspaceMode};
use crate::{
    browser, embedded, envfile, git, github_proxy, node_tools, orca, process, sbx, sha256, sync,
};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HARDEN_CREDENTIAL_PERMISSIONS: &str = r#"
set -euo pipefail
for file in \
  "${CODEX_HOME:-$HOME/.codex}/auth.json" \
  "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/.credentials.json" \
  "${PI_CODING_AGENT_DIR:-$HOME/.pi/agent}/auth.json"
do
  candidate=$file
  while [ "$candidate" != / ]; do
    if [ -L "$candidate" ]; then
      echo "refusing a symlinked managed credential path" >&2
      exit 1
    fi
    candidate=${candidate%/*}
    [ -n "$candidate" ] || candidate=/
  done
  [ -e "$file" ] || continue
  [ -f "$file" ] && [ "$(stat -c '%u' "$file")" = "$(id -u)" ] || {
    echo "refusing an unsafe managed credential file" >&2
    exit 1
  }
  chmod 0600 -- "$file"
done
"#;

fn validate_kits(context: &Context) -> Result<(), String> {
    for name in embedded::KIT_NAMES {
        let spec = context.kit_root.join(name).join("spec.yaml");
        if !spec.is_file() {
            return Err(format!("missing vendored kit: {}", spec.display()));
        }
    }
    for (kit, relative) in embedded::KIT_ASSET_PATHS {
        let asset = context.kit_root.join(kit).join(relative);
        if !asset.is_file() {
            return Err(format!("missing vendored kit asset: {}", asset.display()));
        }
    }
    Ok(())
}

fn sandbox_exists(context: &Context, wanted: &str) -> Result<bool, String> {
    let output = sbx::list_json(context.preserve_xdg_state)?;
    Ok(sbx::sandbox_state(&output.stdout, wanted)?.is_some())
}

pub(crate) fn sandbox_name<'a>(project: &'a Project, profile: &Profile) -> &'a str {
    match profile.workspace {
        WorkspaceMode::Shared => &project.name,
        WorkspaceMode::Isolated => &project.isolated_name,
    }
}

pub(crate) fn ensure_dev(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
    profile: &Profile,
    force_sync: bool,
    force_plan: bool,
) -> Result<sync::BootstrapState, String> {
    process::require("sbx")?;
    validate_kits(context)?;
    let sbx_version = sbx::require_minimum_version(context.preserve_xdg_state)?;
    let name = sandbox_name(project, profile);
    let kind = match profile.workspace {
        WorkspaceMode::Shared => envfile::Kind::Development(preset),
        WorkspaceMode::Isolated => envfile::Kind::Isolated(preset),
    };
    let definition = envfile::materialize(context, project, kind, rocksdb, profile)?;
    if sandbox_exists(context, name)? {
        definition.verify_existing(context.preserve_xdg_state)?;
        println!("==> reusing sandbox {name}");
        sbx::env_ensure_up(
            context.preserve_xdg_state,
            definition.path(),
            sbx_version,
            force_plan,
            &profile.resources,
        )?;
    } else {
        println!(
            "==> creating sandbox {} for {}",
            name,
            project.path.display()
        );
        sbx::env_ensure_up(
            context.preserve_xdg_state,
            definition.path(),
            sbx_version,
            force_plan,
            &profile.resources,
        )?;
        definition.mark_applied(context.preserve_xdg_state)?;
    }
    maybe_initialize_empty_project(context, name, project)?;
    ensure_project_toolchain(context, name, project)?;
    node_tools::ensure(context, name, preset, profile)?;
    if profile.github_proxy {
        github_proxy::ensure(context, name)?;
    }
    if profile.orca {
        orca::ensure_installed(context, name)?;
    }
    let mut state = sync::bootstrap_state(name, context.preserve_xdg_state)?;
    if force_sync {
        state = sync::BootstrapState::default();
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    if !state.locale {
        println!("==> ensuring the en_US.UTF-8 locale for VS Code terminals");
        ensure_utf8_locale(context, name)?;
        state.locale = true;
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    if !state.git {
        println!("==> configuring editor, Git completion, identity, SSH push, and commit signing");
        git::configure(context, name, &project.path)?;
        state.git = true;
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    let shell_disabled = env::var("SBXR_SYNC_HOST_SHELL").as_deref() == Ok("0");
    let shell_current = shell_disabled
        || (state.shell
            && sync::host_shell_profile_current(
                name,
                context.preserve_xdg_state,
                &profile.shell,
                &context.state_root,
            )?);
    if !shell_current {
        sync_host_shell(context, name, &profile.shell)?;
        state.shell = true;
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    if !state.agent {
        sync_agent_capabilities(context, name, preset)?;
        state.agent = true;
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    harden_credential_permissions(context, name)?;
    if profile.browser {
        browser::configure(context, name, preset)?;
    }
    apply_network_profile(context, name, profile.network)?;
    configure_paseo_relay(context, name, profile.paseo_relay, force_sync)?;
    Ok(state)
}

fn harden_credential_permissions(context: &Context, sandbox_name: &str) -> Result<(), String> {
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args(["--", "bash", "-lc", HARDEN_CREDENTIAL_PERMISSIONS]),
        "protecting managed guest credential sentinels",
    )
}

fn configure_paseo_relay(
    context: &Context,
    sandbox_name: &str,
    enabled: bool,
    force: bool,
) -> Result<(), String> {
    const MARKER: &str = "/home/agent/.local/state/sbxr/paseo-relay-v1";
    let wanted = if enabled { "enabled" } else { "disabled" };
    if !force
        && sbx::read_file(context.preserve_xdg_state, sandbox_name, MARKER)?
            .as_deref()
            .map(|value| value.trim_ascii())
            == Some(wanted.as_bytes())
    {
        return Ok(());
    }
    let existing = process::output_checked(
        sbx::command(context.preserve_xdg_state).args(["policy", "ls", sandbox_name, "--json"]),
        "inspecting the sandbox Paseo relay policy",
    )?;
    let relay_rule_exists =
        policy_has_resource(&existing.stdout, sandbox_name, "relay.paseo.sh:443")?;
    if enabled {
        if !relay_rule_exists {
            println!("==> allowing the optional Paseo relay for this sandbox");
            process::run_checked(
                sbx::command(context.preserve_xdg_state).args([
                    "policy",
                    "allow",
                    "network",
                    "--sandbox",
                    sandbox_name,
                    "relay.paseo.sh:443",
                ]),
                "allowing the sandbox Paseo relay",
            )?;
        }
    } else if relay_rule_exists {
        println!("==> removing the optional Paseo relay rule for this sandbox");
        process::run_checked(
            sbx::command(context.preserve_xdg_state).args([
                "policy",
                "rm",
                "network",
                "--sandbox",
                sandbox_name,
                "--resource",
                "relay.paseo.sh:443",
                "--force",
            ]),
            "removing the sandbox Paseo relay rule",
        )?;
    }

    let was_running = paseo_status(context, sandbox_name)?.is_some();
    let script = r#"
set -euo pipefail
enabled=$1
config=${PASEO_HOME:-$HOME/.paseo}/config.json
mkdir -p "${config%/*}"
node - "$config" "$enabled" <<'JS'
const fs = require('node:fs');
const path = process.argv[2];
const enabled = process.argv[3] === 'true';
if (fs.existsSync(path) && fs.lstatSync(path).isSymbolicLink()) {
  throw new Error('refusing a symlinked Paseo configuration');
}
if (!enabled && !fs.existsSync(path)) {
  console.log('unchanged');
  process.exit(0);
}
let config = {version: 1};
if (fs.existsSync(path)) config = JSON.parse(fs.readFileSync(path, 'utf8'));
if (!enabled && config.daemon?.relay === undefined) {
  console.log('unchanged');
  process.exit(0);
}
config.daemon ??= {};
config.daemon.listen ??= '127.0.0.1:6767';
if (config.daemon.relay?.enabled === enabled) {
  console.log('unchanged');
  process.exit(0);
}
config.daemon.relay = {...config.daemon.relay, enabled};
const temporary = `${path}.sbxr-${process.pid}`;
fs.writeFileSync(temporary, `${JSON.stringify(config, null, 2)}\n`, {mode: 0o600, flag: 'wx'});
fs.renameSync(temporary, path);
console.log('changed');
JS
"#;
    let configured = process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .arg("--")
            .args(["bash", "-lc", script, "bash"])
            .arg(if enabled { "true" } else { "false" }),
        "configuring the Paseo relay",
    )?;
    let changed = String::from_utf8_lossy(&configured.stdout).trim() == "changed";
    if was_running && changed {
        println!("==> restarting Paseo to apply relay configuration");
        let stopped = sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .arg("--")
            .args(["bash", "-lc", "paseo stop"])
            .status()
            .map_err(|error| format!("could not stop Paseo before relay restart: {error}"))?;
        if !stopped.success() {
            eprintln!("warning: Paseo stop failed before relay restart; attempting start anyway");
        }
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state)
                .arg(sandbox_name)
                .arg("--")
                .args(["bash", "-lc", "paseo start"]),
            "restarting the sandbox Paseo daemon",
        )?;
    }
    sbx::write_file(
        context.preserve_xdg_state,
        sandbox_name,
        MARKER,
        OsStr::new(wanted),
    )?;
    Ok(())
}

fn policy_has_resource(bytes: &[u8], sandbox_name: &str, resource: &str) -> Result<bool, String> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("could not parse `sbx policy ls --json`: {error}"))?;
    let rules = document
        .get("rules")
        .and_then(serde_json::Value::as_array)
        .ok_or("`sbx policy ls --json` has no rules array")?;
    let scope = format!("sandbox:{sandbox_name}");
    Ok(rules.iter().any(|rule| {
        rule.get("scope").and_then(serde_json::Value::as_str) == Some(scope.as_str())
            && rule.get("sandbox_id").and_then(serde_json::Value::as_str) == Some(sandbox_name)
            && rule
                .get("resource_type")
                .and_then(serde_json::Value::as_str)
                == Some("network")
            && rule.get("decision").and_then(serde_json::Value::as_str) == Some("allow")
            && rule.get("status").and_then(serde_json::Value::as_str) == Some("active")
            && rule.get("editable").and_then(serde_json::Value::as_bool) == Some(true)
            && rule
                .get("resources")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|resources| {
                    resources
                        .iter()
                        .any(|value| value.as_str() == Some(resource))
                })
    }))
}

pub(crate) fn paseo_relay_policy_enabled(
    context: &Context,
    sandbox_name: &str,
) -> Result<bool, String> {
    let output = process::output_checked(
        sbx::command(context.preserve_xdg_state).args(["policy", "ls", sandbox_name, "--json"]),
        "inspecting the sandbox Paseo relay policy",
    )?;
    policy_has_resource(&output.stdout, sandbox_name, "relay.paseo.sh:443")
}

fn ensure_project_toolchain(
    context: &Context,
    sandbox_name: &str,
    project: &Project,
) -> Result<(), String> {
    let manifests = project_toolchain_manifests(&project.path)?;
    if manifests.is_empty() {
        return Ok(());
    }
    let mut contract = b"sbxr-project-toolchains-v2\0".to_vec();
    for manifest in &manifests {
        let metadata = fs::symlink_metadata(manifest)
            .map_err(|error| format!("could not inspect {}: {error}", manifest.display()))?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "Rust toolchain manifest must be a regular file: {}",
                manifest.display()
            ));
        }
        if metadata.len() > 64 * 1024 {
            return Err(format!(
                "refusing oversized Rust toolchain manifest: {}",
                manifest.display()
            ));
        }
        let relative = manifest.strip_prefix(&project.path).map_err(|error| {
            format!(
                "could not make Rust toolchain manifest {} project-relative: {error}",
                manifest.display()
            )
        })?;
        let contents = fs::read(manifest)
            .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
        contract.extend_from_slice(relative.as_os_str().as_encoded_bytes());
        contract.push(0);
        contract.extend_from_slice(&contents);
        contract.push(0);
    }
    let wanted = sha256::hex(&contract);
    const MARKER: &str = "/home/agent/.local/state/sbxr/project-toolchains-v2.sha256";
    let marker_matches = sbx::read_file(context.preserve_xdg_state, sandbox_name, MARKER)?
        .as_deref()
        .map(|value| value.trim_ascii())
        == Some(wanted.as_bytes());
    if marker_matches && project_toolchains_available(context, sandbox_name, &manifests)? {
        return Ok(());
    }

    println!(
        "==> reconciling {} project Rust toolchain manifest(s)",
        manifests.len()
    );
    for manifest in &manifests {
        println!("  - {}", manifest.display());
    }
    let script = r#"
set -euo pipefail
for directory do
  cd -- "$directory"
  RUSTUP_AUTO_INSTALL=1 cargo --version >/dev/null
  RUSTUP_AUTO_INSTALL=0 rustup show active-toolchain
done
"#;
    let mut command = sbx::exec_command(context.preserve_xdg_state);
    command
        .arg(sandbox_name)
        .arg("--")
        .args(["bash", "-lc", script, "bash"]);
    for manifest in &manifests {
        command.arg(
            manifest.parent().ok_or_else(|| {
                format!("toolchain manifest has no parent: {}", manifest.display())
            })?,
        );
    }
    process::run_checked(
        &mut command,
        "installing or selecting project Rust toolchains",
    )?;
    sbx::write_file(
        context.preserve_xdg_state,
        sandbox_name,
        MARKER,
        OsStr::new(&wanted),
    )
}

fn project_toolchains_available(
    context: &Context,
    sandbox_name: &str,
    manifests: &[PathBuf],
) -> Result<bool, String> {
    if manifests.is_empty() {
        return Ok(true);
    }
    let script = r#"
set -euo pipefail
for directory do
  cd -- "$directory"
  RUSTUP_AUTO_INSTALL=0 cargo --version >/dev/null
  RUSTUP_AUTO_INSTALL=0 rustc --version >/dev/null
  RUSTUP_AUTO_INSTALL=0 rust-analyzer --version | grep -F 'rust-analyzer 1.99.1' >/dev/null
done
"#;
    let mut command = sbx::exec_command(context.preserve_xdg_state);
    command
        .arg(sandbox_name)
        .arg("--")
        .args(["bash", "-lc", script, "bash"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for manifest in manifests {
        command.arg(
            manifest.parent().ok_or_else(|| {
                format!("toolchain manifest has no parent: {}", manifest.display())
            })?,
        );
    }
    command
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("could not check project Rust toolchains: {error}"))
}

pub(crate) fn verify_project_toolchains(
    context: &Context,
    sandbox_name: &str,
    project: &Project,
) -> Result<(), String> {
    let manifests = project_toolchain_manifests(&project.path)?;
    if !project_toolchains_available(context, sandbox_name, &manifests)? {
        return Err(
            "one or more project Rust toolchains are unavailable; run `sbxr repair`".to_owned(),
        );
    }
    println!(
        "ok   project Rust toolchains: {} manifest(s) ready",
        manifests.len()
    );
    Ok(())
}

fn project_toolchain_manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    const MAX_DEPTH: usize = 4;
    const MAX_MANIFESTS: usize = 64;
    const SKIPPED_DIRECTORIES: &[&str] =
        &[".git", ".jj", ".cache", "node_modules", "target", "vendor"];

    fn visit(directory: &Path, depth: usize, manifests: &mut Vec<PathBuf>) -> Result<(), String> {
        let manifest = ["rust-toolchain.toml", "rust-toolchain"]
            .into_iter()
            .map(|name| directory.join(name))
            .find(|path| path.exists());
        if let Some(manifest) = manifest {
            manifests.push(manifest);
            if manifests.len() > MAX_MANIFESTS {
                return Err(format!(
                    "refusing to reconcile more than {MAX_MANIFESTS} Rust toolchain manifests below {}",
                    directory.display()
                ));
            }
        }
        if depth == MAX_DEPTH {
            return Ok(());
        }
        let mut entries = fs::read_dir(directory)
            .map_err(|error| format!("could not inspect {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not inspect {}: {error}", directory.display()))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
            if !metadata.file_type().is_dir()
                || SKIPPED_DIRECTORIES
                    .iter()
                    .any(|name| entry.file_name() == OsStr::new(name))
            {
                continue;
            }
            visit(&path, depth + 1, manifests)?;
        }
        Ok(())
    }

    let mut manifests = Vec::new();
    visit(root, 0, &mut manifests)?;
    manifests.sort();
    Ok(manifests)
}

fn sync_host_shell(
    context: &Context,
    sandbox_name: &str,
    shell: &crate::profile::ShellConfig,
) -> Result<(), String> {
    if env::var("SBXR_SYNC_HOST_SHELL").as_deref() == Ok("0") {
        println!("note: host shell-environment mirroring is disabled");
        return Ok(());
    }
    println!(
        "==> applying {} {} shell profile to {}",
        shell.profile.name(),
        shell.program.name(),
        sandbox_name
    );
    sync::host_shell(
        sandbox_name,
        context.preserve_xdg_state,
        shell,
        &context.state_root,
    )
}

fn ensure_utf8_locale(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let script = r#"
set -euo pipefail
if locale -a | tr '[:upper:]' '[:lower:]' | grep -qx 'en_us.utf8'; then
  exit 0
fi
export DEBIAN_FRONTEND=noninteractive
attempt=0
until sudo apt-get update; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then
    echo "apt-get update remained unavailable after 60 seconds" >&2
    exit 1
  fi
  sleep 2
done
sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends locales
if [ -f /etc/locale.gen ]; then
  sudo sed -i 's/^# *en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen
  sudo locale-gen en_US.UTF-8
else
  sudo localedef -i en_US -f UTF-8 en_US.UTF-8
fi
locale -a | tr '[:upper:]' '[:lower:]' | grep -qx 'en_us.utf8'
"#;
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .arg("--")
            .args(["bash", "-lc", script]),
        "installing the sandbox UTF-8 locale",
    )
}

fn sync_agent_capabilities(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
) -> Result<(), String> {
    if env::var("SBXR_SYNC_AGENT_CONFIG").as_deref() == Ok("0") {
        println!("note: host agent capability mirroring is disabled");
        return Ok(());
    }
    println!(
        "==> mirroring trusted host Codex/Claude/Pi configuration and extensions into {}",
        sandbox_name
    );
    sync::capabilities(
        sandbox_name,
        context.preserve_xdg_state,
        preset.has_codex(),
        preset.has_claude(),
        preset == Preset::MultiAgent,
    )
}

pub(crate) fn ensure_review(
    context: &Context,
    project: &Project,
    rocksdb: Option<&RocksDbHost>,
    force_plan: bool,
) -> Result<(), String> {
    process::require("sbx")?;
    process::require("git")?;
    validate_kits(context)?;
    let sbx_version = sbx::require_minimum_version(context.preserve_xdg_state)?;
    let git_status = Command::new("git")
        .arg("-C")
        .arg(&project.path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !git_status.success() {
        return Err("review mode requires a Git working tree".to_owned());
    }

    let profile = Profile::default();
    let definition =
        envfile::materialize(context, project, envfile::Kind::Review, rocksdb, &profile)?;
    if sandbox_exists(context, &project.review_name)? {
        definition.verify_existing(context.preserve_xdg_state)?;
        println!("==> reusing review sandbox {}", project.review_name);
        sbx::env_ensure_up(
            context.preserve_xdg_state,
            definition.path(),
            sbx_version,
            force_plan,
            &profile.resources,
        )?;
    } else {
        println!(
            "==> creating no-OAuth clone-mode review sandbox {}",
            project.review_name
        );
        sbx::env_ensure_up(
            context.preserve_xdg_state,
            definition.path(),
            sbx_version,
            force_plan,
            &profile.resources,
        )?;
        definition.mark_applied(context.preserve_xdg_state)?;
    }
    Ok(())
}

fn maybe_initialize_empty_project(
    context: &Context,
    sandbox_name: &str,
    project: &Project,
) -> Result<(), String> {
    if project.path.join("Cargo.toml").is_file() {
        return Ok(());
    }
    let non_ignored = fs::read_dir(&project.path)
        .map_err(|error| format!("could not inspect {}: {error}", project.path.display()))?
        .filter_map(Result::ok)
        .any(|entry| entry.file_name() != ".git" && entry.file_name() != ".gitignore");
    if non_ignored {
        eprintln!("note: no Cargo.toml found; the non-empty directory was left unchanged");
        return Ok(());
    }

    let crate_name = crate_name(&project.path);
    println!("==> initializing empty directory as Rust package '{crate_name}'");
    let mut command = sbx::exec_command(context.preserve_xdg_state);
    command
        .arg(sandbox_name)
        .arg("--")
        .args(["sh", "-lc"])
        .arg("cd -- \"$1\" && cargo init --vcs none --name \"$2\" . && cargo generate-lockfile")
        .arg("sh")
        .arg(&project.path)
        .arg(crate_name);
    process::run_checked(&mut command, "initializing Cargo project")
}

/// Starts the sandbox-local Paseo daemon when it is not already running and
/// returns the SSH transport URL a host Paseo client connects to.
pub(crate) fn paseo_endpoint_for(context: &Context, sandbox_name: &str) -> Result<String, String> {
    if paseo_status(context, sandbox_name)?.is_none() {
        println!("==> starting the sandbox-local Paseo daemon");
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state)
                .arg(sandbox_name)
                .arg("--")
                .args(["bash", "-lc", "paseo start"]),
            "starting the sandbox Paseo daemon",
        )?;
    }
    let listen = paseo_status(context, sandbox_name)?
        .ok_or("the sandbox Paseo daemon did not report a running state")?;
    let port = listen.rsplit(':').next().unwrap_or("6767").to_owned();
    let user = remote_user(context, sandbox_name)?;
    let mut endpoint = format!("ssh://{user}@{sandbox_name}.sbx");
    if port != "6767" {
        endpoint.push_str(&format!("?daemonPort={port}"));
    }
    Ok(endpoint)
}

/// Returns the daemon's listen address while it is running, or `None`.
fn paseo_status(context: &Context, sandbox_name: &str) -> Result<Option<String>, String> {
    let output = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .arg("--")
        .args(["bash", "-lc", "paseo status --json"])
        .output()
        .map_err(|error| format!("could not query the sandbox Paseo daemon: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let document: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(document) => document,
        Err(_) => return Ok(None),
    };
    if document.get("localDaemon").and_then(|value| value.as_str()) != Some("running") {
        return Ok(None);
    }
    Ok(Some(
        document
            .get("listen")
            .and_then(|value| value.as_str())
            .unwrap_or("127.0.0.1:6767")
            .to_owned(),
    ))
}

fn remote_user(context: &Context, sandbox_name: &str) -> Result<String, String> {
    let output = process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .arg("--")
            .args(["id", "-un"]),
        "resolving the sandbox user",
    )?;
    let user = String::from_utf8(output.stdout)
        .map_err(|error| format!("the sandbox user name is not UTF-8: {error}"))?
        .trim()
        .to_owned();
    if user.is_empty() {
        return Err("the sandbox reported an empty user name".to_owned());
    }
    Ok(user)
}

pub(crate) fn run_remote(
    context: &Context,
    project: &Project,
    sandbox_name: &str,
    program: &[&str],
) -> Result<(), String> {
    process::run_checked(
        &mut sbx::interactive_exec_command(
            context.preserve_xdg_state,
            sandbox_name,
            &project.path,
            program,
        ),
        "opening sandbox command",
    )
}

pub(crate) fn audit(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
    profile: &Profile,
    force_plan: bool,
) -> Result<(), String> {
    ensure_dev(
        context, project, preset, rocksdb, profile, false, force_plan,
    )?;
    let name = sandbox_name(project, profile);
    let script = r#"
set -euo pipefail
cd -- "$1"
test -f Cargo.lock || {
  echo "error: Cargo.lock is required for a reproducible audit" >&2
  exit 1
}
cargo audit
cargo deny check advisories sources
if [ -f supply-chain/config.toml ]; then
  cargo vet
else
  echo "note: cargo-vet skipped; no supply-chain/config.toml policy exists"
fi
"#;
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(name)
            .arg("--")
            .args(["bash", "-lc", script, "bash"])
            .arg(&project.path),
        "auditing Cargo project",
    )
}

const RESTRICTED_NETWORK_RESOURCES: &[&str] = &[
    "registry.npmjs.org",
    "index.crates.io",
    "static.crates.io",
    "crates.io",
    "static.rust-lang.org",
    "archive.ubuntu.com",
    "security.ubuntu.com",
    "ports.ubuntu.com",
    "download.docker.com",
    "cdn.playwright.dev",
    "playwright.download.prss.microsoft.com",
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkRule {
    id: String,
    scope: String,
    sandbox_id: Option<String>,
    resources: BTreeSet<String>,
    editable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedNetworkRule {
    id: String,
    profile: NetworkProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NetworkPolicyRecord {
    profile: NetworkProfile,
    owned_rules: Vec<OwnedNetworkRule>,
}

fn apply_network_profile(
    context: &Context,
    sandbox_name: &str,
    network: NetworkProfile,
) -> Result<(), String> {
    let mut record =
        read_network_policy_record(context, sandbox_name)?.unwrap_or(NetworkPolicyRecord {
            profile: NetworkProfile::Development,
            owned_rules: Vec::new(),
        });
    let mut rules = read_network_rules(context, sandbox_name)?;

    // The ownership record lives in protected host state. Never trust a marker
    // from the writable guest when deciding which host policy rule may be removed.
    record.owned_rules.retain(|owned| {
        rules.iter().any(|rule| {
            rule.id == owned.id && owned_rule_matches(rule, sandbox_name, owned.profile)
        })
    });
    record
        .owned_rules
        .sort_by(|left, right| left.id.cmp(&right.id));
    record
        .owned_rules
        .dedup_by(|left, right| left.id == right.id);
    let obsolete_ids = record
        .owned_rules
        .iter()
        .filter(|owned| owned.profile != network)
        .map(|owned| owned.id.clone())
        .collect::<BTreeSet<_>>();

    if network != NetworkProfile::Development
        && !network_profile_enforced_excluding(&rules, sandbox_name, network, &obsolete_ids)
    {
        let before = rules
            .iter()
            .map(|rule| rule.id.clone())
            .collect::<BTreeSet<_>>();
        println!(
            "==> applying sandbox-scoped {} network profile",
            network.name()
        );
        let mut command = sbx::command(context.preserve_xdg_state);
        command.args(["policy", "deny", "network", "--sandbox", sandbox_name]);
        command.arg(network_resources(network).join(","));
        process::run_checked(&mut command, "applying the sandbox network profile")?;
        rules = read_network_rules(context, sandbox_name)?;
        let added = rules
            .iter()
            .filter(|rule| {
                !before.contains(&rule.id) && owned_rule_matches(rule, sandbox_name, network)
            })
            .map(|rule| rule.id.clone())
            .collect::<Vec<_>>();
        if !added.is_empty() {
            record
                .owned_rules
                .extend(added.into_iter().map(|id| OwnedNetworkRule {
                    id,
                    profile: network,
                }));
        } else if !network_profile_enforced_excluding(&rules, sandbox_name, network, &obsolete_ids)
        {
            return Err(format!(
                "Docker accepted the {} network profile but no enforcing rule was found",
                network.name()
            ));
        }
    }

    // Persist the transition before removing the previous rule. If removal is
    // interrupted, the next run still knows exactly which sbxr-owned ID to retry.
    record.profile = network;
    write_network_policy_record(context, sandbox_name, &record)?;
    let obsolete = record
        .owned_rules
        .iter()
        .filter(|owned| network == NetworkProfile::Development || owned.profile != network)
        .map(|owned| owned.id.clone())
        .collect::<Vec<_>>();
    for id in obsolete {
        println!("==> removing the previous sbxr network-profile rule");
        process::run_checked(
            sbx::command(context.preserve_xdg_state).args([
                "policy",
                "rm",
                "network",
                "--sandbox",
                sandbox_name,
                "--id",
                &id,
                "--force",
            ]),
            "removing the previous sandbox network profile",
        )?;
        record.owned_rules.retain(|owned| owned.id != id);
        write_network_policy_record(context, sandbox_name, &record)?;
    }
    if network != NetworkProfile::Development {
        let rules = read_network_rules(context, sandbox_name)?;
        if !network_profile_enforced(&rules, sandbox_name, network) {
            return Err(format!(
                "the {} network profile was not enforced after reconciliation",
                network.name()
            ));
        }
    }
    Ok(())
}

fn read_network_rules(context: &Context, sandbox_name: &str) -> Result<Vec<NetworkRule>, String> {
    let output = process::output_checked(
        sbx::command(context.preserve_xdg_state).args(["policy", "ls", sandbox_name, "--json"]),
        "inspecting the sandbox network profile",
    )?;
    parse_network_rules(&output.stdout)
}

fn parse_network_rules(bytes: &[u8]) -> Result<Vec<NetworkRule>, String> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("could not parse `sbx policy ls --json`: {error}"))?;
    let values = document
        .get("rules")
        .and_then(serde_json::Value::as_array)
        .ok_or("`sbx policy ls --json` has no rules array")?;
    let mut rules = Vec::new();
    for value in values {
        if value
            .get("resource_type")
            .and_then(serde_json::Value::as_str)
            != Some("network")
            || value.get("decision").and_then(serde_json::Value::as_str) != Some("deny")
            || value.get("status").and_then(serde_json::Value::as_str) != Some("active")
            || value.get("id").and_then(serde_json::Value::as_str) == Some("default-deny-all")
        {
            continue;
        }
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or("active network deny rule has no id")?;
        let scope = value
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .ok_or("active network deny rule has no scope")?;
        let resources = value
            .get("resources")
            .and_then(serde_json::Value::as_array)
            .ok_or("active network deny rule has no resources array")?
            .iter()
            .map(|resource| {
                resource
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("active network deny rule has a non-string resource")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let actions = value
            .get("actions")
            .and_then(serde_json::Value::as_array)
            .ok_or("active network deny rule has no actions array")?;
        if !["net:connect:tcp", "net:connect:udp"]
            .iter()
            .all(|wanted| actions.iter().any(|action| action.as_str() == Some(wanted)))
        {
            continue;
        }
        rules.push(NetworkRule {
            id: id.to_owned(),
            scope: scope.to_owned(),
            sandbox_id: value
                .get("sandbox_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            resources,
            editable: value
                .get("editable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(rules)
}

fn network_resources(profile: NetworkProfile) -> &'static [&'static str] {
    match profile {
        NetworkProfile::Development => &[],
        NetworkProfile::Restricted => RESTRICTED_NETWORK_RESOURCES,
        NetworkProfile::Offline => &["**"],
    }
}

fn resource_set(profile: NetworkProfile) -> BTreeSet<String> {
    network_resources(profile)
        .iter()
        .map(|resource| (*resource).to_owned())
        .collect()
}

fn applicable_rule(rule: &NetworkRule, sandbox_name: &str) -> bool {
    rule.scope == "global"
        || (rule.scope == format!("sandbox:{sandbox_name}")
            && rule.sandbox_id.as_deref() == Some(sandbox_name))
}

fn network_profile_enforced(
    rules: &[NetworkRule],
    sandbox_name: &str,
    profile: NetworkProfile,
) -> bool {
    network_profile_enforced_excluding(rules, sandbox_name, profile, &BTreeSet::new())
}

fn network_profile_enforced_excluding(
    rules: &[NetworkRule],
    sandbox_name: &str,
    profile: NetworkProfile,
    excluded_ids: &BTreeSet<String>,
) -> bool {
    network_resources(profile).iter().all(|resource| {
        rules.iter().any(|rule| {
            !excluded_ids.contains(&rule.id)
                && applicable_rule(rule, sandbox_name)
                && (rule.resources.contains(*resource) || rule.resources.contains("**"))
        })
    })
}

fn owned_rule_matches(rule: &NetworkRule, sandbox_name: &str, profile: NetworkProfile) -> bool {
    let wanted = resource_set(profile);
    rule.editable
        && rule.scope == format!("sandbox:{sandbox_name}")
        && rule.sandbox_id.as_deref() == Some(sandbox_name)
        && !rule.resources.is_empty()
        && rule.resources.is_subset(&wanted)
}

fn read_network_policy_record(
    context: &Context,
    sandbox_name: &str,
) -> Result<Option<NetworkPolicyRecord>, String> {
    let Some(bytes) = envfile::read_network_policy_record(context, sandbox_name)? else {
        return Ok(None);
    };
    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid protected network-policy record: {error}"))?;
    if document.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("protected network-policy record has an unsupported version".to_owned());
    }
    let profile = document
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "protected network-policy record has no profile".to_owned())?;
    let profile = NetworkProfile::parse(profile)?;
    let values = document
        .get("ownedRules")
        .and_then(serde_json::Value::as_array)
        .ok_or("protected network-policy record has no ownedRules array")?;
    let mut owned_rules = Vec::new();
    for value in values {
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("protected network-policy record has an invalid rule id")?;
        let profile = value
            .get("profile")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                "protected network-policy record has a rule without a profile".to_owned()
            })?;
        let profile = NetworkProfile::parse(profile)?;
        if profile == NetworkProfile::Development {
            return Err(
                "protected network-policy record claims a development deny rule".to_owned(),
            );
        }
        owned_rules.push(OwnedNetworkRule {
            id: id.to_owned(),
            profile,
        });
    }
    Ok(Some(NetworkPolicyRecord {
        profile,
        owned_rules,
    }))
}

fn write_network_policy_record(
    context: &Context,
    sandbox_name: &str,
    record: &NetworkPolicyRecord,
) -> Result<(), String> {
    let rules = record
        .owned_rules
        .iter()
        .map(|rule| serde_json::json!({"id": rule.id, "profile": rule.profile.name()}))
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "profile": record.profile.name(),
        "ownedRules": rules,
    }))
    .map_err(|error| format!("could not serialize network-policy record: {error}"))?;
    envfile::write_network_policy_record(context, sandbox_name, &bytes)
}

pub(crate) fn network_profile_applied(
    context: &Context,
    sandbox_name: &str,
    profile: NetworkProfile,
) -> Result<bool, String> {
    let Some(record) = read_network_policy_record(context, sandbox_name)? else {
        return Ok(false);
    };
    if record.profile != profile {
        return Ok(false);
    }
    let rules = read_network_rules(context, sandbox_name)?;
    let owned_rules_valid = record.owned_rules.iter().all(|owned| {
        owned.profile == profile
            && rules.iter().any(|rule| {
                rule.id == owned.id && owned_rule_matches(rule, sandbox_name, owned.profile)
            })
    });
    if !owned_rules_valid {
        return Ok(false);
    }
    Ok(match profile {
        NetworkProfile::Development => record.owned_rules.is_empty(),
        NetworkProfile::Restricted | NetworkProfile::Offline => {
            network_profile_enforced(&rules, sandbox_name, profile)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        HARDEN_CREDENTIAL_PERMISSIONS, NetworkProfile, network_profile_enforced,
        network_profile_enforced_excluding, owned_rule_matches, parse_network_rules,
        policy_has_resource, project_toolchain_manifests,
    };
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn discovers_nested_toolchains_and_prunes_generated_trees() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-toolchain-discovery-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("repository/crates/member")).unwrap();
        fs::create_dir_all(root.join("target/generated")).unwrap();
        fs::write(root.join("rust-toolchain"), b"stable\n").unwrap();
        fs::write(
            root.join("repository/rust-toolchain.toml"),
            b"[toolchain]\nchannel = \"1.95.0\"\n",
        )
        .unwrap();
        fs::write(root.join("repository/rust-toolchain"), b"ignored\n").unwrap();
        fs::write(
            root.join("target/generated/rust-toolchain.toml"),
            b"[toolchain]\nchannel = \"nightly\"\n",
        )
        .unwrap();

        let manifests = project_toolchain_manifests(&root).unwrap();

        assert_eq!(
            manifests,
            [
                root.join("repository/rust-toolchain.toml"),
                root.join("rust-toolchain"),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_only_exact_structured_policy_resources() {
        let policy = include_bytes!("../tests/fixtures/sbx-policy-ls-0.51.0.json");
        assert!(policy_has_resource(policy, "rust-multi-test", "relay.paseo.sh:443").unwrap());
        assert!(!policy_has_resource(policy, "rust-multi-test", "relay.paseo.sh").unwrap());
        assert!(!policy_has_resource(policy, "missing", "relay.paseo.sh:443").unwrap());
        assert!(policy_has_resource(br#"{"rules": {}}"#, "sandbox", "x").is_err());
    }

    #[test]
    fn parses_and_matches_only_full_transport_network_denies() {
        let policy = br#"{
          "rules": [
            {
              "id":"default-deny-all", "scope":"global", "resource_type":"network",
              "decision":"deny", "resources":["**"], "status":"active", "editable":false,
              "actions":["net:connect:tcp","net:connect:udp"]
            },
            {
              "id":"restricted", "scope":"sandbox:test", "sandbox_id":"test",
              "resource_type":"network", "decision":"deny",
              "resources":["registry.npmjs.org","index.crates.io","static.crates.io","crates.io","static.rust-lang.org","archive.ubuntu.com","security.ubuntu.com","ports.ubuntu.com","download.docker.com","cdn.playwright.dev","playwright.download.prss.microsoft.com"],
              "status":"active", "editable":true,
              "actions":["net:connect:tcp","net:connect:udp"]
            },
            {
              "id":"tcp-only", "scope":"sandbox:test", "sandbox_id":"test",
              "resource_type":"network", "decision":"deny", "resources":["**"],
              "status":"active", "editable":true, "actions":["net:connect:tcp"]
            }
          ]
        }"#;
        let rules = parse_network_rules(policy).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(network_profile_enforced(
            &rules,
            "test",
            NetworkProfile::Restricted
        ));
        assert!(!network_profile_enforced(
            &rules,
            "test",
            NetworkProfile::Offline
        ));
        assert!(owned_rule_matches(
            &rules[0],
            "test",
            NetworkProfile::Restricted
        ));
        let mut split = rules[0].clone();
        split.resources = ["registry.npmjs.org".to_owned()].into_iter().collect();
        assert!(owned_rule_matches(
            &split,
            "test",
            NetworkProfile::Restricted
        ));
        let offline = super::NetworkRule {
            id: "offline".to_owned(),
            scope: "sandbox:test".to_owned(),
            sandbox_id: Some("test".to_owned()),
            resources: ["**".to_owned()].into_iter().collect(),
            editable: true,
        };
        assert!(network_profile_enforced(
            std::slice::from_ref(&offline),
            "test",
            NetworkProfile::Restricted
        ));
        assert!(!network_profile_enforced_excluding(
            &[offline],
            "test",
            NetworkProfile::Restricted,
            &["offline".to_owned()].into_iter().collect()
        ));
        assert!(!owned_rule_matches(
            &rules[0],
            "other",
            NetworkProfile::Restricted
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hardens_managed_sentinels_and_rejects_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-credential-permissions-{}-{unique}",
            std::process::id()
        ));
        let codex = root.join(".codex/auth.json");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::write(&codex, b"{}\n").unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o644)).unwrap();

        let status = Command::new("bash")
            .args(["-lc", HARDEN_CREDENTIAL_PERMISSIONS])
            .env("HOME", &root)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::metadata(&codex).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let target = root.join("target.json");
        fs::write(&target, b"{}\n").unwrap();
        fs::remove_file(&codex).unwrap();
        symlink(&target, &codex).unwrap();
        let status = Command::new("bash")
            .args(["-lc", HARDEN_CREDENTIAL_PERMISSIONS])
            .env("HOME", &root)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .status()
            .unwrap();
        assert!(!status.success());
        fs::remove_dir_all(root).unwrap();
    }
}
