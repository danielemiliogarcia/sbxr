use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost};
use crate::profile::Profile;
use crate::{auth, envfile, git_trust, health, process, sandbox, sbx};
use serde_json::Value;
use std::process::Stdio;

const PINNED_RUST_VERSION: &str = "1.99.1";
const PINNED_PI_VERSION: &str = "1.4.0";
const PINNED_PASEO_VERSION: &str = "0.8.0";
const PINNED_NODE_VERSION: &str = "22.23.4";
const PINNED_GH_VERSION: &str = "2.102.0";
const PINNED_BROWSER_VERSION: &str = "0.0.83";
const PINNED_ORCA_VERSION: &str = "1.4.247";
const LIVE_CLAUDE_MODEL: &str = "haiku";
const LIVE_PI_MODEL: &str = "openai-codex/gpt-6-luna";

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
    "Bearer proxy-managed",
    "oai-oat01-proxy-managed",
    "oai-ort01-proxy-managed",
    "sk-ant-oat01-proxy-managed",
    "sk-ant-ort01-proxy-managed",
    "gho_sbxproxymanaged000000000000000000000",
}
oauth_value_keys = {"access", "refresh"}
findings = []

native_codex_auth = {
    "auth_mode": "chatgptAuthTokens",
    "OPENAI_API_KEY": None,
    "tokens": {
        "id_token": "proxy-managed.e30.proxy-managed",
        "access_token": "proxy-managed",
        "refresh_token": "",
        "account_id": None,
    },
    "last_refresh": "1970-01-01T00:00:00Z",
}
managed_pi_auth = {
    "openai-codex": {
        "type": "oauth",
        "access": "oai-oat01-proxy-managed",
        "refresh": "proxy-managed",
        "expires": 4102444800000,
        "accountId": "proxy-managed",
        "source": "docker-sbx-proxy",
    },
    "anthropic": {
        "type": "oauth",
        "access": "sk-ant-oat01-proxy-managed",
        "refresh": "sk-ant-ort01-proxy-managed",
        "expires": 4102444800000,
        "source": "docker-sbx-proxy",
    },
}

def native_claude_auth(value):
    if not isinstance(value, dict) or set(value) != {"claudeAiOauth"}:
        return False
    oauth = value.get("claudeAiOauth")
    required = {"accessToken", "expiresAt", "refreshToken", "scopes"}
    optional = {"refreshTokenExpiresAt", "subscriptionType", "rateLimitTier"}
    return (
        isinstance(oauth, dict)
        and required.issubset(oauth)
        and set(oauth).issubset(required | optional)
        and oauth.get("accessToken") == "sk-ant-oat01-proxy-managed"
        and oauth.get("refreshToken") == "sk-ant-ort01-proxy-managed"
        and isinstance(oauth.get("expiresAt"), int)
        and (
            "refreshTokenExpiresAt" not in oauth
            or isinstance(oauth.get("refreshTokenExpiresAt"), int)
        )
        and all(
            key not in oauth or isinstance(oauth.get(key), str)
            for key in ("subscriptionType", "rateLimitTier")
        )
        and isinstance(oauth.get("scopes"), list)
        and all(isinstance(scope, str) for scope in oauth["scopes"])
    )

def label_for(path, fallback=None):
    try:
        return path.relative_to(home).as_posix()
    except ValueError:
        return fallback or "external-credential-store"

def has_symlink_component(path):
    # Fail closed if any ancestor redirects credential storage outside the
    # expected guest tree. The current Docker guest has ordinary /home paths.
    candidate = path.absolute()
    for component in (candidate, *candidate.parents):
        if component == Path(component.anchor):
            break
        try:
            if component.is_symlink():
                return True
        except OSError:
            return True
    return False

def safe_file(path, label):
    try:
        if has_symlink_component(path):
            findings.append(label + ":symlink")
            return False
        if not path.exists():
            return False
        stat = path.stat()
        if not path.is_file() or stat.st_uid != os.getuid():
            findings.append(label + ":unsafe-owner-or-type")
            return False
        return True
    except OSError:
        findings.append(label + ":unreadable")
        return False

def allowed(value):
    return value in (None, "") or isinstance(value, str) and value in placeholders

def credential_key(key):
    compact = re.sub(r"[^a-z0-9]", "", key.lower())
    exact = compact in {
        "authorization", "cookie", "token", "tokens", "secret", "secrets",
        "password", "passwords", "apikey", "apikeys", "privatekey",
        "privatekeys", "credential", "credentials",
    }
    singular = compact.endswith(
        ("token", "secret", "password", "apikey", "privatekey", "cookie")
    )
    composite_plural = compact.endswith(
        ("secrets", "passwords", "apikeys", "privatekeys", "credentials")
    ) and compact not in {"secrets", "passwords", "apikeys", "privatekeys", "credentials"}
    qualified_plural = any(
        compact.endswith(qualifier + suffix)
        for qualifier in (
            "access", "api", "auth", "bearer", "client", "id", "identity",
            "oauth", "refresh", "session",
        )
        for suffix in ("tokens", "cookies")
    )
    return exact or singular or composite_plural or qualified_plural

