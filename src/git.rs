use crate::environment::Context;
use crate::{process, sbx, sync};
use std::path::Path;
use std::process::{Command, Stdio};

const GITHUB_KNOWN_HOSTS: &str = r#"github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl
github.com ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=
github.com ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQCj7ndNxQowgcQnjshcLrqPEiiphnt+VTTvDP6mHBL9j1aNUkY4Ue1gvwnGLVlOhGeYrnZaMgRK6+PKCUXaDbC7qtbW8gIkhL7aGCsOr/C56SJMy/BCZfxd1nWzAOxSDPgVsmerOBYfNqltV9/hWCqBywINIR+5dIg6JTJ72pcEpEjcYgXkE2YEFXV1JHnsKgbLWNlhScqb2UmyRkQyytRLtL+38TGxkxCflmO+5Z8CSSNY7GidjMIZ7Q4zMjA2n1nGrlTDkzwDCsw+wqFPGQA179cnfGWOWRVruj16z6XyvxvjJwbz0wQZ75XK5tKSb7FNyeIEs4TT4jk+S4dhPeAUC5y+bDYirYgM4GC7uEnztnZyaVWQ7B381AK4Qdrwt51ZqExKbQpTUNn+EjqoTwvqNj4kqx5QUCI0ThS/YkOxJCXmPUWZbhjpCg56i+2aB6CmK2JGhn57K5mj0MNdBXA4/WnwH6XoPWJzK5Nyu2zB3nAZp+S5hpQs+p1vN1/wsjk=
"#;
const SSH_SIGNING_COMMAND: &str = r#"#!/bin/sh
set -e

if [ -z "$SSH_AUTH_SOCK" ]; then
  echo "[sbxr] no forwarded SSH agent; reconnect with agent forwarding enabled" >&2
  exit 1
fi

if ! keys=$(ssh-add -L 2>/dev/null); then
  echo "[sbxr] the forwarded SSH agent is unavailable; on the host, verify it with: ssh-add -L" >&2
  echo "[sbxr] if the host agent socket changed, reopen the sandbox through sbxr" >&2
  exit 1
fi

key=$(printf '%s\n' "$keys" | head -n 1)
if [ -z "$key" ]; then
  echo "[sbxr] the forwarded SSH agent has no keys; load one on the host with ssh-add" >&2
  exit 1
fi

config_dir="${GIT_SSH_SIGN_CONFIG_DIR:-/home/agent/.config/git}"
mkdir -p "$config_dir"
email=$(git config user.email 2>/dev/null || printf '%s' "agent@sandbox.local")
printf '%s %s\n' "$email" "$key" > "$config_dir/allowed_signers"
printf 'key::%s\n' "$key"
"#;

pub(crate) fn configure(
    context: &Context,
    sandbox_name: &str,
    project: &Path,
) -> Result<(), String> {
    ensure_github_policy(context, sandbox_name)?;
    install_sandbox_git_integration(context, sandbox_name)?;
    mirror_host_git_settings(context, sandbox_name, project)?;
    Ok(())
}

fn ensure_github_policy(context: &Context, sandbox_name: &str) -> Result<(), String> {
    process::run_checked(
        sbx::command(context.preserve_xdg_state).args([
            "policy",
            "allow",
            "network",
            "--sandbox",
            sandbox_name,
            "github.com:22",
        ]),
        "allowing GitHub SSH for the sandbox",
    )
}

fn install_sandbox_git_integration(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let signing_command = "/home/agent/.config/git/ssh-signing-key-command";
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state).args([
            sandbox_name,
            "mkdir",
            "-p",
            "/home/agent/.config/git",
        ]),
        "creating sandbox Git configuration directory",
    )?;
    sync::upload_file(
        sandbox_name,
        context.preserve_xdg_state,
        signing_command,
        SSH_SIGNING_COMMAND.as_bytes(),
    )?;
    let github_keys = "/home/agent/.config/git/github-known-hosts";
    sync::upload_file(
        sandbox_name,
        context.preserve_xdg_state,
        github_keys,
        GITHUB_KNOWN_HOSTS.as_bytes(),
    )?;
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state).args([
            sandbox_name,
            "chmod",
            "755",
            signing_command,
        ]),
        "making the SSH signing-key resolver executable",
    )?;

    let script = r#"
