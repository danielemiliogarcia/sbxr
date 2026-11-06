use crate::process;
use crate::profile::Resources;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MINIMUM_VERSION: Version = Version {
    major: 0,
    minor: 51,
    patch: 0,
};

const CLI_REFACTOR_VERSION: Version = Version {
    major: 0,
    minor: 48,
    patch: 0,
};

static CAPABILITY_CHECK: OnceLock<Result<(), String>> = OnceLock::new();
static RUNTIME_INFO: OnceLock<Result<RuntimeInfo, String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    fn parse(output: &str) -> Result<Self, String> {
        let version = output
            .split_whitespace()
            .filter_map(|word| word.strip_prefix('v'))
            .find(|word| word.as_bytes().first().is_some_and(u8::is_ascii_digit))
            .ok_or_else(|| format!("could not parse Docker Sandboxes version from: {output}"))?;
        let core = version.split_once('-').map_or(version, |(core, _)| core);
        let mut parts = core.split('.');
        let parse = |part: Option<&str>, label: &str| {
            part.ok_or_else(|| format!("Docker Sandboxes version has no {label}: {version}"))?
                .parse::<u64>()
                .map_err(|error| format!("invalid Docker Sandboxes {label} in {version}: {error}"))
        };
        let parsed = Self {
            major: parse(parts.next(), "major component")?,
            minor: parse(parts.next(), "minor component")?,
            patch: parse(parts.next(), "patch component")?,
        };
        if parts.next().is_some() {
            return Err(format!("invalid Docker Sandboxes version: {version}"));
        }
        Ok(parsed)
    }

    fn diagnose_format_flag(self) -> &'static str {
        if self >= CLI_REFACTOR_VERSION {
            "--format"
        } else {
            "--output"
        }
    }

    fn env_detach_flag(self) -> &'static str {
        if self >= CLI_REFACTOR_VERSION {
            "--detach"
        } else {
            "--detached"
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeInfo {
    pub(crate) client: Version,
    pub(crate) server: Version,
    pub(crate) api_version: String,
    pub(crate) server_state: String,
}

impl RuntimeInfo {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let document: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("could not parse `sbx version --json`: {error}"))?;
        let client = document
            .get("client")
            .and_then(|value| value.get("version"))
            .and_then(serde_json::Value::as_str)
            .ok_or("`sbx version --json` has no client.version string")?;
        let server = document
            .get("server")
            .ok_or("`sbx version --json` has no server object")?;
        let server_version = server
            .get("version")
            .and_then(serde_json::Value::as_str)
            .ok_or("`sbx version --json` has no server.version string")?;
        let server_state = server
            .get("state")
            .and_then(serde_json::Value::as_str)
            .ok_or("`sbx version --json` has no server.state string")?;
        let api_version = server
            .get("api_version")
            .and_then(serde_json::Value::as_str)
            .ok_or("`sbx version --json` has no server.api_version string")?;
        if api_version.is_empty() {
            return Err("`sbx version --json` has an empty server.api_version".to_owned());
        }
        Ok(Self {
            client: Version::parse(client)?,
            server: Version::parse(server_version)?,
            api_version: api_version.to_owned(),
            server_state: server_state.to_owned(),
        })
    }
}

pub(crate) fn command(preserve_xdg_state: bool) -> Command {
    let mut command = Command::new("sbx");
    if let Some(socket) = process::selected_ssh_agent_socket() {
        command.env("SSH_AUTH_SOCK", socket);
    }
    if !preserve_xdg_state {
        command.env_remove("XDG_STATE_HOME");
    }
    command
}

pub(crate) fn exec_command(preserve_xdg_state: bool) -> Command {
    let mut command = command(preserve_xdg_state);
    command.arg("exec");
    command
}

pub(crate) fn interactive_exec_command(
    preserve_xdg_state: bool,
    sandbox_name: &str,
    workdir: &Path,
    program: &[&str],
) -> Command {
    let mut command = exec_command(preserve_xdg_state);
    command
        .arg("-it")
        .arg("--workdir")
        .arg(workdir)
        .arg(sandbox_name)
        .args(program);
    command
}

pub(crate) fn require_minimum_version(preserve_xdg_state: bool) -> Result<Version, String> {
    process::require("sbx")?;
    let info = runtime_info(preserve_xdg_state)?;
    validate_minimum_versions(&info)?;
    if info.server_state != "running" {
        return Err(format!(
            "Docker Sandboxes server is {}; run `sbx daemon start --detach` and retry",
            info.server_state
        ));
    }
    CAPABILITY_CHECK
        .get_or_init(|| validate_cli_capabilities(preserve_xdg_state))
        .clone()?;
    Ok(info.client)
}

