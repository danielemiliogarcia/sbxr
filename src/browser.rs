use crate::cli::Preset;
use crate::environment::Context;
use crate::{process, sbx};
use std::process::Stdio;

const MCP_NAME: &str = "sbxr-browser";

pub(crate) fn configure(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
) -> Result<(), String> {
    let agent = if preset.has_codex() {
        "codex"
    } else {
        "claude"
    };
    let present = sbx::exec_command(context.preserve_xdg_state)
        .arg(sandbox_name)
        .args(["--", agent, "mcp", "get", MCP_NAME])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if present {
        return Ok(());
    }
    println!("==> registering the isolated Playwright browser MCP");
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args([
                "--",
                agent,
                "mcp",
                "add",
                MCP_NAME,
                "--",
                "/usr/local/bin/sbxr-browser-mcp",
            ]),
        "registering the sandbox browser MCP",
    )
}