config_reference = re.compile(r"^\$(?:[A-Za-z_][A-Za-z0-9_]*|\{[A-Za-z_][A-Za-z0-9_]*\})$")

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

def inspect_json(path, fallback=None, codex=False, claude=False, pi=False, private=False):
    label = label_for(path, fallback)
    if not safe_file(path, label):
        return
    try:
        value = json.loads(path.read_text())
        if not isinstance(value, dict):
            raise ValueError("expected an object")
        if private and path.stat().st_mode & 0o077:
            findings.append(label + ":unsafe-permissions")
        if codex:
            legacy = value == {"OPENAI_API_KEY": "proxy-managed"}
            if value == native_codex_auth or legacy:
                return
            findings.append(label + ":credential")
            return
        if claude:
            if native_claude_auth(value):
                return
            findings.append(label + ":credential")
            return
        if pi and path.name == "auth.json":
            # Fail closed if Pi rewrites either managed provider with a shape
            # other than the exact proxy sentinel schema installed by sbxr.
            for provider, expected in managed_pi_auth.items():
                if provider in value and value[provider] != expected:
                    findings.append(label + ":managed-schema")
        def visit(current, trail):
            if isinstance(current, dict):
                for key, child in current.items():
                    visit(child, trail + [str(key)])
            elif isinstance(current, list):
                for index, child in enumerate(current):
                    visit(child, trail + [str(index)])
            elif current and any(
                credential_key(key)
                or key.lower() in oauth_value_keys
                for key in trail
            ) and not allowed(current) and not (
                pi
                and path.name == "models.json"
                and isinstance(current, str)
                and config_reference.fullmatch(current)
            ):
                findings.append(label + ":credential")
        visit(value, [])
    except (OSError, ValueError, TypeError):
        findings.append(label + ":unreadable")

for name in (
    ".claude/.credentials.json",
    ".claude/.config.json",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".claude.json",
    ".pi/agent/auth.json",
    ".pi/agent/models.json",
    ".pi/agent/settings.json",
    ".vscode/cli/token.json",
    ".vscode-server/cli/token.json",
):
    inspect_json(
        home / name,
        claude=name == ".claude/.credentials.json",
        pi=name.startswith(".pi/"),
        private=name.endswith("auth.json") or name.endswith(".credentials.json"),
    )

def configured_directory(variable):
    raw = os.environ.get(variable)
    if not raw:
        return None
    directory = Path(raw).expanduser()
    try:
        if has_symlink_component(directory):
            findings.append("environment:" + variable + ":symlink")
            return None
        if directory.exists():
            stat = directory.stat()
            if not directory.is_dir() or stat.st_uid != os.getuid():
                findings.append("environment:" + variable + ":unsafe-owner-or-type")
                return None
        return directory.resolve()
    except OSError:
        findings.append("environment:" + variable + ":unreadable")
        return None

codex_directories = {home / ".codex"}
custom_codex = configured_directory("CODEX_HOME")
if custom_codex:
    codex_directories.add(custom_codex)
for directory in codex_directories:
    inspect_json(
        directory / "auth.json",
        "CODEX_HOME/auth.json",
        codex=True,
        private=True,
    )

directory = configured_directory("CLAUDE_CONFIG_DIR")
if directory:
    inspect_json(
        directory / ".credentials.json",
        "CLAUDE_CONFIG_DIR/.credentials.json",
        claude=True,
        private=True,
    )

directory = configured_directory("PI_CODING_AGENT_DIR")
if directory:
    for name in ("auth.json", "models.json", "settings.json"):
        inspect_json(
            directory / name,
            f"PI_CODING_AGENT_DIR/{name}",
            pi=True,
            private=name == "auth.json",
        )

gh = home / ".config/gh/hosts.yml"
if safe_file(gh, ".config/gh/hosts.yml"):
    try:
        if gh.stat().st_mode & 0o077:
            findings.append(".config/gh/hosts.yml:unsafe-permissions")
        for match in re.finditer(r"^\s*oauth_token:\s*(\S+)", gh.read_text(), re.M):
            if not allowed(match.group(1).strip("\"'")):
                findings.append(".config/gh/hosts.yml:oauth_token")
    except OSError:
        findings.append(".config/gh/hosts.yml:unreadable")

