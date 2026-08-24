use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost, crate_name};
use crate::{embedded, process, sync};
use std::env;
use std::ffi::OsStr;
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
    let output = process::output_checked(
        process::sbx_command(context.preserve_xdg_state).args(["ls", "--json"]),
        "listing Docker sandboxes",
    )
    .map_err(|error| format!("{error}\nRun `sbx login`, then `sbx diagnose`."))?;
    Ok(process::json_sandbox_names(&output.stdout)
        .iter()
        .any(|name| name == wanted))
}

pub(crate) fn ensure_dev(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
) -> Result<(), String> {
    process::require("sbx")?;
    validate_kits(context)?;
    if sandbox_exists(context, &project.name)? {
        println!("==> reusing sandbox {}", project.name);
    } else {
        println!(
            "==> creating sandbox {} for {}",
            project.name,
            project.path.display()
        );
        let mut command = process::sbx_command(context.preserve_xdg_state);
        command.arg("create").arg("--name").arg(&project.name);
        command.arg("--kit").arg(context.kit_root.join("rust"));
        if preset == Preset::MultiAgent {
            command
                .arg("--kit")
                .arg(context.kit_root.join("claude-cli"));
            command.arg("--kit").arg(context.kit_root.join("pi-cli"));
        }
        append_rocksdb_create_options(&mut command, context, rocksdb);
        command
            .arg("--kit")
            .arg(context.kit_root.join("vscode-remote"))
            .arg(preset.primary_agent())
            .arg(&project.path);
        if let Some(rocksdb) = rocksdb {
            command.arg(&rocksdb.mount_argument);
        }
        process::run_checked(&mut command, "creating development sandbox")?;
    }
    maybe_initialize_empty_project(context, project)?;
    sync_agent_capabilities(context, project, preset)
}

fn append_rocksdb_create_options(
    command: &mut Command,
    context: &Context,
    rocksdb: Option<&RocksDbHost>,
) {
    if let Some(rocksdb) = rocksdb {
        command
            .arg("--kit")
            .arg(context.kit_root.join("rocksdb-host"))
            .arg("--env")
            .arg(format!("ROCKSDB_LIB_DIR={}", rocksdb.library_dir))
            .arg("--env")
            .arg(format!("ROCKSDB_INCLUDE_DIR={}", rocksdb.include_dir))
            .arg("--env")
            .arg(format!("LD_LIBRARY_PATH={}", rocksdb.library_dir));
    }
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
        "==> mirroring safe host Codex/Claude/Pi capabilities into {}",
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

    if sandbox_exists(context, &project.review_name)? {
        println!("==> reusing review sandbox {}", project.review_name);
    } else {
        println!(
            "==> creating no-OAuth clone-mode review sandbox {}",
            project.review_name
        );
        let mut command = process::sbx_command(context.preserve_xdg_state);
        command
            .arg("create")
            .arg("--clone")
            .arg("--name")
            .arg(&project.review_name)
            .arg("--kit")
            .arg(context.kit_root.join("rust"));
        append_rocksdb_create_options(&mut command, context, rocksdb);
        command
            .arg("--kit")
            .arg(context.kit_root.join("vscode-remote"))
            .arg("shell")
            .arg(&project.path);
        if let Some(rocksdb) = rocksdb {
            command.arg(&rocksdb.mount_argument);
        }
        process::run_checked(&mut command, "creating review sandbox")?;
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
    let mut command = process::sbx_command(context.preserve_xdg_state);
    command
        .arg("exec")
        .arg(&project.name)
        .arg("--")
        .args(["sh", "-lc"])
        .arg("cd -- \"$1\" && cargo init --vcs none --name \"$2\" . && cargo generate-lockfile")
        .arg("sh")
        .arg(&project.path)
        .arg(crate_name);
    process::run_checked(&mut command, "initializing Cargo project")
}

pub(crate) fn run_remote(project: &Project, program: &[&str]) -> Result<(), String> {
    process::require("ssh")?;
    let mut remote = format!(
        "cd -- {} && exec",
        process::shell_quote(project.path.as_os_str())
    );
    for argument in program {
        remote.push(' ');
        remote.push_str(&process::shell_quote(OsStr::new(argument)));
    }
    process::run_checked(
        Command::new("ssh")
            .arg("-t")
            .arg(format!("{}.sbx", project.name))
            .arg(remote),
        "opening sandbox command",
    )
}

pub(crate) fn audit(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
) -> Result<(), String> {
    ensure_dev(context, project, preset, rocksdb)?;
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
        process::sbx_command(context.preserve_xdg_state)
            .arg("exec")
            .arg(&project.name)
            .arg("--")
            .args(["bash", "-lc", script, "bash"])
            .arg(&project.path),
        "auditing Cargo project",
    )
}