pub(crate) fn runtime_info(preserve_xdg_state: bool) -> Result<RuntimeInfo, String> {
    RUNTIME_INFO
        .get_or_init(|| {
            let output = process::output_checked(
                command(preserve_xdg_state).args(["version", "--json"]),
                "reading Docker Sandboxes client/server versions",
            )?;
            RuntimeInfo::parse(&output.stdout)
        })
        .clone()
}

pub(crate) fn validate_minimum_versions(info: &RuntimeInfo) -> Result<(), String> {
    if info.client < MINIMUM_VERSION || info.server < MINIMUM_VERSION {
        return Err(format!(
            "Docker Sandboxes client {} and server {} must both be >= {MINIMUM_VERSION} for Kits v3 declarative environments\nUpdate Docker Sandboxes: https://docs.docker.com/ai/sandboxes/install/",
            info.client, info.server
        ));
    }
    Ok(())
}

fn validate_cli_capabilities(preserve_xdg_state: bool) -> Result<(), String> {
    for (arguments, required) in [
        (&["env", "run", "--help"][..], &["--force", "--detach"][..]),
        (&["ports", "--help"][..], &["--json"][..]),
    ] {
        let output = process::output_checked(
            command(preserve_xdg_state).args(arguments),
            "checking Docker Sandboxes CLI capabilities",
        )?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        for flag in required {
            if !help_has_flag(&text, flag) {
                return Err(format!(
                    "installed Docker Sandboxes CLI is missing required capability {flag}; client/server versions alone are not sufficient"
                ));
            }
        }
    }
    validate_skills_environment_schema(preserve_xdg_state)
}

fn validate_skills_environment_schema(preserve_xdg_state: bool) -> Result<(), String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sbxr-sbx-capability-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    #[cfg(unix)]
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect {}: {error}", directory.display()))?;
    let definition = directory.join("sbxenv.yaml");
    let result = (|| {
        fs::write(
            &definition,
            include_bytes!("../tests/fixtures/sbxenv-skills-off-0.51.0.yaml"),
        )
        .map_err(|error| format!("could not write {}: {error}", definition.display()))?;
        process::output_checked(
            command(preserve_xdg_state).args([
                "env",
                "plan",
                "--skip-host-commands",
                definition
                    .to_str()
                    .ok_or("temporary environment path is not UTF-8")?,
            ]),
            "checking Docker Sandboxes skills environment schema",
        )?;
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&directory)
        .map_err(|error| format!("could not remove {}: {error}", directory.display()));
    result.and(cleanup)
}

fn help_has_flag(text: &str, flag: &str) -> bool {
    text.split_whitespace().any(|token| {
        token.trim_matches(|character: char| {
            matches!(character, ',' | ';' | ':' | '[' | ']' | '(' | ')')
        }) == flag
    })
}

pub(crate) fn list_json(preserve_xdg_state: bool) -> Result<Output, String> {
    process::output_checked(
        command(preserve_xdg_state).args(["ls", "--json"]),
        "listing Docker sandboxes",
    )
    .map_err(|error| format!("{error}\nRun `sbx login`, then `sbx diagnose`."))
}

pub(crate) fn inspect_json(preserve_xdg_state: bool, sandbox_name: &str) -> Result<Output, String> {
    process::output_checked(
        command(preserve_xdg_state).args(["inspect", sandbox_name, "--json"]),
        "inspecting Docker sandbox runtime metadata",
    )
}

pub(crate) fn published_host_port(
    preserve_xdg_state: bool,
    sandbox_name: &str,
    sandbox_port: u16,
) -> Result<u16, String> {
    let output = process::output_checked(
        command(preserve_xdg_state).args(["ports", sandbox_name, "--json"]),
        "reading sandbox port mappings",
    )?;
    parse_published_host_port(&output.stdout, sandbox_port)
}

fn parse_published_host_port(bytes: &[u8], sandbox_port: u16) -> Result<u16, String> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("could not parse `sbx ports --json`: {error}"))?;
    let mappings = document
        .as_array()
        .ok_or("`sbx ports --json` did not return an array")?;
    let mapping = mappings
        .iter()
        .find(|mapping| {
            mapping
                .get("sandbox_port")
                .and_then(serde_json::Value::as_u64)
                == Some(u64::from(sandbox_port))
                && matches!(
                    mapping.get("host_ip").and_then(serde_json::Value::as_str),
                    Some("127.0.0.1" | "::1")
                )
                && mapping
                    .get("protocol")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|protocol| protocol.starts_with("tcp"))
        })
        .ok_or_else(|| {
            format!("sandbox port {sandbox_port} has no loopback-only published mapping")
        })?;
    mapping
        .get("host_port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("sandbox port {sandbox_port} has an invalid host port"))
}