set -eu
if ! command -v nano >/dev/null 2>&1 || [ ! -r /usr/share/bash-completion/bash_completion ]; then
  attempt=0
  until sudo env DEBIAN_FRONTEND=noninteractive apt-get update; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
      echo "apt-get update remained unavailable after 60 seconds" >&2
      exit 1
    fi
    sleep 2
  done
  attempt=0
  until sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends bash-completion nano openssh-client; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
      echo "apt-get install remained unavailable after 60 seconds" >&2
      exit 1
    fi
    sleep 2
  done
  sudo rm -rf /var/lib/apt/lists/*
fi

git config --global core.editor nano
git config --global --unset-all user.signingKey 2>/dev/null || true
sudo git config --system gpg.format ssh
sudo git config --system --unset-all user.signingKey 2>/dev/null || true
sudo git config --system --unset-all commit.gpgSign 2>/dev/null || true
sudo git config --system --unset-all tag.gpgSign 2>/dev/null || true
sudo git config --system gpg.ssh.defaultKeyCommand /home/agent/.config/git/ssh-signing-key-command
sudo git config --system gpg.ssh.allowedSignersFile /home/agent/.config/git/allowed_signers

mkdir -p "$HOME/.ssh"
chmod 700 "$HOME/.ssh"
if ! grep -q '^github\.com ' "$HOME/.ssh/known_hosts" 2>/dev/null; then
  cat "$HOME/.config/git/github-known-hosts" >> "$HOME/.ssh/known_hosts"
fi
sort -u -o "$HOME/.ssh/known_hosts" "$HOME/.ssh/known_hosts"
chmod 600 "$HOME/.ssh/known_hosts"

touch "$HOME/.bashrc"
if ! grep -Fq '/usr/share/bash-completion/bash_completion' "$HOME/.bashrc"; then
  printf '%s\n' '' '# sbxr Bash and Git completion' \
    '[ -r /usr/share/bash-completion/bash_completion ] && . /usr/share/bash-completion/bash_completion' \
    >> "$HOME/.bashrc"
fi
"#;
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .arg("--")
            .args(["bash", "-lc", script]),
        "configuring sandbox Git tooling",
    )
}

fn mirror_host_git_settings(
    context: &Context,
    sandbox_name: &str,
    project: &Path,
) -> Result<(), String> {
    for key in ["user.name", "user.email", "init.defaultBranch"] {
        if let Some(value) = host_global_git_value(key)? {
            process::run_checked(
                sbx::exec_command(context.preserve_xdg_state).args([
                    sandbox_name,
                    "git",
                    "config",
                    "--global",
                    key,
                    &value,
                ]),
                &format!("mirroring host Git setting {key}"),
            )?;
        }
    }
    let commit_signing = host_git_bool(project, "commit.gpgSign")?;
    let tag_signing = host_git_bool(project, "tag.gpgSign")?;
    let signing_format = host_git_value(project, "gpg.format")?
        .unwrap_or_else(|| "openpgp".to_owned())
        .to_ascii_lowercase();
    let ssh_signing = signing_format_is_forwardable(&signing_format);

    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state).args([
            sandbox_name,
            "git",
            "config",
            "--global",
            "gpg.format",
            &signing_format,
        ]),
        "mirroring host Git signing format",
    )?;

    if ssh_signing {
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state).args([
                sandbox_name,
                "--",
                "sh",
                "-c",
                "git config --global --unset-all user.signingKey 2>/dev/null || true",
            ]),
            "selecting the forwarded SSH agent signing key",
        )?;
    } else if let Some(signing_key) = host_git_value(project, "user.signingKey")? {
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state).args([
                sandbox_name,
                "git",
                "config",
                "--global",
                "user.signingKey",
                &signing_key,
            ]),
            "mirroring host Git signing-key identity",
        )?;
    }

    for (key, enabled) in [
        ("commit.gpgSign", commit_signing && ssh_signing),
        ("tag.gpgSign", tag_signing && ssh_signing),
    ] {
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state).args([
                sandbox_name,
                "git",
                "config",
                "--global",
                key,
                if enabled { "true" } else { "false" },
            ]),
            &format!("mirroring host Git setting {key}"),
        )?;
    }
    println!(
        "==> mirrored host Git signing policy (format: {}, sandbox commits: {}, sandbox tags: {})",
        signing_format,
        enabled_label(commit_signing && ssh_signing),
        enabled_label(tag_signing && ssh_signing)
    );
    if (commit_signing || tag_signing) && !ssh_signing {
        warn_unavailable_host_signing(&signing_format);
    }
    Ok(())
}

fn enabled_label(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

fn signing_format_is_forwardable(format: &str) -> bool {
    format.eq_ignore_ascii_case("ssh")
}

fn host_global_git_value(key: &str) -> Result<Option<String>, String> {
    if !process::command_exists("git") {
        return Ok(None);
    }
    let output = Command::new("git")
        .args(["config", "--global", "--get", key])
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not inspect host Git setting {key}: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("host Git setting {key} is not UTF-8: {error}"))?;
    Ok(Some(value.trim_end_matches(['\r', '\n']).to_owned()))
}

fn host_git_value(project: &Path, key: &str) -> Result<Option<String>, String> {
    if !process::command_exists("git") {
        return Ok(None);
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["config", "--get", key])
        .output()
        .map_err(|error| format!("could not inspect host Git setting {key}: {error}"))?;
    if output.status.success() {
        let value = String::from_utf8(output.stdout)
            .map_err(|error| format!("host Git setting {key} is not UTF-8: {error}"))?;
        return Ok(Some(value.trim_end_matches(['\r', '\n']).to_owned()));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "could not read host Git setting {key}: {}",
        stderr.trim()
    ))
}

fn host_git_bool(project: &Path, key: &str) -> Result<bool, String> {
    if !process::command_exists("git") {
        return Ok(false);
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["config", "--bool", "--get", key])
        .output()
        .map_err(|error| format!("could not inspect host Git setting {key}: {error}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim() == "true");
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "could not parse host Git setting {key}: {}",
        stderr.trim()
    ))
}

fn warn_unavailable_host_signing(format: &str) {
    eprintln!(
        "warning: host Git requires {format} signing, but Docker Sandboxes cannot forward that signer"
    );
    eprintln!(
        "warning: automatic commit and tag signing is disabled in this sandbox; commits made here will be unsigned"
    );
    eprintln!("warning: sign or re-sign commits on the host before pushing them");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_signing_policy() {
        assert_eq!(enabled_label(true), "enabled");
        assert_eq!(enabled_label(false), "disabled");
    }

    #[test]
    fn only_ssh_signing_is_forwardable() {
        assert!(signing_format_is_forwardable("ssh"));
        assert!(signing_format_is_forwardable("SSH"));
        assert!(!signing_format_is_forwardable("openpgp"));
        assert!(!signing_format_is_forwardable("x509"));
    }
}
