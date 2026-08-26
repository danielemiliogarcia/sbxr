use crate::environment::Context;
use crate::{process, sync};
use serde_json::{Map, Value};
use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const REMOTE_SERVER_WAIT: Duration = Duration::from_secs(60);
const REMOTE_SERVER_POLL: Duration = Duration::from_millis(500);
const REMOTE_SERVER_ROOT: &str = "/home/agent/.vscode-server/cli/servers";
const REMOTE_MACHINE_SETTINGS: &str = "/home/agent/.vscode-server/data/Machine/settings.json";
const REMOTE_MACHINE_SETTINGS_DIRECTORY: &str = "/home/agent/.vscode-server/data/Machine";
const REMOTE_WINDOW_CLOSE_WAIT: Duration = Duration::from_secs(30);

pub(crate) fn check_host() -> Result<(), String> {
    process::require("code")?;
    process::require("ssh")?;
    let output = process::output_checked(
        Command::new("code").arg("--list-extensions"),
        "listing VS Code extensions",
    )?;
    let installed = String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.eq_ignore_ascii_case("ms-vscode-remote.remote-ssh"));
    if installed {
        Ok(())
    } else {
        Err("VS Code Remote-SSH extension is missing.\nInstall it with: code --install-extension ms-vscode-remote.remote-ssh".to_owned())
    }
}

pub(crate) fn open(
    context: &Context,
    sandbox_name: &str,
    project: &Path,
    mirror_extensions: bool,
    bootstrap: bool,
) -> Result<bool, String> {
    if bootstrap {
        process::run_checked(
            process::sbx_command(context.preserve_xdg_state).args(["setup", "ssh"]),
            "configuring Docker Sandbox SSH",
        )?;
    }
    process::run_checked(
        process::code_command(context.preserve_xdg_state).args(remote_open_arguments(
            sandbox_name,
            project,
            mirror_extensions,
        )),
        "opening VS Code",
    )?;
    if !bootstrap {
        return Ok(false);
    }
    println!("==> waiting for the VS Code Server on {sandbox_name}.sbx");
    let Some(server_cli) = wait_for_remote_server(context, sandbox_name)? else {
        eprintln!(
            "warning: VS Code Server was not ready after {} seconds; rerun `sbxr vscode` to finish remote setup",
            REMOTE_SERVER_WAIT.as_secs()
        );
        return Ok(false);
    };
    configure_terminal_persistence(context, sandbox_name)?;
    if mirror_extensions {
        sync_extensions(context, sandbox_name, &server_cli)?;
    } else {
        println!("note: host VS Code extensions are not mirrored in review mode");
    }
    Ok(true)
}