docker = home / ".docker/config.json"
if safe_file(docker, ".docker/config.json"):
    try:
        if docker.stat().st_mode & 0o077:
            findings.append(".docker/config.json:unsafe-permissions")
        value = json.loads(docker.read_text())
        auths = value.get("auths", {})
        if not isinstance(auths, dict):
            raise ValueError("invalid auths")
        if value.get("credsStore") or value.get("credHelpers"):
            findings.append(".docker/config.json:credential-helper")
        for entry in auths.values():
            if not isinstance(entry, dict):
                raise ValueError("invalid auth entry")
            if entry.get("auth"):
                decoded = base64.b64decode(entry["auth"], validate=True).decode()
                _, separator, password = decoded.partition(":")
                if not separator or not allowed(password):
                    findings.append(".docker/config.json:auths")
            if not allowed(entry.get("identitytoken")):
                findings.append(".docker/config.json:identitytoken")
            if not allowed(entry.get("registrytoken")):
                findings.append(".docker/config.json:registrytoken")
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
    rocksdb: Option<&RocksDbHost>,
    profile: &Profile,
    live: bool,
) -> Result<(), String> {
    process::require("sbx")?;
    let inventory = sbx::list_json(context.preserve_xdg_state)?;
    let name = sandbox::sandbox_name(project, profile);
    let mut ledger = health::Ledger::load(context, name, preset, profile)?;
    if sbx::sandbox_state(&inventory.stdout, name)?.is_none() {
        return Err(format!(
            "sandbox {} does not exist; create it with `sbxr up {}` before verification",
            name,
            shell_quote(&project.path)
        ));
    }
    ledger.begin_verification()?;

    verify_host_git_trust(&mut ledger, project)?;

    checked(
        &mut ledger,
        "runtime",
        "sandbox contract matches",
        verify_runtime(context, project, preset, rocksdb, profile),
    )?;
    checked(
        &mut ledger,
        "guest-credentials",
        "known stores contain managed sentinels only",
        verify_credentials(context, name),
    )?;
    if preset.has_codex() {
        checked(
            &mut ledger,
            "openai",
            if live {
                "live model request succeeded"
            } else {
                "host OAuth binding configured"
            },
            verify_openai(context, name, live),
        )?;
    }
    if preset.has_claude() {
        checked(
            &mut ledger,
            "anthropic",
            if live {
                "proxy route and live model request succeeded"
            } else {
                "proxy route accepted the guest sentinel"
            },
            verify_anthropic(context, name, live),
        )?;
    }
    if profile.github_proxy {
        checked(
            &mut ledger,
            "github",
            "proxied GitHub CLI authentication succeeded",
            verify_github(context, name),
        )?;
    }
    if preset == Preset::MultiAgent {
        checked(
            &mut ledger,
            "pi",
            if live {
                "managed sentinels and live model request succeeded"
            } else {
                "managed provider sentinels verified"
            },
            verify_pi(context, name, live),
        )?;
    }
    checked(
        &mut ledger,
        "rust-toolchain",
        "managed and project toolchains match",
        verify_rust_toolchains(context, name, project, preset, profile),
    )?;
    checked(
        &mut ledger,
        "paseo",
        "pinned CLI is runnable",
        verify_command_version(context, name, "paseo", PINNED_PASEO_VERSION),
    )?;
    if profile.paseo_relay {
        checked(
            &mut ledger,
            "paseo-relay",
            "configuration and scoped policy enabled",
            verify_paseo_relay(context, name),
        )?;
    }
    if profile.browser {
        checked(
            &mut ledger,
            "browser",
            "pinned MCP package and launcher installed",
            verify_browser(context, name),
        )?;
    }
    if profile.orca {
        checked(
            &mut ledger,
            "orca",
            "pinned server and launcher installed",
            verify_orca(context, name),
        )?;
    }
    if verify_forwarded_agent(context, name)? {
        ledger.mark(
            "ssh-agent",
            health::Status::Verified,
            "forwarded agent has a key",
        )?;
    } else {
        ledger.mark(
            "ssh-agent",
            health::Status::NeedsAttention,
            "forwarded agent unavailable or empty",
        )?;
    }
    if live {
        checked(
            &mut ledger,
            "guest-credentials",
            "post-request audit remains clean",
            verify_credentials(context, name),
        )?;
    }
    ledger.print();
    println!("verified: {name}");
    Ok(())
}

pub(crate) fn verify_host_git_trust(
    ledger: &mut health::Ledger,
    project: &Project,
) -> Result<(), String> {
    let result = git_trust::audit_with_trusted_host(&project.path);
    match result {
        Ok(None) => ledger.mark(
            "host-git-trust",
            health::Status::Verified,
            "project has no Git metadata",
        ),
        Ok(Some(audit)) => {
            audit.print();
            let status = if audit.has_attention() {
                health::Status::NeedsAttention
            } else {
                health::Status::Verified
            };
            ledger.mark("host-git-trust", status, audit.health_detail())
        }
        Err(error) => {
            println!("WARN host Git trust inspection failed: {error}");
            ledger.mark(
                "host-git-trust",
                health::Status::NeedsAttention,
                "repository metadata could not be safely inspected",
            )
        }
    }
}