pub(crate) fn sandbox_state(json: &[u8], wanted: &str) -> Result<Option<String>, String> {
    let document: serde_json::Value = serde_json::from_slice(json)
        .map_err(|error| format!("could not parse Docker sandbox list: {error}"))?;
    let sandboxes = document.as_array().or_else(|| {
        document
            .get("sandboxes")
            .and_then(serde_json::Value::as_array)
    });
    let sandboxes = sandboxes
        .ok_or("Docker sandbox list is neither an array nor an object with a sandboxes array")?;
    Ok(sandboxes.iter().find_map(|sandbox| {
        (sandbox.get("name").and_then(serde_json::Value::as_str) == Some(wanted)).then(|| {
            sandbox
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_owned()
        })
    }))
}

pub(crate) fn setup_ssh(preserve_xdg_state: bool) -> Result<(), String> {
    process::run_checked(
        command(preserve_xdg_state).args(["setup", "ssh"]),
        "configuring Docker Sandbox SSH",
    )
}

pub(crate) fn diagnose_json(preserve_xdg_state: bool, version: Version) -> Result<Output, String> {
    process::output_checked(
        command(preserve_xdg_state).args(["diagnose", version.diagnose_format_flag(), "json"]),
        "running Docker Sandboxes diagnostics",
    )
}

pub(crate) fn env_ensure_up(
    preserve_xdg_state: bool,
    definition: &Path,
    version: Version,
    force: bool,
    resources: &Resources,
) -> Result<(), String> {
    let mut command = env_run_command(preserve_xdg_state, definition, version, force, resources);
    process::run_checked(&mut command, "creating or starting declarative sandbox")
}

fn env_run_command(
    preserve_xdg_state: bool,
    definition: &Path,
    version: Version,
    force: bool,
    resources: &Resources,
) -> Command {
    let mut command = command(preserve_xdg_state);
    command.args(["env", "run", version.env_detach_flag()]);
    if force {
        command.arg("--force");
    }
    if let Some(size) = &resources.root_disk {
        command.env("DOCKER_SANDBOXES_ROOT_SIZE", size);
    }
    if let Some(size) = &resources.docker_disk {
        command.env("DOCKER_SANDBOXES_DOCKER_SIZE", size);
    }
    command.arg(definition);
    command
}

pub(crate) fn env_remove(
    preserve_xdg_state: bool,
    definition: &Path,
    force: bool,
) -> Result<(), String> {
    let mut command = command(preserve_xdg_state);
    command.args(["env", "rm"]);
    if force {
        command.arg("--force");
    }
    command.arg(definition);
    process::run_checked(&mut command, "removing declarative sandbox")
}

pub(crate) fn stop(preserve_xdg_state: bool, sandbox_name: &str) -> Result<(), String> {
    process::run_checked(
        command(preserve_xdg_state).arg("stop").arg(sandbox_name),
        "stopping sandbox",
    )
}

pub(crate) fn read_file(
    preserve_xdg_state: bool,
    sandbox_name: &str,
    path: &str,
) -> Result<Option<Vec<u8>>, String> {
    let output = exec_command(preserve_xdg_state)
        .arg(sandbox_name)
        .args(["test", "-f", path])
        .output()
        .map_err(|error| format!("could not inspect sandbox contract marker: {error}"))?;
    if !output.status.success() {
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return Ok(None);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "inspecting sandbox contract marker failed with {}{}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ));
    }
    let output = process::output_checked(
        exec_command(preserve_xdg_state)
            .arg(sandbox_name)
            .args(["cat", path]),
        "reading sandbox contract marker",
    )?;
    Ok(Some(output.stdout))
}

