use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost};
use crate::sbx;

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

pub(crate) fn show_status(context: &Context, sandbox_name: &str, preset: Preset) {
    if preset.has_codex() {
        println!("Codex (Docker host-managed OAuth):");
        let _ = sbx::command(context.preserve_xdg_state)
            .args(["secret", "ls", "--service", "openai"])
            .status();
    }
    if preset.has_claude() {
        println!("Claude (Docker host-managed OAuth):");
        let _ = sbx::exec_command(context.preserve_xdg_state)
            .args([sandbox_name, "--", "claude", "auth", "status", "--text"])
            .status();
    }
    if preset == Preset::MultiAgent {
        println!("Pi via Docker-managed Codex OAuth:");
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
        println!("Pi via Docker-managed Claude OAuth:");
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
}
