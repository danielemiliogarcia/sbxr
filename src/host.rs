use crate::cli::{Action, Preset};
use crate::environment::{Context, RocksDbHost};
use crate::{embedded, envfile, process, sbx};
use std::fs;
use std::io::{self, Write};
use std::process::{Command, Stdio};

pub(crate) fn setup(preset: Preset, preserve_xdg_state: bool) -> Result<(), String> {
    println!("sbxr setup: configuring missing interactive host prerequisites");
    println!("note: this command never installs system packages or invokes sudo");

    let mut failures = Vec::new();

    if process::command_exists("sbx") {
        let ready = sbx::command(preserve_xdg_state)
            .args(["ls", "--json"])
            .output()
            .is_ok_and(|output| output.status.success());
        if ready {
            println!("ok   Docker sign-in and sandbox daemon");
        } else {
            println!("==> Docker Sandboxes is not ready; starting `sbx login`");
            if let Err(error) = process::run_checked(
                sbx::command(preserve_xdg_state).arg("login"),
                "Docker sign-in",
            ) {
                failures.push(error);
            } else {
                match sbx::command(preserve_xdg_state)
                    .args(["ls", "--json"])
                    .output()
                {
                    Ok(output) if output.status.success() => {
                        println!("ok   Docker sign-in and sandbox daemon")
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        failures.push(format!(
                            "Docker sign-in completed, but Docker Sandboxes is not ready: {}\nRun `sbx diagnose`.",
                            stderr.trim()
                        ));
                    }
                    Err(error) => failures.push(format!(
                        "Docker sign-in completed, but readiness could not be checked: {error}\nRun `sbx diagnose`."
                    )),
                }
            }
        }

        if preset.has_codex() {
            match openai_oauth_configured(preserve_xdg_state) {
                Ok(true) => println!("ok   Docker host-managed OpenAI OAuth"),
                Ok(false) => {
                    println!("==> OpenAI OAuth is missing; starting browser authorization");
                    if let Err(error) = process::run_checked(
                        sbx::command(preserve_xdg_state)
                            .args(["secret", "set", "openai", "--oauth"]),
                        "OpenAI OAuth setup",
                    ) {
                        failures.push(error);
                    } else if openai_oauth_configured(preserve_xdg_state).unwrap_or(false) {
                        println!("ok   Docker host-managed OpenAI OAuth");
                    } else {
                        failures.push(
                            "OpenAI OAuth command completed, but OAuth is not reported as configured"
                                .to_owned(),
                        );
                    }
                }
                Err(error) => failures.push(error),
            }
        }
        if let Err(error) = sbx::require_minimum_version(preserve_xdg_state) {
            failures.push(error);
        } else if let Err(error) = sbx::setup_ssh(preserve_xdg_state) {
            failures.push(error);
        } else {
            println!("ok   Docker-managed *.sbx SSH configuration");
        }
    } else {
        failures.push(format!("sbx is missing\n{}", process::install_hint("sbx")));
    }

    if process::command_exists("code") {
        match vscode_remote_ssh_installed(preserve_xdg_state) {
            Ok(true) => println!("ok   VS Code Remote-SSH extension"),
            Ok(false) => {
                println!("==> VS Code Remote-SSH is missing; installing it");
                if let Err(error) = process::run_checked(
                    process::code_command(preserve_xdg_state)
                        .args(["--install-extension", "ms-vscode-remote.remote-ssh"]),
                    "installing VS Code Remote-SSH extension",
                ) {
                    failures.push(error);
                } else if vscode_remote_ssh_installed(preserve_xdg_state).unwrap_or(false) {
                    println!("ok   VS Code Remote-SSH extension");
                } else {
                    failures.push(
                        "VS Code reported a successful extension install, but Remote-SSH is still missing"
                            .to_owned(),
                    );
                }
            }
            Err(error) => failures.push(error),
        }
    } else {
        failures.push(format!(
            "code is missing\n{}",
            process::install_hint("code")
        ));
    }

    if process::command_exists("ssh") {
        println!("ok   ssh");
    } else {
        failures.push(format!("ssh is missing\n{}", process::install_hint("ssh")));
    }

    if process::command_exists("git") {
        println!("ok   git");
    } else {
        println!("WARN git is missing; review mode will be unavailable");
        println!("{}", process::install_hint("git"));
    }

    if cfg!(target_os = "linux") {
        if fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok()
        {
            println!("ok   KVM is accessible");
        } else {
            failures.push(
                "/dev/kvm is not accessible; Docker Sandboxes cannot start microVMs\nSee: https://docs.docker.com/ai/sandboxes/install/"
                    .to_owned(),
            );
        }
    }

    if failures.is_empty() {
        println!("setup complete; run `sbxr doctor` for the full diagnostic");
        Ok(())
    } else {
        io::stdout()
            .flush()
            .map_err(|error| format!("could not flush setup output: {error}"))?;
        Err(format!(
            "setup is incomplete:\n\n{}\n\nInstall or fix the prerequisites above, then rerun `sbxr setup`.",
            failures.join("\n\n")
        ))
    }
}

