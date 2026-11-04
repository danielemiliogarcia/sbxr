use crate::environment::Context;
use crate::{process, sbx, sync};
use std::ffi::OsStr;
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
    configure_cargo_git_cli(context, sandbox_name)?;
    mirror_host_github_rewrites(context, sandbox_name, project)?;
    mirror_host_git_settings(context, sandbox_name, project)?;
    Ok(())
}

fn configure_cargo_git_cli(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let path = "/home/agent/.cargo/config.toml";
    let current = sbx::read_file(context.preserve_xdg_state, sandbox_name, path)?;
    let current = match current {
        Some(contents) => String::from_utf8(contents)
            .map_err(|error| format!("sandbox Cargo config is not UTF-8: {error}"))?,
        None => String::new(),
    };
    let updated = cargo_config_with_git_cli(&current);
    if updated.trim_end() != current.trim_end() {
        sbx::write_file(
            context.preserve_xdg_state,
            sandbox_name,
            path,
            OsStr::new(&updated),
        )?;
        println!("==> configured Cargo Git dependencies to use the forwarded SSH agent");
    }
    Ok(())
}

fn cargo_config_with_git_cli(contents: &str) -> String {
    let mut lines = contents.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut section = "";
    let mut net_header = None;
    let mut setting = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed;
            if section == "[net]" {
                net_header = Some(index);
            }
            continue;
        }
        let key = trimmed.split_once('=').map(|(key, _)| key.trim());
        if (section.is_empty() && key == Some("net.git-fetch-with-cli"))
            || (section == "[net]" && key == Some("git-fetch-with-cli"))
        {
            setting = Some((index, section.is_empty()));
            break;
        }
    }
    if let Some((index, dotted)) = setting {
        let indentation = lines[index]
            .chars()
            .take_while(|character| character.is_whitespace())
            .collect::<String>();
        let key = if dotted {
            "net.git-fetch-with-cli"
        } else {
            "git-fetch-with-cli"
        };
        lines[index] = format!("{indentation}{key} = true");
    } else if let Some(index) = net_header {
        lines.insert(index + 1, "git-fetch-with-cli = true".to_owned());
    } else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.extend(["[net]".to_owned(), "git-fetch-with-cli = true".to_owned()]);
    }
    lines.join("\n")
}

fn mirror_host_github_rewrites(
    context: &Context,
    sandbox_name: &str,
    project: &Path,
) -> Result<(), String> {
    let rewrites = host_github_ssh_rewrites(project)?;
    for (key, value) in &rewrites {
        let script = r#"set -eu
key=$1
value=$2
if git config --global --get-all "$key" 2>/dev/null | grep -F -x -- "$value" >/dev/null; then
  exit 0
fi
git config --global --add "$key" "$value"
"#;
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state)
                .arg(sandbox_name)
                .arg("--")
                .args(["sh", "-c", script, "sh", key, value]),
            "mirroring a safe GitHub HTTPS-to-SSH rewrite",
        )?;
    }
    if !rewrites.is_empty() {
        println!(
            "==> mirrored {} safe GitHub HTTPS-to-SSH rewrite(s)",
            rewrites.len()
        );
    }
    Ok(())
}

fn host_github_ssh_rewrites(project: &Path) -> Result<Vec<(String, String)>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["config", "--get-regexp", r"^url\..*\.insteadof$"])
        .output()
        .map_err(|error| format!("could not inspect host Git URL rewrites: {error}"))?;
    if output.status.code() == Some(1) {
        return Ok(Vec::new());
    }
    if !output.status.success() {
        return Err(format!(
            "could not inspect host Git URL rewrites: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|error| format!("host Git URL rewrites are not UTF-8: {error}"))?;
    let mut rewrites = text
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .filter_map(|(key, value)| safe_github_ssh_rewrite(key, value.trim_start()))
        .collect::<Vec<_>>();
    rewrites.sort();
    rewrites.dedup();
    Ok(rewrites)
}

fn safe_github_ssh_rewrite(key: &str, instead_of: &str) -> Option<(String, String)> {
    let base = key.strip_prefix("url.")?.strip_suffix(".insteadof")?;
    let https_suffix = instead_of.strip_prefix("https://github.com/")?;
    let ssh_suffix = base
        .strip_prefix("git@github.com:")
        .or_else(|| base.strip_prefix("ssh://git@github.com/"))?;
    if https_suffix != ssh_suffix || !safe_github_path_suffix(https_suffix) {
        return None;
    }
    Some((key.to_owned(), instead_of.to_owned()))
}

fn safe_github_path_suffix(value: &str) -> bool {
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return false;
    }
    let trimmed = value.trim_end_matches('/');
    if trimmed.is_empty() {
        return value.is_empty();
    }
    trimmed
        .split('/')
        .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
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

    #[test]
    fn accepts_only_same_scope_github_https_to_ssh_rewrites() {
        assert_eq!(
            safe_github_ssh_rewrite(
                "url.git@github.com:FairgateLabs/.insteadof",
                "https://github.com/FairgateLabs/",
            ),
            Some((
                "url.git@github.com:FairgateLabs/.insteadof".to_owned(),
                "https://github.com/FairgateLabs/".to_owned(),
            ))
        );
        assert!(
            safe_github_ssh_rewrite("url.ssh://git@github.com/.insteadof", "https://github.com/")
                .is_some()
        );
        assert!(
            safe_github_ssh_rewrite(
                "url.git@github.com:Other/.insteadof",
                "https://github.com/FairgateLabs/"
            )
            .is_none()
        );
        assert!(
            safe_github_ssh_rewrite(
                "url.file:///tmp/.insteadof",
                "https://github.com/FairgateLabs/"
            )
            .is_none()
        );
        assert!(
            safe_github_ssh_rewrite(
                "url.git@evil.example:FairgateLabs/.insteadof",
                "https://github.com/FairgateLabs/"
            )
            .is_none()
        );
    }

    #[test]
    fn enables_cargo_git_cli_without_discarding_other_configuration() {
        assert_eq!(
            cargo_config_with_git_cli(""),
            "[net]\ngit-fetch-with-cli = true"
        );
        assert_eq!(
            cargo_config_with_git_cli("[build]\njobs = 4"),
            "[build]\njobs = 4\n\n[net]\ngit-fetch-with-cli = true"
        );
        assert_eq!(
            cargo_config_with_git_cli("[net]\nretry = 3\ngit-fetch-with-cli = false"),
            "[net]\nretry = 3\ngit-fetch-with-cli = true"
        );
        assert_eq!(
            cargo_config_with_git_cli("net.git-fetch-with-cli = false\n[build]\njobs = 4"),
            "net.git-fetch-with-cli = true\n[build]\njobs = 4"
        );
    }
}