pub(crate) fn close_for_stop(
    context: &Context,
    sandbox_name: &str,
    project: &Path,
) -> Result<(), String> {
    if !process::command_exists("code") || !remote_window_open(context, sandbox_name)? {
        return Ok(());
    }
    println!("==> asking VS Code to save and close {sandbox_name}.sbx");
    if !close_remote_windows_with_window_manager(context, sandbox_name)? {
        let (all_windows, matching_windows) = vscode_window_counts(context, sandbox_name)?;
        if all_windows != matching_windows || matching_windows != 1 {
            return Err(format!(
                "could not address the {sandbox_name}.sbx VS Code window without risking another window. Close that Remote-SSH window manually, then retry `sbxr stop`. On Linux/X11, install `wmctrl` to enable targeted automatic closing. The sandbox was not stopped."
            ));
        }
        close_remote_window_with_code(context, sandbox_name, project)?;
    }

    let started = Instant::now();
    loop {
        if !remote_window_open(context, sandbox_name)? {
            thread::sleep(Duration::from_millis(500));
            return Ok(());
        }
        if started.elapsed() >= REMOTE_WINDOW_CLOSE_WAIT {
            return Err(format!(
                "VS Code did not close the {sandbox_name}.sbx window. Resolve any unsaved-file prompt or close that window manually, then retry `sbxr stop`. The sandbox was not stopped so terminal state is not discarded."
            ));
        }
        thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(target_os = "linux")]
fn close_remote_windows_with_window_manager(
    _context: &Context,
    sandbox_name: &str,
) -> Result<bool, String> {
    if !process::command_exists("wmctrl") {
        return Ok(false);
    }
    let output = process::output_checked(
        Command::new("wmctrl").arg("-l"),
        "listing desktop windows before stopping the sandbox",
    )?;
    let listing = String::from_utf8_lossy(&output.stdout);
    let window_ids = remote_window_ids(&listing, sandbox_name);
    for window_id in &window_ids {
        process::run_checked(
            Command::new("wmctrl").args(["-ic", window_id]),
            "asking the sandbox VS Code window to close",
        )?;
    }
    Ok(!window_ids.is_empty())
}

#[cfg(not(target_os = "linux"))]
fn close_remote_windows_with_window_manager(
    _context: &Context,
    _sandbox_name: &str,
) -> Result<bool, String> {
    Ok(false)
}

fn close_remote_window_with_code(
    context: &Context,
    sandbox_name: &str,
    project: &Path,
) -> Result<(), String> {
    process::run_checked(
        process::code_command(context.preserve_xdg_state).args([
            "--remote",
            &format!("ssh-remote+{sandbox_name}.sbx"),
            &project.to_string_lossy(),
        ]),
        "focusing the sandbox VS Code window",
    )?;
    thread::sleep(Duration::from_millis(750));
    process::run_checked(
        process::code_command(context.preserve_xdg_state).args([
            "--reuse-window",
            "--open-url",
            "command:workbench.action.closeWindow",
        ]),
        "closing the sandbox VS Code window",
    )
}

fn remote_open_arguments(
    sandbox_name: &str,
    project: &Path,
    disable_workspace_trust: bool,
) -> Vec<OsString> {
    let mut arguments = Vec::new();
    if disable_workspace_trust {
        arguments.push("--disable-workspace-trust".into());
    }
    arguments.extend([
        OsString::from("--remote"),
        format!("ssh-remote+{sandbox_name}.sbx").into(),
        project.as_os_str().to_owned(),
    ]);
    arguments
}

fn remote_window_open(context: &Context, sandbox_name: &str) -> Result<bool, String> {
    let (_, matching) = vscode_window_counts(context, sandbox_name)?;
    Ok(matching != 0)
}

fn vscode_window_counts(context: &Context, sandbox_name: &str) -> Result<(usize, usize), String> {
    let output = process::output_checked(
        process::code_command(context.preserve_xdg_state).arg("--status"),
        "inspecting VS Code windows before stopping the sandbox",
    )?;
    Ok(status_window_counts(
        &String::from_utf8_lossy(&output.stdout),
        sandbox_name,
    ))
}

fn status_window_counts(status: &str, sandbox_name: &str) -> (usize, usize) {
    let marker = format!("[SSH: {sandbox_name}.sbx]");
    status.lines().fold((0, 0), |(all, matching), line| {
        if !line.contains("window [") {
            (all, matching)
        } else {
            (all + 1, matching + usize::from(line.contains(&marker)))
        }
    })
}

#[cfg(target_os = "linux")]
fn remote_window_ids<'a>(listing: &'a str, sandbox_name: &str) -> Vec<&'a str> {
    let marker = format!("[SSH: {sandbox_name}.sbx]");
    listing
        .lines()
        .filter(|line| line.contains(&marker))
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