fn openai_oauth_configured(preserve_xdg_state: bool) -> Result<bool, String> {
    let output = process::output_checked(
        sbx::command(preserve_xdg_state).args(["secret", "ls", "--service", "openai"]),
        "checking OpenAI OAuth",
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).contains("oauth configured"))
}

fn vscode_remote_ssh_installed(preserve_xdg_state: bool) -> Result<bool, String> {
    let output = process::output_checked(
        process::code_command(preserve_xdg_state).arg("--list-extensions"),
        "listing VS Code extensions",
    )?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.eq_ignore_ascii_case("ms-vscode-remote.remote-ssh")))
}

pub(crate) fn preflight(action: &Action) -> Result<(), String> {
    let mut failures = Vec::new();

    for tool in action.required_host_commands() {
        if !process::command_exists(tool) {
            failures.push(format!(
                "{tool} is missing\n{}",
                process::install_hint(tool)
            ));
        }
    }

    if action.needs_vscode_remote_ssh() && process::command_exists("code") {
        match Command::new("code").arg("--list-extensions").output() {
            Ok(output) if output.status.success() => {
                let installed = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("ms-vscode-remote.remote-ssh"));
                if !installed {
                    failures.push(
                        "VS Code Remote-SSH extension is missing\nInstall it with:\n  code --install-extension ms-vscode-remote.remote-ssh"
                            .to_owned(),
                    );
                }
            }
            Ok(output) => failures.push(format!(
                "VS Code is installed, but its CLI could not list extensions ({})\n{}",
                output.status,
                process::install_hint("code")
            )),
            Err(error) => failures.push(format!(
                "VS Code is installed, but its CLI could not list extensions: {error}\n{}",
                process::install_hint("code")
            )),
        }
    }

    if cfg!(target_os = "linux")
        && action.needs_kvm()
        && fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_err()
    {
        failures.push(
            "/dev/kvm is not accessible; Docker Sandboxes cannot start microVMs\nSee: https://docs.docker.com/ai/sandboxes/install/"
                .to_owned(),
        );
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "host preflight failed; no action was taken:\n\n{}\n\nInstall system prerequisites above, or run `sbxr setup` to configure missing interactive prerequisites. Then retry. Run `sbxr doctor` for the complete host diagnostic.",
            failures.join("\n\n")
        ))
    }
}

