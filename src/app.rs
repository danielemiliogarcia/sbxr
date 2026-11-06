use crate::cli::{self, Action, Cli};
use crate::environment::{Context, Project, RocksDbHost};
use crate::profile::{self, WorkspaceMode};
use crate::{
    auth, diagnostics, envfile, github_proxy, health, host, orca, process, sandbox, sbx, sign,
    verify, vscode,
};
use std::env;
use std::path::Path;

pub(crate) fn run() -> Result<i32, String> {
    let mut cli = Cli::parse(env::args_os().skip(1).collect(), env::var_os("SBXR_PRESET"))?;
    if matches!(cli.action, Action::Help) {
        cli::print_usage();
        return Ok(0);
    }
    if matches!(cli.action, Action::Version) {
        cli::print_version();
        return Ok(0);
    }
    if let Action::Setup(request) = &cli.action {
        let preserve = env::var("SBXR_PRESERVE_XDG_STATE_HOME").as_deref() == Ok("1");
        host::setup(cli.preset, preserve)?;
        if request.prebuild {
            let context = Context::discover()?;
            host::prebuild(&context, cli.preset)?;
        }
        return Ok(0);
    }
    if let Action::CommitPgpSign(request) = &cli.action {
        host::preflight(&cli.action)?;
        sign::commit_pgp_sign(
            request.path.as_deref().unwrap_or_else(|| Path::new(".")),
            &request.selection,
            cli.force,
        )?;
        return Ok(0);
    }
    if !matches!(cli.action, Action::Doctor(_)) {
        host::preflight(&cli.action)?;
    }

    let rocksdb = RocksDbHost::resolve(cli.rocksdb.take())?;
    let read_only = matches!(
        cli.action,
        Action::Inspect(_) | Action::Profile(_) | Action::Diagnose(_)
    );
    let context = if read_only {
        Context::discover_read_only()?
    } else {
        Context::discover()?
    };
    if let Action::Doctor(path) = &cli.action {
        let mut healthy = host::doctor(&context, cli.preset, rocksdb.as_ref());
        if let Some(path) = path {
            verify::report_host_versions(cli.preset);
            let project = Project::resolve(path, cli.preset, rocksdb.as_ref())?;
            let selected = profile::resolve(&context, &project, &cli.profile)?;
            let inventory = sbx::list_json(context.preserve_xdg_state)?;
            let name = sandbox::sandbox_name(&project, &selected);
            if sbx::sandbox_state(&inventory.stdout, name)?.is_some() {
                if let Err(error) = verify::run(
                    &context,
                    &project,
                    cli.preset,
                    rocksdb.as_ref(),
                    &selected,
                    false,
                ) {
                    println!("FAIL sandbox version/credential report: {error}");
                    healthy = false;
                }
            } else {
                println!("info project sandbox {name} does not exist; guest checks skipped");
            }
        }
        return Ok(if healthy { 0 } else { 1 });
    }

    let requested = cli.action.path().unwrap_or_else(|| Path::new("."));
    let project = Project::resolve(requested, cli.preset, rocksdb.as_ref())?;
    match &cli.action {
        Action::Isolated(_) => cli.profile.workspace = Some(WorkspaceMode::Isolated),
        Action::Wizard(_) => cli.profile.wizard = true,
        Action::Orca(_) => cli.profile.orca = Some(true),
        _ => {}
    }
    let mut selected = profile::resolve(&context, &project, &cli.profile)?;
    if cli.profile.wizard {
        selected = profile::run_wizard(&context, &project, &selected)?;
    }
    if selected.browser && !cli.preset.has_codex() {
        return Err(
            "the browser MCP requires a Codex-capable preset; use rust-codex or rust-multi-agent"
                .to_owned(),
        );
    }
    let mut report = if action_has_report(&cli.action) {
        Some(diagnostics::Report::start(
            &context,
            action_label(&cli.action),
        )?)
    } else {
        None
    };
    if let Some(report) = &mut report {
        report.event("profile", "validated", None)?;
        if let Ok(info) = sbx::runtime_info(context.preserve_xdg_state) {
            report.runtime(&info)?;
        }
    }
    let result = (|| -> Result<i32, String> {
        if selected.github_proxy && action_ensures_sandbox(&cli.action) {
            github_proxy::preflight_host()?;
        }
        if let Some(report) = &mut report {
            report.event("host-preflight", "completed", None)?;
            report.event("operation", "started", None)?;
        }
        let code = dispatch(
            &context,
            &project,
            cli.preset,
            rocksdb.as_ref(),
            &selected,
            cli.action,
            cli.force,
        )?;
        if let Some(report) = &mut report {
            report.event("operation", "completed", None)?;
        }
        Ok(code)
    })();
    match result {
        Ok(code) => {
            if let Some(report) = report {
                println!("diagnostic report: {}", report.complete()?.display());
            }
            Ok(code)
        }
        Err(error) => {
            if let Some(report) = report {
                if let Ok(path) = report.fail(&error) {
                    eprintln!("diagnostic report: {}", path.display());
                }
            }
            Err(error)
        }
    }
}

