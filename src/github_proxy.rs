use crate::environment::Context;
use crate::{process, sbx, sha256};
use std::ffi::OsStr;
use std::process::Stdio;

const VERSION: &str = "2.102.0";
const AMD64_SHA256: &str = "bb766f710eef8ede859c18578c72c327597cd4c8a85b06001b1f3843c6019386";
const ARM64_SHA256: &str = "7862c86c72f43df3a2d93ddde6f473285b4e2af61b494849846827e513ef6484";
const MARKER: &str = "/home/agent/.local/state/sbxr/github-proxy-v1.sha256";
const HELPER: &str = r#"set -euo pipefail
git config --global --unset-all credential.https://github.com.helper || true
git config --global --add credential.https://github.com.helper '!f() { if [ "$1" = get ]; then printf "%s\n" username=x-access-token "password=${GH_TOKEN:?missing Docker GitHub proxy sentinel}"; fi; }; f'
git config --global credential.https://github.com.useHttpPath true
git config --global url."https://github.com/".insteadOf "git://github.com/"
git config --global --get credential.https://github.com.helper >/dev/null
gh --version | head -n1
"#;

pub(crate) fn preflight_host() -> Result<(), String> {
    process::require("gh")?;
    let output = process::output_checked(
        std::process::Command::new("gh").args(["auth", "status", "--hostname", "github.com"]),
        "checking host GitHub CLI authentication",
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err("GitHub CLI is not authenticated; run `gh auth login`, then retry".to_owned())
    }
}

pub(crate) fn ensure(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let wanted =
        sha256::hex(format!("{VERSION}\0{AMD64_SHA256}\0{ARM64_SHA256}\0{HELPER}").as_bytes());
    let ready = sbx::read_file(context.preserve_xdg_state, sandbox_name, MARKER)?
        .as_deref()
        .map(|value| value.trim_ascii())
        == Some(wanted.as_bytes())
        && sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args(["--", "gh", "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    if ready {
        return Ok(());
    }
    let script = format!(
        r#"set -euo pipefail
version='{VERSION}'
case "$(uname -m)" in
  x86_64|amd64) arch=amd64; expected='{AMD64_SHA256}' ;;
  aarch64|arm64) arch=arm64; expected='{ARM64_SHA256}' ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
if ! gh --version 2>/dev/null | head -n1 | grep -Fq "gh version $version"; then
  archive=/tmp/gh.tar.gz
  directory="gh_${{version}}_linux_${{arch}}"
  curl --proto '=https' --tlsv1.2 -fsSL -o "$archive" \
    "https://github.com/cli/cli/releases/download/v${{version}}/gh_${{version}}_linux_${{arch}}.tar.gz"
  printf '%s  %s\n' "$expected" "$archive" | sha256sum -c -
  rm -rf "/tmp/$directory"
  tar -xzf "$archive" -C /tmp
  install -m 0755 "/tmp/$directory/bin/gh" /usr/local/bin/gh
  rm -rf "$archive" "/tmp/$directory"
fi
"#
    );
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state).args([
            "-u",
            "root",
            sandbox_name,
            "bash",
            "-lc",
            &script,
        ]),
        "installing the checksum-pinned GitHub CLI",
    )?;
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args(["--", "bash", "-lc", HELPER]),
        "configuring GitHub HTTPS proxy authentication",
    )?;
    sbx::write_file(
        context.preserve_xdg_state,
        sandbox_name,
        MARKER,
        OsStr::new(&wanted),
    )
}
