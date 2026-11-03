use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost, crate_name};
use crate::profile::{NetworkProfile, Profile, WorkspaceMode};
use crate::{
    browser, embedded, envfile, git, github_proxy, node_tools, orca, process, sbx, sha256, sync,
};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    github_proxy::ensure(context, name)?;
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
    if !state.shell {
        sync_host_shell(context, name)?;
        state.shell = true;
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    if !state.agent {
        sync_agent_capabilities(context, name, preset)?;
        state.agent = true;
        sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
    }
    if profile.browser {
        browser::configure(context, name)?;
    }
    apply_network_profile(context, name, profile.network)?;
    Ok(state)
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
    if sbx::read_file(context.preserve_xdg_state, sandbox_name, MARKER)?
        .as_deref()
        .map(|value| value.trim_ascii())
        == Some(wanted.as_bytes())
    {
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

fn sync_host_shell(context: &Context, sandbox_name: &str) -> Result<(), String> {
    if env::var("SBXR_SYNC_HOST_SHELL").as_deref() == Ok("0") {
        println!("note: host shell-environment mirroring is disabled");
        return Ok(());
    }
    println!(
        "==> mirroring trusted host Bash aliases and ~/bin utilities into {}",
        sandbox_name
    );
    sync::host_shell(sandbox_name, context.preserve_xdg_state)
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
sudo sed -i 's/^# *en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen
sudo locale-gen en_US.UTF-8
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

fn apply_network_profile(
    context: &Context,
    sandbox_name: &str,
    network: NetworkProfile,
) -> Result<(), String> {
    const MARKER: &str = "/home/agent/.local/state/sbxr/network-profile-v1";
    if sbx::read_file(context.preserve_xdg_state, sandbox_name, MARKER)?
        .as_deref()
        .map(|value| value.trim_ascii())
        == Some(network.name().as_bytes())
    {
        return Ok(());
    }
    let resources: &[&str] = match network {
        NetworkProfile::Development => &[],
        NetworkProfile::Restricted => &[
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
        ],
        NetworkProfile::Offline => &["**"],
    };
    if !resources.is_empty() {
        let existing = sbx::command(context.preserve_xdg_state)
            .args(["policy", "ls", sandbox_name, "--json"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        if !resources.iter().all(|resource| existing.contains(resource)) {
            println!(
                "==> applying sandbox-scoped {} network profile",
                network.name()
            );
            let mut command = sbx::command(context.preserve_xdg_state);
            command.args(["policy", "deny", "network", "--sandbox", sandbox_name]);
            command.arg(resources.join(","));
            process::run_checked(&mut command, "applying the sandbox network profile")?;
        }
    }
    sbx::write_file(
        context.preserve_xdg_state,
        sandbox_name,
        MARKER,
        OsStr::new(network.name()),
    )
}

#[cfg(test)]
mod tests {
    use super::project_toolchain_manifests;
    use std::fs;
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
}
