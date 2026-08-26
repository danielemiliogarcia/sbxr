use crate::cli::{self, Action, Cli};
use crate::environment::{Context, Project, RocksDbHost};
use crate::{auth, envfile, host, process, sandbox, sbx, vscode};
use std::env;
use std::path::Path;

pub(crate) fn run() -> Result<i32, String> {
    let cli = Cli::parse(env::args_os().skip(1).collect(), env::var_os("SBXR_PRESET"))?;
    let action = cli.action;
    let preset = cli.preset;
    let rocksdb_requested = cli.rocksdb;

    if matches!(action, Action::Help) {
        cli::print_usage();
        return Ok(0);
    }

    if matches!(action, Action::Setup) {
        let preserve_xdg_state = env::var("SBXR_PRESERVE_XDG_STATE_HOME").as_deref() == Ok("1");
        host::setup(preset, preserve_xdg_state)?;
        return Ok(0);
    }

    if !matches!(action, Action::Doctor) {
        host::preflight(&action)?;
    }

    let rocksdb = RocksDbHost::resolve(rocksdb_requested)?;
    let context = if matches!(action, Action::Inspect(_)) {
        Context::discover_read_only()?
    } else {
        Context::discover()?
    };
    if matches!(action, Action::Doctor) {
        return Ok(if host::doctor(&context, preset, rocksdb.as_ref()) {
            0
        } else {
            1
        });
    }

    let requested = action.path().unwrap_or_else(|| Path::new("."));
    let project = Project::resolve(requested, preset, rocksdb.as_ref())?;

    match action {
        Action::New(_) | Action::Code(_) => {
            open_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
        }
        Action::Update(_) => {
            open_dev(&context, &project, preset, rocksdb.as_ref(), true)?;
        }
        Action::Up(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
            print_ready(&context, &project);
        }
        Action::Shell(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
            sandbox::run_remote(&context, &project, &["bash", "-l"])?;
        }
        Action::Codex(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
            if !preset.has_codex() {
                return Err(format!(
                    "Codex is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::run_remote(&context, &project, &["codex"])?;
        }
        Action::Claude(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
            if !preset.has_claude() {
                return Err(format!(
                    "Claude is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::run_remote(&context, &project, &["claude"])?;
        }
        Action::Pi(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
            if preset != cli::Preset::MultiAgent {
                return Err(format!(
                    "Pi is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::run_remote(&context, &project, &["pi"])?;
        }
        Action::AuthImport(_) => {
            auth::import(&context, &project, preset, rocksdb.as_ref())?;
        }
        Action::AuthStatus(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref(), false)?;
            auth::show_status(&context, &project.name, preset);
        }
        Action::Audit(_) => sandbox::audit(&context, &project, preset, rocksdb.as_ref())?,
        Action::Review(_) => {
            vscode::check_host()?;
            sandbox::ensure_review(&context, &project, rocksdb.as_ref())?;
            vscode::open(&context, &project.review_name, &project.path, false, true)?;
        }
        Action::Inspect(_) => {
            let inspection = envfile::inspect(
                &context,
                &project,
                envfile::Kind::Development(preset),
                rocksdb.as_ref(),
            )?;
            inspection.print();
        }
        Action::Name(_) => println!("{}", project.name),
        Action::Status(_) => {
            return Ok(if sandbox::show_status(&context, &project)? {
                0
            } else {
                1
            });
        }
        Action::Stop(_) => {
            process::require("sbx")?;
            vscode::close_for_stop(&context, &project.name, &project.path)?;
            sbx::stop(context.preserve_xdg_state, &project.name)?;
        }
        Action::Remove { force, .. } => {
            process::require("sbx")?;
            println!(
                "Removing sandbox-local state for {}; host project files remain.",
                project.name
            );
            sbx::require_minimum_version(context.preserve_xdg_state)?;
            let definition = envfile::materialize(
                &context,
                &project,
                envfile::Kind::Development(preset),
                rocksdb.as_ref(),
            )?;
            sbx::env_remove(context.preserve_xdg_state, definition.path(), force)?;
            definition.remove_host_state(&context.state_root)?;
        }
        Action::Setup | Action::Doctor | Action::Help => unreachable!(),
    }
    Ok(0)
}

fn open_dev(
    context: &Context,
    project: &Project,
    preset: cli::Preset,
    rocksdb: Option<&RocksDbHost>,
    force_sync: bool,
) -> Result<(), String> {
    vscode::check_host()?;
    if force_sync {
        println!("==> forcing host integration update for {}", project.name);
    }
    let mut state = sandbox::ensure_dev(context, project, preset, rocksdb, force_sync)?;
    if force_sync || !state.vscode {
        auth::offer_host_import(context, project, preset)?;
    }
    if vscode::open(context, &project.name, &project.path, true, !state.vscode)? {
        state.vscode = true;
        crate::sync::write_bootstrap_state(&project.name, context.preserve_xdg_state, state)?;
        println!("==> sandbox host-integration bootstrap complete");
    }
    print_ready(context, project);
    Ok(())
}

fn print_ready(context: &Context, project: &Project) {
    println!("ready: ssh {}.sbx", project.name);
    println!(
        "environment: {}",
        envfile::development_path(context, project).display()
    );
    println!("inspect: sbxr inspect {}", shell_quote_path(&project.path));
}

fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