fn configure_terminal_persistence(context: &Context, sandbox_name: &str) -> Result<(), String> {
    process::run_checked(
        process::sbx_command(context.preserve_xdg_state).args([
            "exec",
            sandbox_name,
            "mkdir",
            "-p",
            REMOTE_MACHINE_SETTINGS_DIRECTORY,
        ]),
        "creating the remote VS Code machine-settings directory",
    )?;
    let existing = sync::remote_file(
        sandbox_name,
        context.preserve_xdg_state,
        REMOTE_MACHINE_SETTINGS,
    )?;
    let mut settings = match existing {
        Some(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map_err(|error| format!("could not parse remote VS Code machine settings: {error}"))?,
        None => Value::Object(Map::new()),
    };
    let Some(object) = settings.as_object_mut() else {
        return Err("remote VS Code machine settings must be a JSON object".to_owned());
    };
    let wanted = [
        (
            "terminal.integrated.enablePersistentSessions",
            Value::Bool(true),
        ),
        (
            "terminal.integrated.persistentSessionReviveProcess",
            Value::String("onExitAndWindowClose".to_owned()),
        ),
        (
            "terminal.integrated.persistentSessionScrollback",
            Value::Number(10_000.into()),
        ),
        (
            "terminal.integrated.shellIntegration.enabled",
            Value::Bool(true),
        ),
    ];
    let mut changed = false;
    for (key, value) in wanted {
        if object.get(key) != Some(&value) {
            object.insert(key.to_owned(), value);
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("could not serialize remote VS Code settings: {error}"))?;
    sync::upload_file(
        sandbox_name,
        context.preserve_xdg_state,
        REMOTE_MACHINE_SETTINGS,
        &bytes,
    )?;
    println!("==> enabled VS Code terminal session revival and persistent scrollback");
    Ok(())
}

fn sync_extensions(context: &Context, sandbox_name: &str, server_cli: &str) -> Result<(), String> {
    if env::var("SBXR_SYNC_VSCODE_EXTENSIONS").as_deref() == Ok("0") {
        println!("note: host VS Code extension mirroring is disabled");
        return Ok(());
    }
    let output = process::output_checked(
        Command::new("code").args(["--list-extensions", "--show-versions"]),
        "listing versioned host VS Code extensions",
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    let extensions = mirrored_extension_specs(&text);
    if extensions.is_empty() {
        return Ok(());
    }

    let remote = remote_extension_listing(context, sandbox_name, server_cli)?;
    let missing = missing_extension_specs(&extensions, &remote);
    if missing.is_empty() {
        println!("==> all compatible host VS Code extensions are already installed remotely");
        return Ok(());
    }

    println!(
        "==> installing {} missing host VS Code extension(s) on {sandbox_name}.sbx",
        missing.len()
    );
    install_extensions(context, sandbox_name, server_cli, &missing);

    let remote = remote_extension_listing(context, sandbox_name, server_cli)?;
    let retry = missing_extension_specs(&missing, &remote);
    if !retry.is_empty() {
        println!(
            "==> retrying {} VS Code extension installation(s) individually",
            retry.len()
        );
        for extension in retry {
            install_extensions(context, sandbox_name, server_cli, &[extension]);
        }
    }

    let remote = remote_extension_listing(context, sandbox_name, server_cli)?;
    let unresolved = missing_extension_specs(&missing, &remote);
    if unresolved.is_empty() {
        println!("==> verified remote VS Code extension inventory");
    } else {
        eprintln!(
            "warning: {} extension(s) remain unavailable on this remote:",
            unresolved.len()
        );
        for extension in unresolved {
            eprintln!("  {extension}");
        }
        eprintln!("These extensions may be incompatible with the sandbox OS or architecture.");
    }
    Ok(())
}

fn wait_for_remote_server(context: &Context, sandbox_name: &str) -> Result<Option<String>, String> {
    let started = Instant::now();
    loop {
        if let Some(path) = find_remote_server(context, sandbox_name)? {
            return Ok(Some(path));
        }
        if started.elapsed() >= REMOTE_SERVER_WAIT {
            return Ok(None);
        }
        thread::sleep(REMOTE_SERVER_POLL);
    }
}

fn find_remote_server(context: &Context, sandbox_name: &str) -> Result<Option<String>, String> {
    let output = process::sbx_command(context.preserve_xdg_state)
        .arg("exec")
        .arg(sandbox_name)
        .arg("--")
        .args([
            "find",
            REMOTE_SERVER_ROOT,
            "-path",
            "*/server/bin/code-server",
            "-not",
            "-path",
            "*.staging*",
            "-type",
            "f",
            "-print",
            "-quit",
        ])
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not inspect the remote VS Code Server: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!path.is_empty()).then_some(path))
}

fn remote_extension_listing(
    context: &Context,
    sandbox_name: &str,
    server_cli: &str,
) -> Result<String, String> {
    let output = process::output_checked(
        process::sbx_command(context.preserve_xdg_state).args([
            "exec",
            sandbox_name,
            "--",
            server_cli,
            "--list-extensions",
            "--show-versions",
        ]),
        "listing remote VS Code extensions",
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn install_extensions(
    context: &Context,
    sandbox_name: &str,
    server_cli: &str,
    extensions: &[&str],
) {
    let mut command = process::sbx_command(context.preserve_xdg_state);
    command
        .arg("exec")
        .arg(sandbox_name)
        .arg("--")
        .arg(server_cli)
        .arg("--force");
    for extension in extensions {
        command.arg("--install-extension").arg(extension);
    }
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(_) => eprintln!("warning: one or more VS Code extensions failed to install remotely"),
        Err(error) => {
            eprintln!("warning: could not install VS Code extensions remotely ({error})")
        }
    }
}

fn mirrored_extension_specs(listing: &str) -> Vec<&str> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let identifier = line.split_once('@').map_or(*line, |(name, _)| name);
            !identifier
                .to_ascii_lowercase()
                .starts_with("ms-vscode-remote.")
        })
        .collect()
}

