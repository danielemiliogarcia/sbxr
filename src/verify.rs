use crate::cli::Preset;
use crate::environment::{Context, Project};
use crate::profile::Profile;
use crate::{envfile, process, sandbox, sbx};
use serde_json::Value;

const CREDENTIAL_AUDIT: &str = r#"
import base64
import json
import os
from pathlib import Path
import re
import sys

home = Path.home().resolve()
placeholders = {
    "proxy-managed",
    "oai-oat01-proxy-managed",
    "oai-ort01-proxy-managed",
    "sk-ant-oat01-proxy-managed",
    "sk-ant-ort01-proxy-managed",
    "gho_sbxproxymanaged000000000000000000000",
}
token_key = re.compile(r"(?:token|secret|password|api.?key|private.?key)", re.I)
oauth_value_keys = {"access", "refresh"}
findings = []

def allowed(value):
    return value in (None, "") or isinstance(value, str) and value in placeholders

for key in (
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
):
    value = os.environ.get(key)
    if value and not allowed(value):
        findings.append("environment:" + key)

def inspect_json(path):
    if not path.exists():
        return
    label = path.relative_to(home).as_posix()
    try:
        value = json.loads(path.read_text())
        def visit(current, trail):
            if isinstance(current, dict):
                for key, child in current.items():
                    visit(child, trail + [str(key)])
            elif isinstance(current, list):
                for index, child in enumerate(current):
                    visit(child, trail + [str(index)])
            elif current and any(token_key.search(key) or key.lower() in oauth_value_keys for key in trail) and not allowed(current):
                findings.append(label + ":credential")
        visit(value, [])
    except (OSError, ValueError, TypeError):
        findings.append(label + ":unreadable")

for name in (
    ".codex/auth.json",
    ".claude/.credentials.json",
    ".pi/agent/auth.json",
    ".vscode/cli/token.json",
    ".vscode-server/cli/token.json",
):
    inspect_json(home / name)

gh = home / ".config/gh/hosts.yml"
if gh.exists():
    try:
        for match in re.finditer(r"^\s*oauth_token:\s*(\S+)", gh.read_text(), re.M):
            if not allowed(match.group(1).strip("\"'")):
                findings.append(".config/gh/hosts.yml:oauth_token")
    except OSError:
        findings.append(".config/gh/hosts.yml:unreadable")

docker = home / ".docker/config.json"
if docker.exists():
    try:
        value = json.loads(docker.read_text())
        auths = value.get("auths", {})
        if not isinstance(auths, dict):
            raise ValueError("invalid auths")
        for entry in auths.values():
            if not isinstance(entry, dict):
                raise ValueError("invalid auth entry")
            if entry.get("auth"):
                decoded = base64.b64decode(entry["auth"], validate=True).decode()
                if not allowed(decoded.partition(":")[2]):
                    findings.append(".docker/config.json:auths")
            if not allowed(entry.get("identitytoken")):
                findings.append(".docker/config.json:identitytoken")
    except (OSError, ValueError, TypeError, UnicodeError):
        findings.append(".docker/config.json:unreadable")

findings = sorted(set(findings))
print(json.dumps({"supported_credentials_clean": not findings, "findings": findings}))
sys.exit(1 if findings else 0)
"#;

pub(crate) fn report_host_versions(preset: Preset) {
    println!("host/sandbox version report:");
    let mut tools = vec!["rustc", "cargo", "node", "gh", "code"];
    if preset.has_codex() {
        tools.push("codex");
    }
    if preset.has_claude() {
        tools.push("claude");
    }
    if preset == Preset::MultiAgent {
        tools.extend(["pi", "paseo"]);
    }
    for tool in tools {
        let output = std::process::Command::new(tool).arg("--version").output();
        match output {
            Ok(output) if output.status.success() => {
                let line = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("unknown")
                    .trim()
                    .to_owned();
                println!("host {tool}: {line}");
            }
            _ => println!("host {tool}: not installed"),
        }
    }
}

pub(crate) fn run(
    context: &Context,
    project: &Project,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    process::require("sbx")?;
    let inventory = sbx::list_json(context.preserve_xdg_state)?;
    let name = sandbox::sandbox_name(project, profile);
    if sbx::sandbox_state(&inventory.stdout, name)?.is_none() {
        return Err(format!(
            "sandbox {} does not exist; create it with `sbxr up {}` before verification",
            name,
            shell_quote(&project.path)
        ));
    }

    verify_runtime(context, project, preset, profile)?;
    verify_credentials(context, name)?;
    report_tool_versions(context, name, preset, profile)?;
    verify_forwarded_agent(context, name)?;
    println!("verified: {name}");
    Ok(())
}

