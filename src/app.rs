use crate::cli::{self, Action, Cli};
use crate::environment::{Context, Project, RocksDbHost};
use crate::{auth, host, process, sandbox, vscode};
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
    let context = Context::discover()?;
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
            vscode::check_host()?;
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            auth::offer_host_import(&context, &project, preset)?;
            vscode::open(&context, &project.name, &project.path, true)?;
        }
        Action::Up(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            println!("ready: ssh {}.sbx", project.name);
        }
        Action::Shell(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            sandbox::run_remote(&project, &["bash", "-l"])?;
        }
        Action::Codex(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            if !preset.has_codex() {
                return Err(format!(
                    "Codex is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::run_remote(&project, &["codex"])?;
        }
        Action::Claude(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            if !preset.has_claude() {
                return Err(format!(
                    "Claude is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::run_remote(&project, &["claude"])?;
        }
        Action::Pi(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            if preset != cli::Preset::MultiAgent {
                return Err(format!(
                    "Pi is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::run_remote(&project, &["pi"])?;
        }
        Action::AuthImport(_) => {
            auth::import(&context, &project, preset, rocksdb.as_ref())?;
        }
        Action::AuthStatus(_) => {
            sandbox::ensure_dev(&context, &project, preset, rocksdb.as_ref())?;
            auth::show_status(&context, &project.name, preset);
        }
        Action::Audit(_) => sandbox::audit(&context, &project, preset, rocksdb.as_ref())?,
        Action::Review(_) => {
            vscode::check_host()?;
            sandbox::ensure_review(&context, &project, rocksdb.as_ref())?;
            vscode::open(&context, &project.review_name, &project.path, false)?;
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
            process::run_checked(
                process::sbx_command(context.preserve_xdg_state)
                    .arg("stop")
                    .arg(&project.name),
                "stopping sandbox",
            )?;
        }
        Action::Remove { force, .. } => {
            process::require("sbx")?;
            println!(
                "Removing sandbox-local state for {}; host project files remain.",
                project.name
            );
            let mut command = process::sbx_command(context.preserve_xdg_state);
            command.arg("rm");
            if force {
                command.arg("--force");
            }
            command.arg(&project.name);
            process::run_checked(&mut command, "removing sandbox")?;
        }
        Action::Setup | Action::Doctor | Action::Help => unreachable!(),
    }
    Ok(0)
}
