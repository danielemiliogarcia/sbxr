use crate::environment::Context;
use crate::sbx::RuntimeInfo;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Report {
    path: PathBuf,
    command: String,
    events: Vec<Value>,
    runtime: Option<Value>,
}

impl Report {
    pub(crate) fn start(context: &Context, command: &str) -> Result<Self, String> {
        let directory = context.state_root.join("diagnostics");
        secure_directory(&context.state_root)?;
        secure_directory(&directory)?;
        let timestamp = timestamp_millis();
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "{}-{timestamp}-{}-{sequence}.json",
            safe_component(command),
            std::process::id()
        ));
        reject_unsafe_file(&path)?;
        let mut report = Self {
            path,
            command: command.to_owned(),
            events: Vec::new(),
            runtime: None,
        };
        report.event("command", "started", None)?;
        Ok(report)
    }

    pub(crate) fn event(
        &mut self,
        stage: &str,
        status: &str,
        category: Option<&str>,
    ) -> Result<(), String> {
        self.events.push(json!({
            "timestampMs": timestamp_millis(),
            "pid": std::process::id(),
            "stage": safe_component(stage),
            "status": safe_component(status),
            "category": category.map(safe_component),
            "reason": category.map(safe_component),
            "source": "sbxr",
        }));
        self.flush()
    }

    pub(crate) fn runtime(&mut self, info: &RuntimeInfo) -> Result<(), String> {
        self.runtime = Some(json!({
            "client": info.client.to_string(),
            "server": info.server.to_string(),
            "api": info.api_version,
            "serverState": safe_component(&info.server_state),
        }));
        self.flush()
    }

    pub(crate) fn complete(mut self) -> Result<PathBuf, String> {
        self.event("command", "completed", None)?;
        Ok(self.path)
    }

    pub(crate) fn fail(mut self, error: &str) -> Result<PathBuf, String> {
        self.event("command", "failed", Some(classify(error)))?;
        Ok(self.path)
    }

    fn flush(&self) -> Result<(), String> {
        let document = serde_json::to_vec_pretty(&json!({
            "version": 1,
            "command": self.command,
            "events": self.events,
            "runtime": self.runtime,
            "sbxrVersion": env!("CARGO_PKG_VERSION"),
            "privacy": {
                "argumentsRecorded": false,
                "environmentRecorded": false,
                "rawErrorsRecorded": false,
                "commandOutputRecorded": false,
            }
        }))
        .map_err(|error| format!("could not serialize diagnostic report: {error}"))?;
        atomic_write(&self.path, &document)
    }
}

pub(crate) fn summarize_latest(context: &Context) -> Result<(), String> {
    let directory = context.state_root.join("diagnostics");
    let mut reports = match fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).ok()?;
                (metadata.file_type().is_file()
                    && path.extension().is_some_and(|ext| ext == "json"))
                .then_some((metadata.modified().ok()?, path))
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("no sbxr diagnostic reports exist yet".to_owned());
        }
        Err(error) => {
            return Err(format!(
                "could not inspect {}: {error}",
                directory.display()
            ));
        }
    };
    reports.sort_by_key(|entry| entry.0);
    let path = reports
        .last()
        .map(|(_, path)| path)
        .ok_or("no sbxr diagnostic reports exist yet")?;
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid diagnostic report {}: {error}", path.display()))?;
    if document.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(format!("unsupported diagnostic report: {}", path.display()));
    }
    println!(
        "diagnostic report (newest across all projects): {}",
        path.display()
    );
    println!(
        "command: {}",
        document
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    );
    if let Some(runtime) = document.get("runtime").and_then(Value::as_object) {
        println!(
            "runtime: client {} · server {} · API {}",
            runtime
                .get("client")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            runtime
                .get("server")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            runtime
                .get("api")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        );
    }
    let events = document
        .get("events")
        .and_then(Value::as_array)
        .ok_or("diagnostic report has no events array")?;
    for event in events {
        let stage = event
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let status = event
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let category = event.get("category").and_then(Value::as_str);
        println!(
            "  {stage:<20} {status}{}",
            category.map_or(String::new(), |value| format!(" ({value})"))
        );
    }
    Ok(())
}

fn classify(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("credential")
        || lower.contains("oauth")
        || lower.contains("secret")
        || lower.contains("authentication")
    {
        "authentication"
    } else if lower.contains("network") || lower.contains("policy") {
        "network-policy"
    } else if lower.contains("contract") || lower.contains("recreat") {
        "contract-drift"
    } else if lower.contains("version") || lower.contains("unsupported") {
        "compatibility"
    } else if lower.contains("kvm") || lower.contains("daemon") || lower.contains("docker") {
        "sandbox-runtime"
    } else if lower.contains("vscode") || lower.contains("remote-ssh") {
        "editor-integration"
    } else if lower.contains("toolchain") || lower.contains("rustup") {
        "toolchain"
    } else if lower.contains("profile") || lower.contains("wizard") {
        "configuration"
    } else if lower.contains("provider")
        || lower.contains("rate-limit")
        || lower.contains("model-unavailable")
    {
        "provider"
    } else if lower.contains("orca")
        || lower.contains("paseo")
        || lower.contains("browser")
        || lower.contains("integration")
    {
        "integration"
    } else {
        "operation"
    }
}

fn safe_component(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() || character == '-' {
            output.push(character);
            separator = false;
        } else if !separator {
            output.push('-');
            separator = true;
        }
    }
    output.trim_matches('-').chars().take(64).collect()
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn secure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "diagnostic path is not a real directory: {}",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)
                .map_err(|error| format!("could not create {}: {error}", path.display()))?;
        }
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect {}: {error}", path.display()))?;
    Ok(())
}

fn reject_unsafe_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "refusing to replace existing diagnostic file: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("diagnostic path has no parent: {}", path.display()))?;
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".report-{}-{sequence}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("could not create {}: {error}", temporary.display()))?;
    let result = (|| {
        file.write_all(contents)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|error| format!("could not replace {}: {error}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_classification_is_redacted_and_stable() {
        assert_eq!(classify("token value abc failed OAuth"), "authentication");
        assert_eq!(classify("rustup toolchain missing"), "toolchain");
        assert_eq!(classify("live pi provider-error"), "provider");
        assert_eq!(classify("Orca failed to start"), "integration");
        assert_eq!(classify("arbitrary private detail"), "operation");
    }

    #[test]
    fn components_never_preserve_paths_or_spaces() {
        assert_eq!(
            safe_component("new /home/user/secret"),
            "new-home-user-secret"
        );
    }
}
