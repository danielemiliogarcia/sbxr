use crate::environment::Context;
use crate::process;
use std::env;
use std::path::Path;
use std::process::Command;

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
) -> Result<(), String> {
    check_host()?;
    process::run_checked(
        process::sbx_command(context.preserve_xdg_state).args(["setup", "ssh"]),
        "configuring Docker Sandbox SSH",
    )?;
    if mirror_extensions {
        sync_extensions(context, sandbox_name)?;
    } else {
        println!("note: host VS Code extensions are not mirrored in review mode");
    }
    let remote = format!("ssh-remote+{sandbox_name}.sbx");
    process::run_checked(
        process::code_command(context.preserve_xdg_state)
            .arg("--new-window")
            .arg("--remote")
            .arg(remote)
            .arg(project),
        "opening VS Code",
    )
}

fn sync_extensions(context: &Context, sandbox_name: &str) -> Result<(), String> {
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

    println!(
        "==> mirroring {} host VS Code extensions to {sandbox_name}.sbx",
        extensions.len()
    );
    let mut command = process::code_command(context.preserve_xdg_state);
    command
        .arg("--remote")
        .arg(format!("ssh-remote+{sandbox_name}.sbx"));
    for extension in &extensions {
        command.arg("--install-extension").arg(extension);
    }
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(_) => eprintln!(
            "warning: one or more host extensions were incompatible with the remote; opening VS Code anyway"
        ),
        Err(error) => {
            eprintln!("warning: could not mirror host extensions ({error}); opening VS Code anyway")
        }
    }
    Ok(())
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
}
