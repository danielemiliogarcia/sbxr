use crate::process;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

const MINIMUM_VERSION: Version = Version {
    major: 0,
    minor: 39,
    patch: 0,
};

const CLI_REFACTOR_VERSION: Version = Version {
    major: 0,
    minor: 48,
    patch: 0,
};

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
    let output = process::output_checked(
        command(preserve_xdg_state).arg("version"),
        "checking Docker Sandboxes version",
    )?;
    let text = String::from_utf8(output.stdout)
        .map_err(|error| format!("Docker Sandboxes version output is not UTF-8: {error}"))?;
    let installed = Version::parse(text.trim())?;
    if installed < MINIMUM_VERSION {
        return Err(format!(
            "Docker Sandboxes {installed} is too old; sbxr requires >= {MINIMUM_VERSION} for declarative environments\nUpdate Docker Sandboxes: https://docs.docker.com/ai/sandboxes/install/"
        ));
    }
    Ok(installed)
}

pub(crate) fn list_json(preserve_xdg_state: bool) -> Result<Output, String> {
    process::output_checked(
        command(preserve_xdg_state).args(["ls", "--json"]),
        "listing Docker sandboxes",
    )
    .map_err(|error| format!("{error}\nRun `sbx login`, then `sbx diagnose`."))
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
) -> Result<(), String> {
    let mut command = env_run_command(preserve_xdg_state, definition, version, force);
    process::run_checked(&mut command, "creating or starting declarative sandbox")
}

fn env_run_command(
    preserve_xdg_state: bool,
    definition: &Path,
    version: Version,
    force: bool,
) -> Command {
    let mut command = command(preserve_xdg_state);
    command.args(["env", "run", version.env_detach_flag()]);
    if force {
        command.arg("--force");
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
            Version::parse("sbx version: v0.39.0 abc").unwrap(),
            MINIMUM_VERSION
        );
        assert!(Version::parse("sbx version: v0.40.1").unwrap() > MINIMUM_VERSION);
        assert!(Version::parse("unknown").is_err());
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
        let json = br#"[
          {"name":"rust-multi-one", "status":"running", "detached":true},
          {"name":"rust-multi-two", "status":"stopped", "detached":false}
        ]"#;
        assert_eq!(
            sandbox_state(json, "rust-multi-one").unwrap(),
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
        let command = env_run_command(true, Path::new("/tmp/project.sbxenv.yaml"), version, true);
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