pub(crate) fn doctor(context: &Context, preset: Preset, rocksdb: Option<&RocksDbHost>) -> bool {
    let mut healthy = true;
    for tool in ["sbx", "code", "ssh", "git"] {
        if process::command_exists(tool) {
            println!("ok   {tool}");
        } else {
            println!("FAIL {tool} is missing");
            println!("{}", process::install_hint(tool));
            healthy = false;
        }
    }

    if process::command_exists("ssh") {
        match process::selected_ssh_agent_socket() {
            Some(socket) if process::ssh_agent_has_keys(socket) => {
                println!(
                    "ok   host SSH agent has a key for Git authentication and SSH-format signing"
                );
                if process::configured_ssh_agent_socket().as_deref() != Some(socket) {
                    println!(
                        "info selected live SSH agent {}; configured SSH_AUTH_SOCK is unavailable",
                        std::path::Path::new(socket).display()
                    );
                }
            }
            _ => println!(
                "WARN no usable host SSH agent key; run `ssh-add -L`, load a key with `ssh-add` if needed, then retry"
            ),
        }
    }

    println!("info Docker Engine and Docker Desktop are not required");
    if let Some(rocksdb) = rocksdb {
        println!(
            "ok   host RocksDB prefix {} (read-only opt-in)",
            rocksdb.prefix.display()
        );
    }
    if let Err(error) = envfile::validate_state_root(context) {
        println!("FAIL {error}");
        healthy = false;
    }

    if process::command_exists("code") {
        match Command::new("code").arg("--list-extensions").output() {
            Ok(output)
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .any(|line| line.eq_ignore_ascii_case("ms-vscode-remote.remote-ssh")) =>
            {
                println!("ok   VS Code Remote-SSH extension")
            }
            _ => {
                println!("FAIL VS Code Remote-SSH extension is missing");
                println!("Run `sbxr setup`, or install it with:");
                println!("  code --install-extension ms-vscode-remote.remote-ssh");
                healthy = false;
            }
        }
    }

    if process::command_exists("sbx") {
        match sbx::require_minimum_version(context.preserve_xdg_state) {
            Ok(version) => println!("ok   Docker Sandboxes {version} (minimum 0.39.0)"),
            Err(error) => {
                println!("FAIL {error}");
                healthy = false;
            }
        }
        match sbx::diagnose_json(context.preserve_xdg_state) {
            Ok(output) => match print_docker_diagnostics(&output.stdout) {
                Ok(result) => healthy &= result,
                Err(error) => {
                    println!("FAIL {error}");
                    healthy = false;
                }
            },
            Err(error) => {
                println!("FAIL {error}");
                healthy = false;
            }
        }
        if preset.has_codex() {
            match sbx::command(context.preserve_xdg_state)
                .args(["secret", "ls", "--service", "openai"])
                .output()
            {
                Ok(output)
                    if output.status.success()
                        && String::from_utf8_lossy(&output.stdout).contains("oauth configured") =>
                {
                    println!("ok   Docker host-managed OpenAI OAuth")
                }
                _ => {
                    println!("FAIL OpenAI OAuth is missing; run `sbxr setup`");
                    healthy = false;
                }
            }
        }
        match sbx::command(context.preserve_xdg_state)
            .args(["policy", "ls"])
            .output()
        {
            Ok(output)
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("default-deny-all") =>
            {
                println!("ok   global network policy is deny-all")
            }
            _ => println!(
                "WARN global network policy is not deny-all; isolation still applies, but egress is broader"
            ),
        }

        for kit in embedded::KIT_NAMES {
            let path = context.kit_root.join(kit);
            let status = sbx::command(context.preserve_xdg_state)
                .args(["kit", "validate"])
                .arg(&path)
                .stdout(Stdio::null())
                .status();
            if matches!(status, Ok(status) if status.success()) {
                println!("ok   kit {kit}");
            } else {
                println!("FAIL kit {kit}: {}", path.display());
                healthy = false;
            }
        }
    }
    healthy
}

fn print_docker_diagnostics(json: &[u8]) -> Result<bool, String> {
    let document: serde_json::Value = serde_json::from_slice(json)
        .map_err(|error| format!("could not parse `sbx diagnose` JSON: {error}"))?;
    let checks = document
        .get("checks")
        .and_then(serde_json::Value::as_array)
        .ok_or("`sbx diagnose` JSON has no checks array")?;
    let mut healthy = true;
    for check in checks {
        let name = check
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unnamed check");
        let status = check
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("fail");
        let message = check
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("no diagnostic message");
        let label = match status {
            "pass" | "skip" => "ok  ",
            "warn" => "WARN",
            _ => {
                healthy = false;
                "FAIL"
            }
        };
        println!("{label} Docker: {name} — {message}");
        if status == "fail"
            && let Some(hint) = check
                .get("hint")
                .and_then(serde_json::Value::as_str)
                .filter(|hint| !hint.is_empty())
        {
            println!("     {hint}");
        }
    }
    Ok(healthy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_diagnostic_failure_changes_health() {
        let json = br#"{
          "checks": [
            {"name":"Daemon", "status":"pass", "message":"healthy"},
            {"name":"SSH", "status":"fail", "message":"missing", "hint":"run setup"}
          ]
        }"#;
        assert!(!print_docker_diagnostics(json).unwrap());
    }
}