fn checked(
    ledger: &mut health::Ledger,
    name: &str,
    detail: &str,
    result: Result<(), String>,
) -> Result<(), String> {
    match result {
        Ok(()) => ledger.mark(name, health::Status::Verified, detail),
        Err(error) => {
            ledger.mark(name, health::Status::NeedsAttention, "verification failed")?;
            Err(error)
        }
    }
}

fn verify_runtime(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
    profile: &Profile,
) -> Result<(), String> {
    let name = sandbox::sandbox_name(project, profile);
    let output = sbx::inspect_json(context.preserve_xdg_state, name)?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not parse Docker sandbox inspection: {error}"))?;
    for (field, expected) in [
        ("name", name),
        ("agent", envfile::agent_name(preset)),
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
    let kits = value
        .get("kits")
        .and_then(Value::as_array)
        .ok_or("runtime verification failed: kits is missing or invalid")?;
    let actual = kits.iter().filter_map(Value::as_str).collect::<Vec<_>>();
    if actual.len() != kits.len() {
        return Err("runtime verification failed: kits contains a non-string value".to_owned());
    }
    let expected = envfile::selected_kit_references(
        context,
        match profile.workspace {
            crate::profile::WorkspaceMode::Shared => envfile::Kind::Development(preset),
            crate::profile::WorkspaceMode::Isolated => envfile::Kind::Isolated(preset),
        },
        rocksdb.is_some(),
        profile,
    )?;
    if !kit_inventory_matches(&actual, &expected) {
        return Err(
            "runtime verification failed: selected kits do not exactly match the environment contract"
                .to_owned(),
        );
    }
    if !sandbox::network_profile_applied(context, name, profile.network)? {
        return Err(format!(
            "runtime verification failed: the {} network profile is not reconciled; run `sbxr repair {}`",
            profile.network.name(),
            shell_quote(&project.path)
        ));
    }
    println!("ok   runtime identity and shared workspace match the sbxr contract");
    Ok(())
}

fn kit_reference_matches(actual: &str, expected: &str) -> bool {
    actual == expected
        || actual.strip_prefix("docker.io/") == Some(expected)
        || expected.strip_prefix("docker.io/") == Some(actual)
}

fn kit_inventory_matches(actual: &[&str], expected: &[String]) -> bool {
    actual.len() == expected.len()
        && expected
            .iter()
            .all(|kit| actual.iter().any(|value| kit_reference_matches(value, kit)))
}

pub(crate) fn verify_credentials(context: &Context, sandbox_name: &str) -> Result<(), String> {
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

fn verify_openai(context: &Context, sandbox_name: &str, live: bool) -> Result<(), String> {
    let output = sbx::command(context.preserve_xdg_state)
        .args(["secret", "ls", "--service", "openai", "--json"])
        .output()
        .map_err(|error| format!("could not inspect the host OpenAI binding: {error}"))?;
    let configured = output.status.success() && openai_oauth_configured(&output.stdout)?;
    if !configured {
        return Err("OpenAI host OAuth is not configured; run `sbxr auth renew openai`".to_owned());
    }
    if live {
        live_provider(context, sandbox_name, "codex")?;
    }
    Ok(())
}

fn openai_oauth_configured(bytes: &[u8]) -> Result<bool, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("could not parse `sbx secret ls --json`: {error}"))?;
    let secrets = value
        .get("secrets")
        .and_then(Value::as_array)
        .ok_or("`sbx secret ls --json` has no secrets array")?;
    Ok(secrets.iter().any(|secret| {
        secret.get("name").and_then(Value::as_str) == Some("openai")
            && secret.get("type").and_then(Value::as_str) == Some("service")
            && secret.get("secret").and_then(Value::as_str) == Some("(oauth configured)")
    }))
}

fn verify_anthropic(context: &Context, sandbox_name: &str, live: bool) -> Result<(), String> {
    auth::verify_anthropic_proxy(context, sandbox_name)?;
    println!("ok   Anthropic OAuth proxy accepted the guest sentinel");
    if live {
        live_provider(context, sandbox_name, "claude")?;
    }
    Ok(())
}

