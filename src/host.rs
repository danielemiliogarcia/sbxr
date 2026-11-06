use crate::cli::{Action, Preset};
use crate::environment::{Context, RocksDbHost};
use crate::{embedded, envfile, process, sbx};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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
        if preset.has_claude() {
            match anthropic_oauth_configured(preserve_xdg_state) {
                Ok(true) => println!("ok   Docker host-managed Anthropic OAuth"),
                Ok(false) => println!(
                    "WARN Anthropic OAuth is not yet in Docker's host store; after creating a sandbox, run `sbxr claude .` and `/login` once so Docker can capture it behind the proxy"
                ),
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

    if process::command_exists("gh") {
        let authenticated = Command::new("gh")
            .args(["auth", "status", "--hostname", "github.com"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if authenticated {
            println!("ok   GitHub CLI host authentication for the sandbox proxy");
        } else {
            println!("==> GitHub CLI is not authenticated; starting `gh auth login`");
            if let Err(error) = process::run_checked(
                Command::new("gh").args(["auth", "login", "--hostname", "github.com"]),
                "GitHub CLI login",
            ) {
                failures.push(error);
            }
        }
    } else {
        failures.push(format!("gh is missing\n{}", process::install_hint("gh")));
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

/// Warms Docker Sandboxes' native content-addressed Kits v3 composition cache
/// from the repository's pinned default composition. The disposable sandbox
/// has no workspace and receives no host personalization; personal overlays
/// are still applied only by `sandbox::ensure_dev` after a real sandbox is
/// created. This is deliberately not a saved sandbox template: sbx templates
/// omit the volume-backed Rust toolchain and other persistent kit state.
pub(crate) fn prebuild(context: &Context, preset: Preset) -> Result<(), String> {
    process::require("sbx")?;
    sbx::require_minimum_version(context.preserve_xdg_state)?;
    cleanup_stale_prebuilds(context.preserve_xdg_state)?;
    let profile = crate::profile::Profile::default();
    let kits =
        envfile::selected_local_kits(context, envfile::Kind::Development(preset), false, &profile);
    let sequence = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("host clock is before the Unix epoch: {error}"))?
        .as_nanos();
    let name = format!("sbxr-prebuild-{}-{sequence:x}", std::process::id());
    let arguments = prebuild_arguments(preset, &name, &kits);
    println!(
        "==> warming the local content-addressed sbxr kit-composition cache (no workspace or personal files)"
    );
    let result = process::run_checked(
        sbx::command(context.preserve_xdg_state).args(&arguments),
        "prebuilding the default Kits v3 composition",
    );
    let cleanup = process::run_checked(
        sbx::command(context.preserve_xdg_state).args(["rm", "--force", &name]),
        "removing the disposable cache-warm sandbox",
    );
    result?;
    cleanup?;
    println!(
        "ok   local kit-composition cache warmed; Docker reuses it during matching sandbox creation"
    );
    Ok(())
}

fn cleanup_stale_prebuilds(preserve_xdg_state: bool) -> Result<(), String> {
    let output = sbx::list_json(preserve_xdg_state)?;
    for name in prebuild_sandbox_names(&output.stdout)? {
        if prebuild_owner_running(&name) {
            continue;
        }
        println!("==> removing stale cache-warm sandbox {name}");
        process::run_checked(
            sbx::command(preserve_xdg_state).args(["rm", "--force", &name]),
            "removing a stale cache-warm sandbox",
        )?;
    }
    Ok(())
}

fn prebuild_sandbox_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("could not parse Docker sandbox inventory: {error}"))?;
    let sandboxes = document.as_array().or_else(|| {
        document
            .get("sandboxes")
            .and_then(serde_json::Value::as_array)
    });
    let sandboxes = sandboxes.ok_or("Docker sandbox inventory has no array of sandboxes")?;
    Ok(sandboxes
        .iter()
        .filter_map(|sandbox| sandbox.get("name").and_then(serde_json::Value::as_str))
        .filter(|name| name.starts_with("sbxr-prebuild-"))
        .map(str::to_owned)
        .collect())
}

/// A cache-warm sandbox is owned by a live sbxr process when the PID embedded
/// in its name is running and is an sbxr binary. `ps` works on both Linux and
/// macOS (no `/proc` there), and the command-name check stops a reused PID from
/// protecting a stale sandbox forever.
fn prebuild_owner_running(name: &str) -> bool {
    let Some(pid) = name
        .strip_prefix("sbxr-prebuild-")
        .and_then(|tail| tail.split('-').next())
        .and_then(|pid| pid.parse::<u32>().ok())
    else {
        return false;
    };
    match Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .rsplit('/')
                    .next()
                    .is_some_and(|command| command.starts_with("sbxr"))
        }
        Err(_) => Path::new("/proc").join(pid.to_string()).exists(),
    }
}

fn prebuild_arguments(preset: Preset, name: &str, kits: &[std::path::PathBuf]) -> Vec<String> {
    let workload = match preset {
        Preset::Claude => envfile::CLAUDE_WORKLOAD,
        Preset::Codex | Preset::MultiAgent => envfile::CODEX_WORKLOAD,
    };
    let mut arguments = vec![
        "create".to_owned(),
        workload.to_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--pull".to_owned(),
        "missing".to_owned(),
        "--no-progress".to_owned(),
        "--skills".to_owned(),
        "off".to_owned(),
        "--static-mcp".to_owned(),
        "none".to_owned(),
    ];
    if preset == Preset::MultiAgent {
        arguments.push("--kit".to_owned());
        arguments.push(envfile::CLAUDE_MIXIN.to_owned());
    }
    for kit in kits {
        arguments.push("--kit".to_owned());
        arguments.push(kit.to_string_lossy().into_owned());
    }
    arguments
}