pub(crate) fn write_file(
    preserve_xdg_state: bool,
    sandbox_name: &str,
    path: &str,
    contents: &OsStr,
) -> Result<(), String> {
    let script = r#"set -eu
path=$1
value=$2
directory=${path%/*}
mkdir -p "$directory"
temporary="$path.tmp.$$"
trap 'rm -f "$temporary"' EXIT HUP INT TERM
umask 077
printf '%s\n' "$value" > "$temporary"
mv -f "$temporary" "$path"
trap - EXIT HUP INT TERM
"#;
    process::run_checked(
        exec_command(preserve_xdg_state)
            .arg(sandbox_name)
            .args(["sh", "-c", script, "sh", path])
            .arg(contents),
        "writing sandbox contract marker",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_versions() {
        assert_eq!(
            Version::parse("sbx version: v0.51.0 abc").unwrap(),
            MINIMUM_VERSION
        );
        assert!(Version::parse("sbx version: v0.51.1").unwrap() > MINIMUM_VERSION);
        assert!(Version::parse("unknown").is_err());
    }

    #[test]
    fn parses_client_server_compatibility_metadata() {
        let info = RuntimeInfo::parse(include_bytes!("../tests/fixtures/sbx-version-0.51.0.json"))
            .unwrap();
        assert_eq!(info.client.to_string(), "0.51.0");
        assert_eq!(info.server.to_string(), "0.51.0");
        assert_eq!(info.api_version, "0.49.0");
        assert!(RuntimeInfo::parse(br#"{"client":{"version":"v0.51.0"}}"#).is_err());
    }

    #[test]
    fn finds_sandbox_state_in_legacy_docker_json() {
        let json = br#"{
          "sandboxes": [
            {"name":"rust-multi-one", "status":"running"},
            {"name":"rust-multi-two", "status":"stopped"}
          ]
        }"#;
        assert_eq!(
            sandbox_state(json, "rust-multi-two").unwrap(),
            Some("stopped".to_owned())
        );
        assert_eq!(sandbox_state(json, "rust-multi-missing").unwrap(), None);
    }

    #[test]
    fn finds_sandbox_state_in_current_docker_json() {
        let json = include_bytes!("../tests/fixtures/sbx-list-0.51.0.json");
        assert_eq!(
            sandbox_state(json, "rust-multi-fixture").unwrap(),
            Some("running".to_owned())
        );
        assert_eq!(sandbox_state(json, "rust-multi-missing").unwrap(), None);
    }

    #[test]
    fn rejects_unknown_sandbox_list_shape() {
        let error = sandbox_state(br#"{"items": []}"#, "rust-multi-one").unwrap_err();
        assert!(error.contains("neither an array nor an object"));
    }

    #[test]
    fn selects_version_specific_cli_flags() {
        let legacy = Version::parse("sbx version v0.47.0").unwrap();
        assert_eq!(legacy.diagnose_format_flag(), "--output");
        assert_eq!(legacy.env_detach_flag(), "--detached");

        let current = Version::parse("sbx version v0.48.0").unwrap();
        assert_eq!(current.diagnose_format_flag(), "--format");
        assert_eq!(current.env_detach_flag(), "--detach");
    }

    #[test]
    fn capability_flags_are_matched_as_whole_tokens() {
        assert!(help_has_flag("-f, --force  apply", "--force"));
        assert!(help_has_flag("usage: command [--json]", "--json"));
        assert!(!help_has_flag("--force-recreate", "--force"));
        assert!(!help_has_flag("prefix--json", "--json"));
    }

    #[test]
    fn parses_loopback_ephemeral_port_mapping() {
        let json = include_bytes!("../tests/fixtures/sbx-ports-0.51.0.json");
        assert_eq!(parse_published_host_port(json, 6768).unwrap(), 45678);
        assert!(parse_published_host_port(json, 6767).is_err());
    }

    #[test]
    fn rejects_non_loopback_port_mapping() {
        let json = br#"[
          {"host_ip":"0.0.0.0","host_port":45678,"sandbox_port":6768,"protocol":"tcp4"}
        ]"#;
        assert!(parse_published_host_port(json, 6768).is_err());

        let udp = br#"[
          {"host_ip":"127.0.0.1","host_port":45678,"sandbox_port":6768,"protocol":"udp4"}
        ]"#;
        assert!(parse_published_host_port(udp, 6768).is_err());
    }

    #[test]
    fn builds_interactive_exec_with_workdir_before_the_sandbox() {
        let command = interactive_exec_command(
            true,
            "rust-project",
            Path::new("/work/project"),
            &["bash", "-l"],
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "exec",
                "-it",
                "--workdir",
                "/work/project",
                "rust-project",
                "bash",
                "-l"
            ]
        );
    }

    #[test]
    fn forwards_force_to_declarative_environment_runs() {
        let version = Version::parse("sbx version v0.50.1").unwrap();
        let command = env_run_command(
            true,
            Path::new("/tmp/project.sbxenv.yaml"),
            version,
            true,
            &Resources::default(),
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "env",
                "run",
                "--detach",
                "--force",
                "/tmp/project.sbxenv.yaml"
            ]
        );
    }
}