fn verify_github(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let status = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args(["--", "gh", "auth", "status", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not verify proxied GitHub authentication: {error}"))?;
    if status.success() {
        println!("ok   GitHub HTTPS/API proxy accepted the guest sentinel");
        Ok(())
    } else {
        Err(
            "GitHub proxy authentication failed; run `sbxr auth renew github` on the host"
                .to_owned(),
        )
    }
}

fn verify_pi(context: &Context, sandbox_name: &str, live: bool) -> Result<(), String> {
    for provider in ["openai-codex", "anthropic"] {
        let status = sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args([
                "--",
                "pi",
                "auth",
                "check",
                "--provider",
                provider,
                "--no-refresh",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("could not verify Pi {provider} configuration: {error}"))?;
        if !status.success() {
            return Err(format!(
                "Pi {provider} is not using a valid managed sentinel; run `sbxr repair`"
            ));
        }
    }
    if live {
        live_provider(context, sandbox_name, "pi")?;
    }
    Ok(())
}

fn live_provider(context: &Context, sandbox_name: &str, provider: &str) -> Result<(), String> {
    let script = match provider {
        "codex" => r#"set -eu
directory=$(mktemp -d)
trap 'rm -rf "$directory"' EXIT HUP INT TERM
cd "$directory"
exec codex exec --ephemeral --skip-git-repo-check --json --sandbox read-only \
  'Reply with exactly READY. Do not call any tools.'
"#
        .to_owned(),
        "claude" => {
            format!(
                r#"set -eu
directory=$(mktemp -d)
trap 'rm -rf "$directory"' EXIT HUP INT TERM
cd "$directory"
exec claude -p 'Reply with exactly READY.' --model {LIVE_CLAUDE_MODEL} --tools '' \
  --strict-mcp-config --mcp-config '{{"mcpServers":{{}}}}' --setting-sources user \
  --no-session-persistence --output-format json
"#
            )
        }
        "pi" => {
            format!(
                r#"set -eu
directory=$(mktemp -d)
trap 'rm -rf "$directory"' EXIT HUP INT TERM
cd "$directory"
exec pi --offline --no-extensions --model {LIVE_PI_MODEL} --thinking low \
  --no-session --no-tools --no-mcp --no-skills --no-prompt-templates \
  --no-context-files --mode json -p 'Reply with exactly READY.'
"#
            )
        }
        _ => return Err("internal error: unknown live provider".to_owned()),
    };
    println!("==> making an opt-in minimal live {provider} verification request");
    let output = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args(["--", "bash", "-lc", &script])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not execute the live {provider} check: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "live {provider} verification failed ({}) without exposing provider output",
            provider_failure(&output)
        ));
    }
    let verified = match provider {
        "codex" => output.stdout.split(|byte| *byte == b'\n').any(|line| {
            serde_json::from_slice::<Value>(line).is_ok_and(|event| {
                event.get("type").and_then(Value::as_str) == Some("item.completed")
                    && event
                        .get("item")
                        .and_then(|item| item.get("type"))
                        .and_then(Value::as_str)
                        == Some("agent_message")
                    && event
                        .get("item")
                        .and_then(|item| item.get("text"))
                        .and_then(Value::as_str)
                        .is_some_and(|text| text.trim() == "READY")
            })
        }),
        "claude" => claude_live_response_succeeded(&output.stdout),
        "pi" => output.stdout.split(|byte| *byte == b'\n').any(|line| {
            serde_json::from_slice::<Value>(line).is_ok_and(|event| {
                event.get("type").and_then(Value::as_str) == Some("message_end")
                    && event
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_array)
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                part.get("type").and_then(Value::as_str) == Some("text")
                                    && part
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .is_some_and(|text| text.trim() == "READY")
                            })
                        })
            })
        }),
        _ => false,
    };
    if !verified {
        return Err(format!(
            "live {provider} verification returned an unexpected sanitized response"
        ));
    }
    println!("ok   live {provider} request completed");
    Ok(())
}

fn claude_live_response_succeeded(bytes: &[u8]) -> bool {
    bytes.split(|byte| *byte == b'\n').any(|line| {
        serde_json::from_slice::<Value>(line).is_ok_and(|value| {
            value.get("type").and_then(Value::as_str) == Some("result")
                && value.get("is_error").and_then(Value::as_bool) == Some(false)
                && value
                    .get("result")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
        })
    })
}

fn provider_failure(output: &std::process::Output) -> &'static str {
    let mut text = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    text.push_str(&String::from_utf8_lossy(&output.stdout).to_ascii_lowercase());
    if text.contains("429") || text.contains("rate limit") || text.contains("quota") {
        "rate-limit"
    } else if text.contains("401") || text.contains("unauthorized") || text.contains("login") {
        "authentication"
    } else if text.contains("403") || text.contains("forbidden") {
        "access-denied"
    } else if text.contains("timeout") || text.contains("econn") || text.contains("fetch failed") {
        "network"
    } else if text.contains("model") && (text.contains("unavailable") || text.contains("not found"))
    {
        "model-unavailable"
    } else {
        "provider-error"
    }
}

