use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost};
use crate::{process, sandbox};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub(crate) fn import(
    context: &Context,
    project: &Project,
    preset: Preset,
    rocksdb: Option<&RocksDbHost>,
) -> Result<(), String> {
    sandbox::ensure_dev(context, project, preset, rocksdb)?;
    if !preset.has_claude() {
        return Err(format!(
            "the {} preset has no sandbox-local subscription cache to import",
            preset.name()
        ));
    }
    confirm_import(&project.name)?;
    let home = process::home_dir()?;
    let claude = env::var_os("SBXR_CLAUDE_AUTH_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude/.credentials.json"));

    if preset.has_codex() {
        println!("note: Codex uses Docker host-managed OAuth; its auth.json is never copied");
    }
    if claude.is_file() {
        import_file(
            context,
            &project.name,
            &claude,
            "umask 077; mkdir -p \"$HOME/.claude\"; cat > \"$HOME/.claude/.credentials.json\"",
        )?;
        println!("==> imported host Claude authentication");
    } else {
        return Err(format!(
            "no readable host Claude cache at {}; use native login inside the sandbox",
            claude.display()
        ));
    }
    show_status(context, &project.name, preset);
    Ok(())
}

fn confirm_import(sandbox_name: &str) -> Result<(), String> {
    if env::var("SBXR_YES").as_deref() == Ok("1") {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        return Err("auth import needs an interactive terminal or SBXR_YES=1".to_owned());
    }
    println!("This copies OAuth refresh credentials into {sandbox_name}.");
    println!("Code, build scripts, proc macros, and agents in that sandbox can read them.");
    print!("Import credentials into this trusted project sandbox? [y/N] ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
        Ok(())
    } else {
        Err("authentication import cancelled".to_owned())
    }
}

fn import_file(
    context: &Context,
    sandbox_name: &str,
    source: &Path,
    script: &str,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    fs::File::open(source)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| format!("could not read {}: {error}", source.display()))?;
    let mut child = process::sbx_command(context.preserve_xdg_state)
        .arg("exec")
        .arg(sandbox_name)
        .arg("--")
        .args(["sh", "-c", script])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start credential import: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("credential import stdin was unavailable")?
        .write_all(&bytes)
        .map_err(|error| format!("could not stream credential: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("could not finish credential import: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("credential import failed with {status}"))
    }
}

pub(crate) fn show_status(context: &Context, sandbox_name: &str, preset: Preset) {
    if preset.has_codex() {
        println!("Codex (Docker host-managed OAuth):");
        let _ = process::sbx_command(context.preserve_xdg_state)
            .args(["secret", "ls", "--service", "openai"])
            .status();
    }
    if preset.has_claude() {
        println!("Claude (sandbox-local subscription OAuth):");
        let _ = process::sbx_command(context.preserve_xdg_state)
            .args([
                "exec",
                sandbox_name,
                "--",
                "claude",
                "auth",
                "status",
                "--text",
            ])
            .status();
    }
    if preset == Preset::MultiAgent {
        println!("Pi via Docker-managed Codex OAuth:");
        let _ = process::sbx_command(context.preserve_xdg_state)
            .args([
                "exec",
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
        println!("Pi via detected sandbox-local Claude OAuth:");
        let _ = process::sbx_command(context.preserve_xdg_state)
            .args([
                "exec",
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
