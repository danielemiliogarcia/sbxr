use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost};
use crate::sbx;

const ANTHROPIC_PROXY_PROBE: &str = r#"
set -eu
status=$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
  --max-time 20 \
  -H 'Authorization: Bearer sk-ant-oat01-proxy-managed' \
  -H 'anthropic-version: 2023-06-01' \
  -H 'anthropic-beta: oauth-2025-04-20' \
  'https://api.anthropic.com/v1/models?limit=1')
case "$status" in
  2*) exit 0 ;;
  *) echo "Anthropic proxy probe returned HTTP $status" >&2; exit 1 ;;
esac
"#;

pub(crate) fn import(
    _context: &Context,
    _project: &Project,
    _preset: Preset,
    _rocksdb: Option<&RocksDbHost>,
) -> Result<(), String> {
    Err(
        "`sbxr auth-import` was removed because copying Claude OAuth credentials into a sandbox exposes them to sandbox code; run `sbxr claude .` and use `/login` so Docker's host proxy stores the real token"
            .to_owned(),
    )
}

pub(crate) fn show_status(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
) -> Result<(), String> {
    println!("GitHub (Docker host-proxied sentinel; no reusable token in guest):");
    let _ = sbx::exec_command(context.preserve_xdg_state)
        .args([
            sandbox_name,
            "--",
            "gh",
            "auth",
            "status",
            "--hostname",
            "github.com",
        ])
        .status();
    if preset.has_codex() {
        println!("Codex (Docker host-managed OAuth):");
        let _ = sbx::command(context.preserve_xdg_state)
            .args(["secret", "ls", "--service", "openai"])
            .status();
    }
    if preset.has_claude() {
        println!("Claude CLI credential view (Docker host-managed OAuth):");
        let _ = sbx::exec_command(context.preserve_xdg_state)
            .args([sandbox_name, "--", "claude", "auth", "status", "--text"])
            .status();
        println!("Anthropic proxy route (real request with guest sentinel; no model tokens):");
        verify_anthropic_proxy(context, sandbox_name)?;
        println!("ok   host-managed Anthropic OAuth proxy is working");
    }
    if preset == Preset::MultiAgent {
        println!("Pi Codex sentinel configuration:");
        let _ = sbx::exec_command(context.preserve_xdg_state)
            .args([
                sandbox_name,
                "--",
                "pi",
                "auth",
                "check",
                "--provider",
                "openai-codex",
                "--no-refresh",
            ])
            .status();
        println!("Pi Claude sentinel configuration:");
        let _ = sbx::exec_command(context.preserve_xdg_state)
            .args([
                sandbox_name,
                "--",
                "pi",
                "auth",
                "check",
                "--provider",
                "anthropic",
                "--no-refresh",
            ])
            .status();
    }
    Ok(())
}

pub(crate) fn verify_anthropic_proxy(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let output = sbx::exec_command(context.preserve_xdg_state)
        .args([sandbox_name, "--", "sh", "-c", ANTHROPIC_PROXY_PROBE])
        .output()
        .map_err(|error| format!("could not run Anthropic proxy probe: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if detail.is_empty() {
            "Anthropic proxy probe failed; verify the host binding with `sbx secret ls --service anthropic`"
                .to_owned()
        } else {
            format!("{detail}; verify the host binding with `sbx secret ls --service anthropic`")
        })
    }
}