fn report_tool_versions(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    let mut tools = vec!["rustc", "cargo", "rust-analyzer"];
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
    for tool in tools {
        let arguments = sandbox_version_arguments(tool);
        let output = process::output_checked(
            sbx::exec_command(context.preserve_xdg_state)
                .arg(sandbox_name)
                .arg("--")
                .args(arguments),
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
            "rustc" | "cargo" => Some(PINNED_RUST_VERSION),
            "codex" => Some(envfile::CODEX_VERSION),
            "claude" if preset == Preset::MultiAgent => Some(envfile::CLAUDE_MIXIN_VERSION),
            "claude" => Some(envfile::CLAUDE_WORKLOAD_VERSION),
            "pi" => Some(PINNED_PI_VERSION),
            "gh" => Some(PINNED_GH_VERSION),
            _ => None,
        };
        if let Some(expected) = expected {
            if !version.contains(expected) {
                return Err(format!(
                    "sandbox {tool} version drift: expected {expected}, got {version}"
                ));
            }
        }
    }
    Ok(())
}

fn verify_command_version(
    context: &Context,
    sandbox_name: &str,
    command: &str,
    expected: &str,
) -> Result<(), String> {
    let output = process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args(["--", command, "--version"]),
        &format!("checking sandbox {command} version"),
    )?;
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_owned();
    if !version.contains(expected) {
        return Err(format!(
            "sandbox {command} version drift: expected {expected}, got {version}"
        ));
    }
    println!("ok   {command}: {version}");
    Ok(())
}