fn openai_oauth_configured(preserve_xdg_state: bool) -> Result<bool, String> {
    oauth_configured(preserve_xdg_state, "openai")
}

fn anthropic_oauth_configured(preserve_xdg_state: bool) -> Result<bool, String> {
    oauth_configured(preserve_xdg_state, "anthropic")
}

fn oauth_configured(preserve_xdg_state: bool, service: &str) -> Result<bool, String> {
    let output = process::output_checked(
        sbx::command(preserve_xdg_state).args(["secret", "ls", "--service", service]),
        &format!("checking {service} OAuth"),
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
    for tool in ["sbx", "code", "ssh", "git", "gh"] {
        if process::command_exists(tool) {
            println!("ok   {tool}");
        } else {
            println!("FAIL {tool} is missing");
            println!("{}", process::install_hint(tool));
            healthy = false;
        }
    }

    if process::command_exists("gh") {
        let ok = Command::new("gh")
            .args(["auth", "status", "--hostname", "github.com"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if ok {
            println!("ok   host GitHub authentication for proxy-managed HTTPS/API access");
        } else {
            println!("FAIL GitHub CLI is not authenticated; run `gh auth login`");
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
        let sbx_version = match sbx::runtime_info(context.preserve_xdg_state) {
            Ok(info) if sbx::validate_minimum_versions(&info).is_ok() => {
                println!(
                    "ok   Docker Sandboxes client {} · server {} · API {} (minimum 0.51.0 for Kits v3)",
                    info.client, info.server, info.api_version
                );
                if info.server_state != "running" {
                    println!(
                        "WARN Docker Sandboxes server is {}; start it with `sbx daemon start --detach` before creating or using a sandbox",
                        info.server_state
                    );
                    None
                } else {
                    match sbx::require_minimum_version(context.preserve_xdg_state) {
                        Ok(version) => Some(version),
                        Err(error) => {
                            println!("FAIL {error}");
                            healthy = false;
                            None
                        }
                    }
                }
            }
            Ok(info) => {
                let error = sbx::validate_minimum_versions(&info).unwrap_err();
                println!("FAIL {error}");
                healthy = false;
                None
            }
            Err(error) => {
                println!("FAIL {error}");
                healthy = false;
                None
            }
        };
        if let Some(version) = sbx_version {
            match sbx::diagnose_json(context.preserve_xdg_state, version) {
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
        if preset.has_claude() {
            match sbx::command(context.preserve_xdg_state)
                .args(["secret", "ls", "--service", "anthropic"])
                .output()
            {
                Ok(output)
                    if output.status.success()
                        && String::from_utf8_lossy(&output.stdout).contains("oauth configured") =>
                {
                    println!("ok   Docker host-managed Anthropic OAuth")
                }
                _ => {
                    println!(
                        "FAIL Anthropic OAuth is missing; create a sandbox, run `sbxr claude .`, and complete `/login` once"
                    );
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
        if status == "fail" {
            if let Some(hint) = check
                .get("hint")
                .and_then(serde_json::Value::as_str)
                .filter(|hint| !hint.is_empty())
            {
                println!("     {hint}");
            }
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

    #[test]
    fn prebuild_recipe_is_workspace_free_and_uses_pinned_kits() {
        let kits = vec![
            std::path::PathBuf::from("/trusted/kits/rust"),
            std::path::PathBuf::from("/trusted/kits/node-runtime"),
        ];
        let arguments = prebuild_arguments(Preset::MultiAgent, "sbxr-prebuild-test", &kits);
        assert_eq!(arguments[0], "create");
        assert_eq!(arguments[1], envfile::CODEX_WORKLOAD);
        assert!(arguments.iter().any(|value| value == envfile::CLAUDE_MIXIN));
        assert!(arguments.iter().any(|value| value == "/trusted/kits/rust"));
        assert!(arguments.windows(2).any(|pair| pair == ["--skills", "off"]));
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--static-mcp", "none"])
        );
        assert!(!arguments.iter().any(|value| value == "."));
        assert!(!arguments.iter().any(|value| value.contains("workspace")));
    }

    #[test]
    fn selects_only_launcher_owned_prebuild_sandboxes() {
        let inventory = br#"[
          {"name":"sbxr-prebuild-999999-abc","status":"running"},
          {"name":"rust-multi-project","status":"running"}
        ]"#;
        assert_eq!(
            prebuild_sandbox_names(inventory).unwrap(),
            ["sbxr-prebuild-999999-abc"]
        );
        assert!(!prebuild_owner_running("sbxr-prebuild-999999-abc"));
        // The test binary is `sbxr-<hash>`, so this process counts as a live owner.
        assert!(prebuild_owner_running(&format!(
            "sbxr-prebuild-{}-abc",
            std::process::id()
        )));
        assert!(!prebuild_owner_running("not-a-prebuild"));
    }
}
