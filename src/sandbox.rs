use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost, crate_name};
use crate::{embedded, envfile, git, process, sbx, sync};
use std::env;
use std::fs;
use std::process::{Command, Stdio};

fn validate_kits(context: &Context) -> Result<(), String> {
    for name in embedded::KIT_NAMES {
        let spec = context.kit_root.join(name).join("spec.yaml");
        if !spec.is_file() {
            return Err(format!("missing vendored kit: {}", spec.display()));
        }
    }
    Ok(())
}

fn sandbox_exists(context: &Context, wanted: &str) -> Result<bool, String> {
    let output = sbx::list_json(context.preserve_xdg_state)?;
    Ok(sbx::sandbox_state(&output.stdout, wanted)?.is_some())
}

pub(crate) fn show_status(context: &Context, project: &Project) -> Result<bool, String> {
    process::require("sbx")?;
    let output = sbx::list_json(context.preserve_xdg_state)?;
    let state = sbx::sandbox_state(&output.stdout, &project.name)?;
    println!("sandbox: {}", project.name);
    println!("project: {}", project.path.display());
    match state {
        Some(state) => {
            println!("status: {state}");
            Ok(true)
        }
        None => {
            println!("status: not found");
            Ok(false)
        }
    }
}

pub(crate) fn ensure_dev(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
    force_sync: bool,
) -> Result<sync::BootstrapState, String> {
    process::require("sbx")?;
    validate_kits(context)?;
    sbx::require_minimum_version(context.preserve_xdg_state)?;
    let definition = envfile::materialize(
        context,
        project,
        envfile::Kind::Development(preset),
        rocksdb,
    )?;
    if sandbox_exists(context, &project.name)? {
        definition.verify_existing(context.preserve_xdg_state)?;
        println!("==> reusing sandbox {}", project.name);
        sbx::env_ensure_up(context.preserve_xdg_state, definition.path())?;
    } else {
        println!(
            "==> creating sandbox {} for {}",
            project.name,
            project.path.display()
        );
        sbx::env_ensure_up(context.preserve_xdg_state, definition.path())?;
        definition.mark_applied(context.preserve_xdg_state)?;
    }
    maybe_initialize_empty_project(context, project)?;
    let mut state = sync::bootstrap_state(&project.name, context.preserve_xdg_state)?;
    if force_sync {
        state = sync::BootstrapState::default();
        sync::write_bootstrap_state(&project.name, context.preserve_xdg_state, state)?;
    }
    if !state.locale {
        println!("==> ensuring the en_US.UTF-8 locale for VS Code terminals");
        ensure_utf8_locale(context, &project.name)?;
        state.locale = true;
        sync::write_bootstrap_state(&project.name, context.preserve_xdg_state, state)?;
    }
    if !state.git {
        println!("==> configuring editor, Git completion, identity, SSH push, and commit signing");
        git::configure(context, &project.name, &project.path)?;
        state.git = true;
        sync::write_bootstrap_state(&project.name, context.preserve_xdg_state, state)?;
    }
    if !state.agent {
        sync_agent_capabilities(context, project, preset)?;
        state.agent = true;
        sync::write_bootstrap_state(&project.name, context.preserve_xdg_state, state)?;
    }
    Ok(state)
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
    project: &Project,
    preset: Preset,
) -> Result<(), String> {
    if env::var("SBXR_SYNC_AGENT_CONFIG").as_deref() == Ok("0") {
        println!("note: host agent capability mirroring is disabled");
        return Ok(());
    }
    println!(
        "==> mirroring trusted host Codex/Claude/Pi configuration and extensions into {}",
        project.name
    );
    sync::capabilities(
        &project.name,
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
) -> Result<(), String> {
    process::require("sbx")?;
    process::require("git")?;
    validate_kits(context)?;
    sbx::require_minimum_version(context.preserve_xdg_state)?;
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

    let definition = envfile::materialize(context, project, envfile::Kind::Review, rocksdb)?;
    if sandbox_exists(context, &project.review_name)? {
        definition.verify_existing(context.preserve_xdg_state)?;
        println!("==> reusing review sandbox {}", project.review_name);
        sbx::env_ensure_up(context.preserve_xdg_state, definition.path())?;
    } else {
        println!(
            "==> creating no-OAuth clone-mode review sandbox {}",
            project.review_name
        );
        sbx::env_ensure_up(context.preserve_xdg_state, definition.path())?;
        definition.mark_applied(context.preserve_xdg_state)?;
    }
    Ok(())
}

fn maybe_initialize_empty_project(context: &Context, project: &Project) -> Result<(), String> {
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
        .arg(&project.name)
        .arg("--")
        .args(["sh", "-lc"])
        .arg("cd -- \"$1\" && cargo init --vcs none --name \"$2\" . && cargo generate-lockfile")
        .arg("sh")
        .arg(&project.path)
        .arg(crate_name);
    process::run_checked(&mut command, "initializing Cargo project")
}

pub(crate) fn run_remote(
    context: &Context,
    project: &Project,
    program: &[&str],
) -> Result<(), String> {
    process::run_checked(
        &mut sbx::interactive_exec_command(
            context.preserve_xdg_state,
            &project.name,
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
) -> Result<(), String> {
    ensure_dev(context, project, preset, rocksdb, false)?;
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
            .arg(&project.name)
            .arg("--")
            .args(["bash", "-lc", script, "bash"])
            .arg(&project.path),
        "auditing Cargo project",
    )
}