fn verify_paseo_relay(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let status = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args([
            "--",
            "node",
            "-e",
            "const fs=require('node:fs');const path=require('node:path');const home=process.env.PASEO_HOME||path.join(process.env.HOME,'.paseo');const c=JSON.parse(fs.readFileSync(path.join(home,'config.json'),'utf8'));process.exit(c?.daemon?.relay?.enabled===true?0:1)",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not inspect the sandbox Paseo relay config: {error}"))?;
    if !status.success() {
        return Err("Paseo relay is not enabled in the sandbox configuration".to_owned());
    }
    if !sandbox::paseo_relay_policy_enabled(context, sandbox_name)? {
        return Err("Paseo relay has no active sandbox-scoped network rule".to_owned());
    }
    println!("ok   Paseo relay configuration and scoped policy are enabled");
    Ok(())
}

fn verify_browser(context: &Context, sandbox_name: &str) -> Result<(), String> {
    verify_command_version(context, sandbox_name, "node", PINNED_NODE_VERSION)?;
    verify_package_version(
        context,
        sandbox_name,
        "/opt/sbxr-tools/browser/node_modules/@playwright/mcp/package.json",
        PINNED_BROWSER_VERSION,
        "Playwright MCP",
    )?;
    let status = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args(["--", "test", "-x", "/usr/local/bin/sbxr-browser-mcp"])
        .status()
        .map_err(|error| format!("could not verify the browser MCP launcher: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "sandbox browser MCP launcher is missing".to_owned())
}

fn verify_orca(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let app_run = format!("/opt/sbxr-orca/{PINNED_ORCA_VERSION}/squashfs-root/AppRun");
    let status = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args([
            "--",
            "test",
            "-x",
            &app_run,
            "-a",
            "-x",
            "/usr/local/bin/sbxr-orca-server",
        ])
        .status()
        .map_err(|error| format!("could not verify Orca: {error}"))?;
    if !status.success() {
        return Err(format!(
            "sandbox Orca {PINNED_ORCA_VERSION} or its launcher is missing"
        ));
    }
    println!("ok   Orca: {PINNED_ORCA_VERSION}");
    Ok(())
}

fn verify_rust_toolchains(
    context: &Context,
    sandbox_name: &str,
    project: &Project,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    verify_login_shell_tools(context, sandbox_name, preset, profile)?;
    report_tool_versions(context, sandbox_name, preset, profile)?;
    sandbox::verify_project_toolchains(context, sandbox_name, project)
}

fn verify_login_shell_tools(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    let passwd = process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args(["--", "getent", "passwd", "agent"]),
        "checking the sandbox login shell",
    )?;
    let actual = String::from_utf8_lossy(&passwd.stdout)
        .trim()
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_owned();
    let expected = profile.shell.program.guest_path();
    if actual != expected {
        return Err(format!(
            "sandbox login shell drift: expected {expected}, got {actual}"
        ));
    }

    let mut tools = vec!["rustc", "cargo", "rustup", "rust-analyzer", "node", "paseo"];
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
    let list = tools.join(" ");
    let script = format!(
        "exec /bin/sh -c 'for tool in {list}; do command -v \"$tool\" >/dev/null || {{ echo missing-login-shell-tool:$tool >&2; exit 1; }}; done'"
    );
    process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .arg("--")
            .arg(profile.shell.program.name())
            .args(["-lic", &script]),
        &format!(
            "checking tools through the {} login shell",
            profile.shell.program.name()
        ),
    )?;
    println!(
        "ok   {} login shell resolves {}",
        profile.shell.program.name(),
        list
    );
    Ok(())
}

fn sandbox_version_arguments(tool: &str) -> Vec<&str> {
    match tool {
        "rustc" | "cargo" | "rust-analyzer" => {
            vec!["rustup", "run", PINNED_RUST_VERSION, tool, "--version"]
        }
        _ => vec![tool, "--version"],
    }
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

fn verify_forwarded_agent(context: &Context, sandbox_name: &str) -> Result<bool, String> {
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
        Ok(true)
    } else {
        println!(
            "WARN forwarded SSH agent is unavailable or has no keys; GitHub SSH and sandbox SSH signing will not work"
        );
        Ok(false)
    }
}

fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(unix)]
    fn audit_fixture(home: &std::path::Path) -> (bool, Vec<String>) {
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", CREDENTIAL_AUDIT])
            .env("HOME", home)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let findings = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        (output.status.success(), findings)
    }

    #[cfg(unix)]
    fn audit_home(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!(
            "sbxr-credential-audit-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[cfg(unix)]
    fn write_private_json(path: &std::path::Path, value: &Value) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn audit_never_embeds_real_credential_prefixes() {
        assert!(!CREDENTIAL_AUDIT.contains("sk-ant-api"));
        assert!(!CREDENTIAL_AUDIT.contains("ghp_"));
        assert!(CREDENTIAL_AUDIT.contains("proxy-managed"));
        assert!(CREDENTIAL_AUDIT.contains(r#"{"access", "refresh"}"#));
        assert!(CREDENTIAL_AUDIT.contains("native_codex_auth"));
        assert!(CREDENTIAL_AUDIT.contains("native_claude_auth"));
        assert!(CREDENTIAL_AUDIT.contains("managed_pi_auth"));
        assert!(CREDENTIAL_AUDIT.contains("config_reference.fullmatch"));
        assert!(CREDENTIAL_AUDIT.contains("CODEX_HOME/auth.json"));
        assert!(CREDENTIAL_AUDIT.contains(":symlink"));
        assert!(CREDENTIAL_AUDIT.contains("has_symlink_component"));
    }

    #[cfg(unix)]
    #[test]
    fn credential_audit_enforces_schema_permissions_and_safe_references() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let codex = json!({
            "auth_mode": "chatgptAuthTokens",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": "proxy-managed.e30.proxy-managed",
                "access_token": "proxy-managed",
                "refresh_token": "",
                "account_id": null
            },
            "last_refresh": "1970-01-01T00:00:00Z"
        });
        let claude = json!({
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-proxy-managed",
                "expiresAt": 4102444800000_u64,
                "refreshToken": "sk-ant-ort01-proxy-managed",
                "scopes": []
            }
        });

        let clean = audit_home("clean");
        write_private_json(&clean.join(".codex/auth.json"), &codex);
        write_private_json(&clean.join(".claude/.credentials.json"), &claude);
        write_private_json(
            &clean.join(".pi/agent/models.json"),
            &json!({
                "providers":{"custom":{"apiKey":"$SAFE_API_KEY"}},
                "maxTokens": 4096,
                "outputTokens": 2048
            }),
        );
        write_private_json(
            &clean.join(".docker/config.json"),
            &json!({"auths":{"ghcr.io":{"auth":"dXNlcjpwcm94eS1tYW5hZ2Vk"}}}),
        );
        assert_eq!(audit_fixture(&clean), (true, Vec::new()));

        let world_readable = audit_home("world-readable");
        let auth = world_readable.join(".codex/auth.json");
        write_private_json(&auth, &codex);
        std::fs::set_permissions(&auth, std::fs::Permissions::from_mode(0o644)).unwrap();
        let claude_auth = world_readable.join(".claude/.credentials.json");
        write_private_json(&claude_auth, &claude);
        std::fs::set_permissions(&claude_auth, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (ok, findings) = audit_fixture(&world_readable);
        assert!(!ok);
        assert_eq!(
            findings,
            [
                ".claude/.credentials.json:unsafe-permissions",
                ".codex/auth.json:unsafe-permissions"
            ]
        );

        let custom_claude = audit_home("custom-claude");
        let custom = custom_claude.join("custom-claude");
        write_private_json(
            &custom.join(".credentials.json"),
            &json!({"token":"proxy-managed"}),
        );
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", CREDENTIAL_AUDIT])
            .env("HOME", &custom_claude)
            .env("CLAUDE_CONFIG_DIR", &custom)
            .env_remove("CODEX_HOME")
            .env_remove("PI_CODING_AGENT_DIR")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            report["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .any(|finding| finding.ends_with(".credentials.json:credential"))
        );

        let linked = audit_home("symlink");
        let target = linked.join("managed.json");
        write_private_json(&target, &codex);
        std::fs::create_dir_all(linked.join(".codex")).unwrap();
        symlink(&target, linked.join(".codex/auth.json")).unwrap();
        assert!(!audit_fixture(&linked).0);

        let literal = audit_home("literal");
        write_private_json(
            &literal.join(".pi/agent/models.json"),
            &json!({
                "providers":{"custom":{"apiKey":"literal-not-managed"}},
                "authTokens":["literal-not-managed"]
            }),
        );
        assert!(!audit_fixture(&literal).0);

        let docker_token = audit_home("docker-token-without-user");
        write_private_json(
            &docker_token.join(".docker/config.json"),
            &json!({"auths":{"ghcr.io":{"auth":"cHJveHktbWFuYWdlZA=="}}}),
        );
        assert_eq!(
            audit_fixture(&docker_token),
            (false, vec![".docker/config.json:auths".to_owned()])
        );

        let docker_helper = audit_home("docker-helper");
        write_private_json(
            &docker_helper.join(".docker/config.json"),
            &json!({"auths":{},"credsStore":"secretservice"}),
        );
        assert_eq!(
            audit_fixture(&docker_helper),
            (
                false,
                vec![".docker/config.json:credential-helper".to_owned()]
            )
        );

        let malformed = audit_home("malformed");
        let malformed_auth = malformed.join(".codex/auth.json");
        std::fs::create_dir_all(malformed_auth.parent().unwrap()).unwrap();
        std::fs::write(&malformed_auth, b"{").unwrap();
        std::fs::set_permissions(&malformed_auth, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!audit_fixture(&malformed).0);

        for home in [
            clean,
            world_readable,
            custom_claude,
            linked,
            literal,
            docker_token,
            docker_helper,
            malformed,
        ] {
            std::fs::remove_dir_all(home).unwrap();
        }
    }

    #[test]
    fn published_kit_verification_accepts_docker_hub_normalization() {
        let digest = "sbx/codex@sha256:abc";
        assert!(kit_reference_matches(
            "docker.io/sbx/codex@sha256:abc",
            digest
        ));
        assert!(kit_reference_matches(
            digest,
            "docker.io/sbx/codex@sha256:abc"
        ));
        assert!(!kit_reference_matches(
            "docker.io/sbx/codex@sha256:def",
            digest
        ));

        let expected = vec![digest.to_owned()];
        assert!(kit_inventory_matches(
            &["docker.io/sbx/codex@sha256:abc"],
            &expected
        ));
        assert!(!kit_inventory_matches(
            &["docker.io/sbx/codex@sha256:abc", "/unexpected/extra-kit"],
            &expected
        ));
    }

    #[test]
    fn pinned_rust_version_checks_ignore_project_overrides() {
        assert_eq!(
            sandbox_version_arguments("rustc"),
            ["rustup", "run", PINNED_RUST_VERSION, "rustc", "--version"]
        );
        assert_eq!(
            sandbox_version_arguments("cargo"),
            ["rustup", "run", PINNED_RUST_VERSION, "cargo", "--version"]
        );
        assert_eq!(sandbox_version_arguments("pi"), ["pi", "--version"]);
    }

    #[test]
    fn claude_live_response_accepts_nonempty_success_amid_notices() {
        let response = br#"notice emitted by a launcher
{"type":"result","is_error":false,"result":"READY"}
"#;
        assert!(claude_live_response_succeeded(response));
        assert!(claude_live_response_succeeded(
            br#"{"type":"result","is_error":false,"result":"A valid assistant response."}"#
        ));
        assert!(!claude_live_response_succeeded(
            br#"{"type":"result","is_error":false,"result":"  "}"#
        ));
        assert!(!claude_live_response_succeeded(
            br#"{"type":"result","is_error":true,"result":"READY"}"#
        ));
    }

    #[test]
    fn parses_openai_oauth_binding_fixture() {
        let fixture = include_bytes!("../tests/fixtures/sbx-secret-ls-0.51.0.json");
        assert_eq!(openai_oauth_configured(fixture), Ok(true));
        assert_eq!(openai_oauth_configured(br#"{"secrets":[]}"#), Ok(false));
        assert!(openai_oauth_configured(br#"{"secrets":{}}"#).is_err());
        assert!(openai_oauth_configured(b"{").is_err());
    }
}