fn verify_runtime(
    context: &Context,
    project: &Project,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    let name = sandbox::sandbox_name(project, profile);
    let output = sbx::inspect_json(context.preserve_xdg_state, name)?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not parse Docker sandbox inspection: {error}"))?;
    for (field, expected) in [
        ("name", name),
        ("agent", envfile::agent_name(preset, profile)),
        ("workspace", project.path.to_string_lossy().as_ref()),
    ] {
        if value.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(format!(
                "runtime verification failed: sandbox inspection has an unexpected {field}"
            ));
        }
    }
    match value.get("runtime_mounts").and_then(Value::as_array) {
        Some(mounts) if mounts.is_empty() => {}
        Some(_) => {
            return Err(
                "runtime verification failed: unexpected runtime mounts are present".to_owned(),
            );
        }
        None => {
            return Err(
                "runtime verification failed: runtime_mounts is missing or invalid".to_owned(),
            );
        }
    }
    if let Some(kits) = value.get("kits").and_then(Value::as_array) {
        let actual = kits.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        for kit in envfile::selected_kits(
            context,
            match profile.workspace {
                crate::profile::WorkspaceMode::Shared => envfile::Kind::Development(preset),
                crate::profile::WorkspaceMode::Isolated => envfile::Kind::Isolated(preset),
            },
            false,
            profile,
        ) {
            let expected = kit.to_string_lossy();
            if !actual.iter().any(|value| *value == expected) {
                return Err(format!(
                    "runtime verification failed: selected kit {} is missing",
                    kit.display()
                ));
            }
        }
    }
    println!("ok   runtime identity and shared workspace match the sbxr contract");
    Ok(())
}

fn verify_credentials(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let output = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args(["--", "python3", "-I", "-c", CREDENTIAL_AUDIT])
        .output()
        .map_err(|error| format!("could not run the sandbox credential audit: {error}"))?;
    let report: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("sandbox credential audit returned invalid JSON: {error}"))?;
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .ok_or("sandbox credential audit report has no findings array")?;
    let clean = report
        .get("supported_credentials_clean")
        .and_then(Value::as_bool)
        .ok_or("sandbox credential audit report has no clean status")?;
    if clean != findings.is_empty() {
        return Err("sandbox credential audit report is inconsistent".to_owned());
    }
    if !findings.is_empty() || !output.status.success() {
        let locations = findings
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "reusable credentials were found in supported sandbox stores: {locations}\nNo credential values were printed. Remove or revoke those credentials and retry."
        ));
    }
    println!("ok   supported guest credential stores contain no reusable credentials");
    Ok(())
}

fn report_tool_versions(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    let mut tools = vec!["rustc", "cargo", "rust-analyzer", "paseo"];
    if preset.has_codex() {
        tools.push("codex");
    }
    if preset.has_claude() {
        tools.push("claude");
    }
    if preset == Preset::MultiAgent {
        tools.push("pi");
    }
    if profile.github_proxy {
        tools.push("gh");
    }
    if profile.browser {
        tools.push("node");
    }
    for tool in tools {
        let output = process::output_checked(
            sbx::exec_command(context.preserve_xdg_state)
                .arg(sandbox_name)
                .args(["--", tool, "--version"]),
            &format!("checking sandbox {tool} version"),
        )?;
        let version = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_owned();
        println!("ok   {tool}: {version}");
        let expected = match tool {
            "rustc" | "cargo" => Some("1.98.0"),
            "claude" => Some("2.1.239"),
            "pi" => Some("1.4.0"),
            "paseo" => Some("0.8.0"),
            "gh" => Some("2.102.0"),
            _ => None,
        };
        if let Some(expected) = expected
            && !version.contains(expected)
        {
            return Err(format!(
                "sandbox {tool} version drift: expected {expected}, got {version}"
            ));
        }
    }
    if profile.browser {
        verify_package_version(
            context,
            sandbox_name,
            "/opt/sbxr-tools/browser/node_modules/@playwright/mcp/package.json",
            "0.0.83",
            "Playwright MCP",
        )?;
    }
    if profile.orca {
        let status = sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args([
                "--",
                "test",
                "-x",
                "/opt/sbxr-orca/1.4.247/squashfs-root/AppRun",
            ])
            .status()
            .map_err(|error| format!("could not verify Orca: {error}"))?;
        if !status.success() {
            return Err("sandbox Orca 1.4.247 is missing".to_owned());
        }
        println!("ok   Orca: 1.4.247");
    }
    Ok(())
}

fn verify_package_version(
    context: &Context,
    sandbox_name: &str,
    path: &str,
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let output = process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args([
                "--",
                "node",
                "-e",
                "const p=require(process.argv[1]); console.log(p.version)",
                path,
            ]),
        &format!("checking sandbox {label} version"),
    )?;
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if actual != expected {
        return Err(format!(
            "sandbox {label} version drift: expected {expected}, got {actual}"
        ));
    }
    println!("ok   {label}: {actual}");
    Ok(())
}

fn verify_forwarded_agent(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let status = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args(["--", "ssh-add", "-L"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("could not check the forwarded SSH agent: {error}"))?;
    if status.success() {
        println!("ok   forwarded SSH agent is active and has at least one key");
    } else {
        println!(
            "WARN forwarded SSH agent is unavailable or has no keys; GitHub SSH and sandbox SSH signing will not work"
        );
    }
    Ok(())
}

fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_never_embeds_real_credential_prefixes() {
        assert!(!CREDENTIAL_AUDIT.contains("sk-ant-api"));
        assert!(!CREDENTIAL_AUDIT.contains("ghp_"));
        assert!(CREDENTIAL_AUDIT.contains("proxy-managed"));
        assert!(CREDENTIAL_AUDIT.contains(r#"{"access", "refresh"}"#));
    }
}