fn dispatch(
    context: &Context,
    project: &Project,
    preset: cli::Preset,
    rocksdb: Option<&RocksDbHost>,
    profile: &profile::Profile,
    action: Action,
    force: bool,
) -> Result<i32, String> {
    let name = sandbox::sandbox_name(project, profile);
    match action {
        Action::New(_) | Action::Code(_) | Action::Isolated(_) | Action::Wizard(_) => {
            open_dev(context, project, preset, rocksdb, profile, false, force)?
        }
        Action::Update(_) => open_dev(context, project, preset, rocksdb, profile, true, force)?,
        Action::Up(_) => {
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            print_ready(context, project, profile);
        }
        Action::Shell(_) => {
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            sandbox::run_remote(
                context,
                project,
                name,
                profile.shell.program.login_arguments(),
            )?;
        }
        Action::Codex(_) => {
            if !preset.has_codex() {
                return Err(format!(
                    "Codex is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            sandbox::run_remote(context, project, name, &["codex"])?;
        }
        Action::Claude(_) => {
            if !preset.has_claude() {
                return Err(format!(
                    "Claude is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            sandbox::run_remote(context, project, name, &["claude"])?;
        }
        Action::Pi(_) => {
            if preset != cli::Preset::MultiAgent {
                return Err(format!(
                    "Pi is not installed by the {} preset",
                    preset.name()
                ));
            }
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            sandbox::run_remote(context, project, name, &["pi"])?;
        }
        Action::Paseo(_) => {
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            let endpoint = sandbox::paseo_endpoint_for(context, name)?;
            println!("{endpoint}\npaste that URL into Paseo Desktop as a remote daemon");
            if profile.paseo_relay {
                println!(
                    "Paseo relay is enabled for authenticated remote access; the SSH URL remains available locally."
                );
            }
        }
        Action::Orca(_) => {
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            let link = orca::pairing_link(context, name)?;
            println!(
                "{link}\npaste this private pairing link into Orca Desktop: Settings → Remote Orca Servers → Add Server"
            );
        }
        Action::AuthImport(_) => auth::import(context, project, preset, rocksdb)?,
        Action::AuthStatus(_) => {
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            auth::show_status(context, name, preset)?;
        }
        Action::AuthRenew(request) => {
            match request.provider {
                cli::AuthProvider::OpenAi if !preset.has_codex() => {
                    return Err(format!(
                        "OpenAI renewal is not used by the {} preset",
                        preset.name()
                    ));
                }
                cli::AuthProvider::Anthropic if !preset.has_claude() => {
                    return Err(format!(
                        "Anthropic renewal is not used by the {} preset",
                        preset.name()
                    ));
                }
                cli::AuthProvider::GitHub if !profile.github_proxy => {
                    return Err(
                        "GitHub renewal requires the host-proxied GitHub profile".to_owned()
                    );
                }
                _ => {}
            }
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, false, force)?;
            verify::verify_credentials(context, name)?;
            auth::renew(context, project, name, request.provider)?;
            verify::verify_credentials(context, name)?;
            verify::run(context, project, preset, rocksdb, profile, false)?;
        }
        Action::Audit(_) => sandbox::audit(context, project, preset, rocksdb, profile, force)?,
        Action::Verify(request) => {
            verify::run(context, project, preset, rocksdb, profile, request.live)?
        }
        Action::Repair(_) => {
            println!("==> reconciling sandbox integrations from trusted sources");
            sandbox::ensure_dev(context, project, preset, rocksdb, profile, true, force)?;
            verify::run(context, project, preset, rocksdb, profile, false)?;
            println!("repaired: {name}");
        }
        Action::Diagnose(_) => {
            diagnostics::summarize_latest(context)?;
            health::Ledger::load(context, name, preset, profile)?.print();
        }
        Action::Review(_) => {
            vscode::check_host()?;
            sandbox::ensure_review(context, project, rocksdb, force)?;
            vscode::open(context, &project.review_name, &project.path, false, true)?;
        }
        Action::Inspect(_) => {
            let kind = selected_kind(preset, profile);
            envfile::inspect(context, project, kind, rocksdb, profile)?.print();
        }
        Action::Profile(_) => profile::print(context, project, profile)?,
        Action::Name(_) => println!("{name}"),
        Action::Status(_) => {
            let exists = show_selected_status(context, project, profile)?;
            let ledger = health::Ledger::load(context, name, preset, profile)?;
            ledger.print();
            if exists && ledger.needs_repair() {
                println!("repair: sbxr repair {}", shell_quote_path(&project.path));
            } else if exists && ledger.needs_verification() {
                println!("verify: sbxr verify {}", shell_quote_path(&project.path));
            }
            return Ok(if exists { 0 } else { 1 });
        }
        Action::Stop(_) => {
            process::require("sbx")?;
            vscode::close_for_lifecycle(context, name, &project.path, "stop", "stopped")?;
            sbx::stop(context.preserve_xdg_state, name)?;
        }
        Action::Remove { force, .. } => {
            process::require("sbx")?;
            vscode::close_for_lifecycle(context, name, &project.path, "rm", "removed")?;
            println!("Removing sandbox-local state for {name}; host project files remain.");
            sbx::require_minimum_version(context.preserve_xdg_state)?;
            let definition = envfile::materialize(
                context,
                project,
                selected_kind(preset, profile),
                rocksdb,
                profile,
            )?;
            sbx::env_remove(context.preserve_xdg_state, definition.path(), force)?;
            definition.remove_host_state(&context.state_root)?;
            health::remove(context, name)?;
        }
        Action::CommitPgpSign(_)
        | Action::Setup(_)
        | Action::Doctor(_)
        | Action::Version
        | Action::Help => unreachable!(),
    }
    Ok(0)
}

fn selected_kind(preset: cli::Preset, profile: &profile::Profile) -> envfile::Kind {
    match profile.workspace {
        WorkspaceMode::Shared => envfile::Kind::Development(preset),
        WorkspaceMode::Isolated => envfile::Kind::Isolated(preset),
    }
}

fn open_dev(
    context: &Context,
    project: &Project,
    preset: cli::Preset,
    rocksdb: Option<&RocksDbHost>,
    profile: &profile::Profile,
    force_sync: bool,
    force_plan: bool,
) -> Result<(), String> {
    vscode::check_host()?;
    let name = sandbox::sandbox_name(project, profile);
    if force_sync {
        println!("==> forcing host integration update for {name}");
    }
    let mut state = sandbox::ensure_dev(
        context, project, preset, rocksdb, profile, force_sync, force_plan,
    )?;
    if vscode::open(context, name, &project.path, true, !state.vscode)? {
        state.vscode = true;
        crate::sync::write_bootstrap_state(name, context.preserve_xdg_state, state)?;
        println!("==> sandbox host-integration bootstrap complete");
    }
    print_ready(context, project, profile);
    Ok(())
}

fn show_selected_status(
    context: &Context,
    project: &Project,
    profile: &profile::Profile,
) -> Result<bool, String> {
    process::require("sbx")?;
    let name = sandbox::sandbox_name(project, profile);
    let output = sbx::list_json(context.preserve_xdg_state)?;
    let state = sbx::sandbox_state(&output.stdout, name)?;
    println!(
        "sandbox: {name}\nproject: {}\nstatus: {}",
        project.path.display(),
        state.as_deref().unwrap_or("not found")
    );
    Ok(state.is_some())
}

fn print_ready(context: &Context, project: &Project, profile: &profile::Profile) {
    let name = sandbox::sandbox_name(project, profile);
    println!("ready: ssh {name}.sbx");
    println!(
        "environment: {}",
        envfile::definition_path(context, name).display()
    );
    println!("inspect: sbxr inspect {}", shell_quote_path(&project.path));
}

fn action_ensures_sandbox(action: &Action) -> bool {
    matches!(
        action,
        Action::New(_)
            | Action::Code(_)
            | Action::Update(_)
            | Action::Up(_)
            | Action::Shell(_)
            | Action::Codex(_)
            | Action::Claude(_)
            | Action::Pi(_)
            | Action::Paseo(_)
            | Action::Orca(_)
            | Action::AuthStatus(_)
            | Action::AuthRenew(_)
            | Action::Audit(_)
            | Action::Repair(_)
            | Action::Isolated(_)
            | Action::Wizard(_)
    )
}

fn action_has_report(action: &Action) -> bool {
    action_ensures_sandbox(action)
        || matches!(
            action,
            Action::Remove { .. } | Action::Review(_) | Action::Verify(_)
        )
}

fn action_label(action: &Action) -> &'static str {
    match action {
        Action::New(_) | Action::Code(_) => "new",
        Action::Update(_) => "update",
        Action::Up(_) => "up",
        Action::Shell(_) => "shell",
        Action::Codex(_) => "codex",
        Action::Claude(_) => "claude",
        Action::Pi(_) => "pi",
        Action::Paseo(_) => "paseo",
        Action::Orca(_) => "orca",
        Action::AuthStatus(_) => "auth-status",
        Action::AuthRenew(_) => "auth-renew",
        Action::Audit(_) => "audit",
        Action::Verify(_) => "verify",
        Action::Repair(_) => "repair",
        Action::Isolated(_) => "isolated",
        Action::Wizard(_) => "wizard",
        Action::Review(_) => "review",
        Action::Remove { .. } => "remove",
        _ => "command",
    }
}

fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