fn missing_extension_specs<'a>(wanted: &[&'a str], installed: &str) -> Vec<&'a str> {
    let installed: Vec<_> = installed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    wanted
        .iter()
        .copied()
        .filter(|wanted| {
            !installed
                .iter()
                .any(|installed| installed.eq_ignore_ascii_case(wanted))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_host_only_vscode_extensions() {
        let listing = "rust-lang.rust-analyzer@1.2.3\nMS-VSCODE-REMOTE.remote-ssh@2.0.0\n";
        assert_eq!(
            mirrored_extension_specs(listing),
            vec!["rust-lang.rust-analyzer@1.2.3"]
        );
    }

    #[test]
    fn opens_trusted_remote_folder_without_forcing_a_fresh_window() {
        let arguments = remote_open_arguments("rust-project", Path::new("/work/project"), true);
        assert_eq!(
            arguments,
            vec![
                OsString::from("--disable-workspace-trust"),
                OsString::from("--remote"),
                OsString::from("ssh-remote+rust-project.sbx"),
                OsString::from("/work/project")
            ]
        );
        assert!(!arguments.iter().any(|argument| argument == "--new-window"));
    }

    #[test]
    fn keeps_workspace_trust_enabled_for_review_windows() {
        let arguments = remote_open_arguments("rust-review", Path::new("/work/project"), false);
        assert!(
            !arguments
                .iter()
                .any(|argument| argument == "--disable-workspace-trust")
        );
    }

    #[test]
    fn identifies_only_the_requested_remote_vscode_window() {
        let status = r#"
    0  1000  123 window [1] (local - Visual Studio Code)
    0  1000  456 window [2] (app [SSH: rust-multi-app-abcd1234.sbx] - Visual Studio Code)
"#;
        assert_eq!(
            status_window_counts(status, "rust-multi-app-abcd1234"),
            (2, 1)
        );
        assert_eq!(
            status_window_counts(status, "rust-multi-other-deadbeef"),
            (2, 0)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn identifies_only_requested_remote_desktop_window_ids() {
        let listing = "\
0x06600004  0 host sbxr - Visual Studio Code\n\
0x06600024  0 host smoke [SSH: rust-multi-smoke-a1.sbx] - Visual Studio Code\n\
0x0660002f  0 host other [SSH: rust-multi-other-b2.sbx] - Visual Studio Code\n";
        assert_eq!(
            remote_window_ids(listing, "rust-multi-smoke-a1"),
            vec!["0x06600024"]
        );
    }

    #[test]
    fn identifies_missing_or_version_mismatched_remote_extensions() {
        let wanted = vec![
            "rust-lang.rust-analyzer@1.2.3",
            "vadimcn.vscode-lldb@1.0.0",
            "eamodio.gitlens@2.0.0",
        ];
        let installed = "RUST-LANG.RUST-ANALYZER@1.2.3\neamodio.gitlens@1.9.0\n";
        assert_eq!(
            missing_extension_specs(&wanted, installed),
            vec!["vadimcn.vscode-lldb@1.0.0", "eamodio.gitlens@2.0.0"]
        );
    }
}
