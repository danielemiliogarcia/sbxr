use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;

static SSH_AGENT_SOCKET: OnceLock<Option<OsString>> = OnceLock::new();

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn command_exists(name: &str) -> bool {
    command_path(name).is_some()
}

pub fn command_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|directory| {
        let direct = directory.join(name);
        if executable(direct.clone()) {
            return Some(direct);
        }
        #[cfg(windows)]
        {
            let extensions = env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
            for extension in extensions.to_string_lossy().split(';') {
                for extension in [
                    extension.to_ascii_lowercase(),
                    extension.to_ascii_uppercase(),
                ] {
                    let candidate = directory.join(format!("{name}{extension}"));
                    if executable(candidate.clone()) {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    })
}

pub fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "neither HOME nor USERPROFILE is set".to_owned())
}

fn executable(path: PathBuf) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn require(name: &str) -> Result<(), String> {
    if command_exists(name) {
        return Ok(());
    }
    Err(format!(
        "required command not found: {name}\n{}",
        install_hint(name)
    ))
}

pub fn install_hint(name: &str) -> &'static str {
    match name {
        "sbx" => {
            "Install Docker Sandboxes: https://docs.docker.com/ai/sandboxes/install/\n\
             Ubuntu/Linux, sbx only (Docker Engine is not required):\n  \
             curl -fsSL https://get.docker.com | sudo REPO_ONLY=1 sh\n  \
             sudo apt-get install docker-sbx\nThen run:\n  sbx login\n  sbx diagnose"
        }
        "code" => {
            "Install VS Code: https://code.visualstudio.com/download\n\
             Then run: code --install-extension ms-vscode-remote.remote-ssh"
        }
        "ssh" => "Install OpenSSH client (Ubuntu: sudo apt-get install openssh-client).",
        "git" => "Install Git (Ubuntu: sudo apt-get install git).",
        _ => "Install the missing host command and retry `sbxr doctor`.",
    }
}

pub fn code_command(preserve_xdg_state: bool) -> Command {
    let mut command = Command::new("code");
    apply_ssh_agent(&mut command);
    if !preserve_xdg_state {
        command.env_remove("XDG_STATE_HOME");
    }
    command
}

pub fn selected_ssh_agent_socket() -> Option<&'static OsStr> {
    SSH_AGENT_SOCKET
        .get_or_init(detect_ssh_agent_socket)
        .as_deref()
}

pub fn configured_ssh_agent_socket() -> Option<OsString> {
    env::var_os("SSH_AUTH_SOCK")
}

pub fn ssh_agent_has_keys(socket: &OsStr) -> bool {
    Command::new("ssh-add")
        .arg("-L")
        .env("SSH_AUTH_SOCK", socket)
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

fn apply_ssh_agent(command: &mut Command) {
    if let Some(socket) = selected_ssh_agent_socket() {
        command.env("SSH_AUTH_SOCK", socket);
    }
}

fn detect_ssh_agent_socket() -> Option<OsString> {
    let mut candidates = Vec::new();
    if let Some(socket) = configured_ssh_agent_socket() {
        candidates.push(socket);
    }
    if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(runtime);
        candidates.push(runtime.join("gcr/ssh").into_os_string());
        candidates.push(runtime.join("gnupg/S.gpg-agent.ssh").into_os_string());
        candidates.push(runtime.join("keyring/ssh").into_os_string());
    }
    candidates.dedup();

    candidates
        .into_iter()
        .find(|socket| ssh_agent_has_keys(socket))
}

pub fn run_checked(command: &mut Command, description: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("could not run {description}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{description} failed with {status}"))
    }
}

pub fn output_checked(command: &mut Command, description: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("could not run {description}: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            Err(format!("{description} failed with {}", output.status))
        } else {
            Err(format!("{description} failed: {stderr}"))
        }
    }
}
