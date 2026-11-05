use crate::environment::{Context, Project};
use serde_json::{Map, Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkProfile {
    Development,
    Restricted,
    Offline,
}

impl NetworkProfile {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "development" | "dev" => Ok(Self::Development),
            "restricted" => Ok(Self::Restricted),
            "offline" => Ok(Self::Offline),
            _ => Err(format!(
                "unknown network profile '{value}'; expected development, restricted, or offline"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Restricted => "restricted",
            Self::Offline => "offline",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceMode {
    Shared,
    Isolated,
}

impl WorkspaceMode {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "shared" | "direct" => Ok(Self::Shared),
            "isolated" | "clone" => Ok(Self::Isolated),
            _ => Err(format!(
                "unknown workspace mode '{value}'; expected shared or isolated"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Resources {
    pub(crate) memory: Option<String>,
    pub(crate) cpus: Option<u32>,
    pub(crate) root_disk: Option<String>,
    pub(crate) docker_disk: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Profile {
    pub(crate) workspace: WorkspaceMode,
    pub(crate) network: NetworkProfile,
    pub(crate) browser: bool,
    pub(crate) orca: bool,
    pub(crate) ghcr: bool,
    pub(crate) paseo_relay: bool,
    pub(crate) github_proxy: bool,
    pub(crate) resources: Resources,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            workspace: WorkspaceMode::Shared,
            network: NetworkProfile::Development,
            browser: false,
            orca: false,
            ghcr: false,
            paseo_relay: false,
            github_proxy: true,
            resources: Resources::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Overrides {
    pub(crate) config: Option<PathBuf>,
    pub(crate) wizard: bool,
    pub(crate) memory: Option<String>,
    pub(crate) cpus: Option<u32>,
    pub(crate) root_disk: Option<String>,
    pub(crate) docker_disk: Option<String>,
    pub(crate) network: Option<NetworkProfile>,
    pub(crate) workspace: Option<WorkspaceMode>,
    pub(crate) browser: Option<bool>,
    pub(crate) orca: Option<bool>,
    pub(crate) ghcr: Option<bool>,
    pub(crate) paseo_relay: Option<bool>,
}

impl Overrides {
    pub(crate) fn has_explicit_values(&self) -> bool {
        self.config.is_some()
            || self.memory.is_some()
            || self.cpus.is_some()
            || self.root_disk.is_some()
            || self.docker_disk.is_some()
            || self.network.is_some()
            || self.workspace.is_some()
            || self.browser.is_some()
            || self.orca.is_some()
            || self.ghcr.is_some()
            || self.paseo_relay.is_some()
    }
}

pub(crate) fn resolve(
    context: &Context,
    project: &Project,
    overrides: &Overrides,
) -> Result<Profile, String> {
    let mut profile = match &overrides.config {
        Some(path) => read_external(path)?,
        None => read_saved(context, project)?.unwrap_or_default(),
    };
    apply_overrides(&mut profile, overrides);
    validate(&profile, &context.state_root)?;
    if overrides.has_explicit_values() {
        save(context, project, &profile)?;
    }
    Ok(profile)
}

pub(crate) fn run_wizard(
    context: &Context,
    project: &Project,
    current: &Profile,
) -> Result<Profile, String> {
    if !io::stdin().is_terminal() {
        return Err("--wizard requires an interactive terminal".to_owned());
    }
    println!("sbxr wizard: press Enter to keep the value shown in brackets");
    let workspace = prompt_choice(
        "Workspace mode",
        current.workspace.name(),
        &["shared", "isolated"],
    )?;
    let network = prompt_choice(
        "Network profile",
        current.network.name(),
        &["development", "restricted", "offline"],
    )?;
    let memory = prompt_size("Memory override", current.resources.memory.as_deref())?;
    let cpus = prompt_cpus(current.resources.cpus)?;
    let root_disk = prompt_size("Root disk override", current.resources.root_disk.as_deref())?;
    let docker_disk = prompt_size(
        "Docker disk override",
        current.resources.docker_disk.as_deref(),
    )?;
    let browser = prompt_bool("Install the Playwright browser MCP", current.browser)?;
    let orca = prompt_bool("Install the Orca remote server", current.orca)?;
    let ghcr = prompt_bool("Enable private GHCR proxy credentials", current.ghcr)?;
    let paseo_relay = prompt_bool(
        "Enable the authenticated Paseo remote relay",
        current.paseo_relay,
    )?;

    let profile = Profile {
        workspace: WorkspaceMode::parse(&workspace)?,
        network: NetworkProfile::parse(&network)?,
        browser,
        orca,
        ghcr,
        paseo_relay,
        github_proxy: true,
        resources: Resources {
            memory,
            cpus,
            root_disk,
            docker_disk,
        },
    };
    validate(&profile, &context.state_root)?;
    save(context, project, &profile)?;
    println!(
        "saved protected profile: {}",
        path(context, project).display()
    );
    Ok(profile)
}

pub(crate) fn print(context: &Context, project: &Project, profile: &Profile) -> Result<(), String> {
    println!("profile: {}", path(context, project).display());
    let document = serde_json::to_string_pretty(&to_json(profile))
        .map_err(|error| format!("could not serialize profile: {error}"))?;
    println!("{document}");
    Ok(())
}

pub(crate) fn contract_bytes(profile: &Profile) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&to_contract_json(profile))
        .map_err(|error| format!("could not serialize profile contract: {error}"))
}

fn apply_overrides(profile: &mut Profile, overrides: &Overrides) {
    if let Some(value) = &overrides.memory {
        profile.resources.memory = Some(value.clone());
    }
    if let Some(value) = overrides.cpus {
        profile.resources.cpus = Some(value);
    }
    if let Some(value) = &overrides.root_disk {
        profile.resources.root_disk = Some(value.clone());
    }
    if let Some(value) = &overrides.docker_disk {
        profile.resources.docker_disk = Some(value.clone());
    }
    if let Some(value) = overrides.network {
        profile.network = value;
    }
    if let Some(value) = overrides.workspace {
        profile.workspace = value;
    }
    if let Some(value) = overrides.browser {
        profile.browser = value;
    }
    if let Some(value) = overrides.orca {
        profile.orca = value;
    }
    if let Some(value) = overrides.ghcr {
        profile.ghcr = value;
    }
    if let Some(value) = overrides.paseo_relay {
        profile.paseo_relay = value;
    }
}

fn path(context: &Context, project: &Project) -> PathBuf {
    context
        .state_root
        .join("profiles")
        .join(format!("{}.json", project.name))
}

fn read_saved(context: &Context, project: &Project) -> Result<Option<Profile>, String> {
    let path = path(context, project);
    for directory in [&context.state_root, &context.state_root.join("profiles")] {
        if directory.exists() {
            inspect_protected_directory(directory)?;
        }
    }
    inspect_protected_file(&path)?;
    match fs::read(&path) {
        Ok(bytes) => parse_json(&bytes, &path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

fn inspect_protected_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "protected profile path is not a real directory: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "protected profile directory is accessible to group/other users: {}",
            path.display()
        ));
    }
    Ok(())
}

fn inspect_protected_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "protected profile path is not a regular file: {}",
            path.display()
        )),
        Ok(metadata) => {
            #[cfg(unix)]
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(format!(
                    "protected profile is accessible to group/other users: {}",
                    path.display()
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn read_external(path: &Path) -> Result<Profile, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read profile {}: {error}", path.display()))?;
    parse_json(&bytes, path)
}

fn parse_json(bytes: &[u8], path: &Path) -> Result<Profile, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid profile {}: {error}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("profile {} must be a JSON object", path.display()))?;
    reject_unknown(
        object,
        &[
            "version",
            "workspace",
            "network",
            "githubProxy",
            "integrations",
            "resources",
        ],
        "profile",
    )?;
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(format!("profile {} must have version 1", path.display()));
    }
    let workspace = match object.get("workspace") {
        None => WorkspaceMode::Shared,
        Some(Value::String(value)) => WorkspaceMode::parse(value)?,
        Some(_) => return Err("workspace must be shared or isolated".to_owned()),
    };
    let network = match object.get("network") {
        None => NetworkProfile::Development,
        Some(Value::String(value)) => NetworkProfile::parse(value)?,
        Some(_) => return Err("network must be development, restricted, or offline".to_owned()),
    };
    let github_proxy = match object.get("githubProxy") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err("githubProxy must be true".to_owned()),
    };
    if !github_proxy {
        return Err(
            "githubProxy=false is unsupported; proxy isolation is a default sbxr security control"
                .to_owned(),
        );
    }

    let integrations = match object.get("integrations") {
        None => Map::new(),
        Some(Value::Object(value)) => value.clone(),
        Some(_) => return Err("integrations must be an object".to_owned()),
    };
    reject_unknown(
        &integrations,
        &["browser", "orca", "ghcr", "paseoRelay"],
        "integrations",
    )?;
    let resources = match object.get("resources") {
        None => Map::new(),
        Some(Value::Object(value)) => value.clone(),
        Some(_) => return Err("resources must be an object".to_owned()),
    };
    reject_unknown(
        &resources,
        &["memory", "cpus", "rootDisk", "dockerDisk"],
        "resources",
    )?;

    Ok(Profile {
        workspace,
        network,
        browser: bool_value(&integrations, "browser")?,
        orca: bool_value(&integrations, "orca")?,
        ghcr: bool_value(&integrations, "ghcr")?,
        paseo_relay: bool_value(&integrations, "paseoRelay")?,
        github_proxy,
        resources: Resources {
            memory: string_value(&resources, "memory")?,
            cpus: integer_value(&resources, "cpus")?,
            root_disk: string_value(&resources, "rootDisk")?,
            docker_disk: string_value(&resources, "dockerDisk")?,
        },
    })
}

fn reject_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    for key in object.keys() {
        let lower = key.to_ascii_lowercase();
        if lower.contains("token")
            || lower.contains("secret")
            || lower.contains("password")
            || lower.contains("credential")
            || lower.contains("apikey")
            || lower.contains("api_key")
        {
            return Err(format!(
                "{label} contains secret-looking key '{key}'; profiles must be nonsecret"
            ));
        }
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown {label} key '{key}'"));
        }
    }
    Ok(())
}

fn bool_value(object: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match object.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

fn string_value(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(normalize_size(value, key)?)),
        Some(_) => Err(format!("{key} must be a size string such as 8g")),
    }
}

fn integer_value(object: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| *number > 0)
            .map(Some)
            .ok_or_else(|| format!("{key} must be a positive integer")),
    }
}

fn to_json(profile: &Profile) -> Value {
    json!({
        "version": 1,
        "workspace": profile.workspace.name(),
        "network": profile.network.name(),
        "githubProxy": profile.github_proxy,
        "integrations": {
            "browser": profile.browser,
            "orca": profile.orca,
            "ghcr": profile.ghcr,
            "paseoRelay": profile.paseo_relay,
        },
        "resources": {
            "memory": profile.resources.memory,
            "cpus": profile.resources.cpus,
            "rootDisk": profile.resources.root_disk,
            "dockerDisk": profile.resources.docker_disk,
        }
    })
}

/// Only creation-time choices belong in the Docker environment contract.
/// Network policy and the Paseo relay are reconciled at runtime, so changing
/// either never requires deleting a healthy sandbox.
fn to_contract_json(profile: &Profile) -> Value {
    json!({
        "version": 1,
        "workspace": profile.workspace.name(),
        "githubProxy": profile.github_proxy,
        "integrations": {
            "browser": profile.browser,
            "orca": profile.orca,
            "ghcr": profile.ghcr,
        },
        "resources": {
            "memory": profile.resources.memory,
            "cpus": profile.resources.cpus,
            "rootDisk": profile.resources.root_disk,
            "dockerDisk": profile.resources.docker_disk,
        }
    })
}

fn save(context: &Context, project: &Project, profile: &Profile) -> Result<(), String> {
    let directory = context.state_root.join("profiles");
    secure_directory(&context.state_root)?;
    secure_directory(&directory)?;
    let path = path(context, project);
    reject_symlink(&path)?;
    let bytes = serde_json::to_vec_pretty(&to_json(profile))
        .map_err(|error| format!("could not serialize profile: {error}"))?;
    atomic_write(&path, &bytes)
}

pub(crate) fn validate(profile: &Profile, state_root: &Path) -> Result<(), String> {
    if profile.paseo_relay && profile.network == NetworkProfile::Offline {
        return Err(
            "Paseo relay requires outbound relay.paseo.sh access and cannot be combined with the offline network profile"
                .to_owned(),
        );
    }
    let memory = profile
        .resources
        .memory
        .as_deref()
        .map(|value| size_mib(value, "memory", 512))
        .transpose()?;
    let root = profile
        .resources
        .root_disk
        .as_deref()
        .map(|value| size_mib(value, "root disk", 1024))
        .transpose()?;
    let docker = profile
        .resources
        .docker_disk
        .as_deref()
        .map(|value| size_mib(value, "Docker disk", 512))
        .transpose()?;

    let host_cpus = std::thread::available_parallelism()
        .map(|value| value.get() as u32)
        .unwrap_or(1);
    if let Some(cpus) = profile.resources.cpus {
        if cpus == 0 || cpus > host_cpus {
            return Err(format!(
                "requested {cpus} CPUs, but this host reports {host_cpus} available"
            ));
        }
        if cpus.saturating_mul(4) > host_cpus.saturating_mul(3) {
            println!(
                "WARN requested {cpus} of {host_cpus} host CPUs; this leaves little host headroom"
            );
        }
    }
    if let (Some(requested), Some(host)) = (memory, host_memory_mib()) {
        let maximum = (host.saturating_mul(3) / 4).max(512);
        if requested > maximum {
            return Err(format!(
                "requested memory {requested}m exceeds the recommended host maximum {maximum}m"
            ));
        }
        if requested.saturating_mul(2) > host {
            println!(
                "WARN requested {requested} MiB of {host} MiB host memory; this leaves limited host headroom"
            );
        }
    }
    if let (Some(root), Some(docker), Some(free)) = (root, docker, free_disk_mib(state_root)) {
        let reserve = (free / 5).max(20 * 1024);
        let budget = free.saturating_sub(reserve);
        if root.saturating_add(docker) > budget {
            return Err(format!(
                "requested root + Docker disks exceed the safe host budget ({budget}m available after reserve)"
            ));
        }
    }
    Ok(())
}

pub(crate) fn normalize_size(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let Some(unit) = normalized.chars().last() else {
        return Err(format!("{label} must be a size such as 8g or 2048m"));
    };
    if !matches!(unit, 'm' | 'g')
        || normalized[..normalized.len() - 1].is_empty()
        || !normalized[..normalized.len() - 1]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("{label} must be a size such as 8g or 2048m"));
    }
    Ok(normalized)
}

fn size_mib(value: &str, label: &str, minimum: u64) -> Result<u64, String> {
    let normalized = normalize_size(value, label)?;
    let (digits, unit) = normalized.split_at(normalized.len() - 1);
    let number = digits
        .parse::<u64>()
        .map_err(|error| format!("invalid {label}: {error}"))?;
    let mib = if unit == "g" {
        number
            .checked_mul(1024)
            .ok_or_else(|| format!("{label} is too large"))?
    } else {
        number
    };
    if mib < minimum {
        return Err(format!("{label} must be at least {minimum}m"));
    }
    Ok(mib)
}

fn host_memory_mib() -> Option<u64> {
    let contents = fs::read_to_string("/proc/meminfo").ok()?;
    let kib = contents
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    Some(kib / 1024)
}

fn free_disk_mib(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|candidate| candidate.exists())?;
    let output = Command::new("df")
        .args(["-Pk", "--"])
        .arg(existing)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .last()?
        .to_owned();
    let available_kib = line.split_whitespace().nth(3)?.parse::<u64>().ok()?;
    Some(available_kib / 1024)
}

fn prompt(label: &str, default: &str) -> Result<String, String> {
    print!("{label} [{default}]: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("could not flush wizard prompt: {error}"))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| format!("could not read wizard input: {error}"))?;
    let value = input.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

fn prompt_choice(label: &str, default: &str, choices: &[&str]) -> Result<String, String> {
    loop {
        let value = prompt(&format!("{label} ({})", choices.join("/")), default)?;
        if choices.contains(&value.as_str()) {
            return Ok(value);
        }
        println!("expected one of: {}", choices.join(", "));
    }
}

fn prompt_bool(label: &str, default: bool) -> Result<bool, String> {
    let default_text = if default { "Y/n" } else { "y/N" };
    loop {
        let value = prompt(label, default_text)?.to_ascii_lowercase();
        if value == default_text.to_ascii_lowercase() {
            return Ok(default);
        }
        match value.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("expected yes or no"),
        }
    }
}

fn prompt_size(label: &str, current: Option<&str>) -> Result<Option<String>, String> {
    loop {
        let default = current.unwrap_or("auto");
        let value = prompt(&format!("{label} (auto or SIZE)"), default)?;
        if value == "auto" || value == "none" {
            return Ok(None);
        }
        match normalize_size(&value, label) {
            Ok(value) => return Ok(Some(value)),
            Err(error) => println!("{error}"),
        }
    }
}

fn prompt_cpus(current: Option<u32>) -> Result<Option<u32>, String> {
    loop {
        let default = current.map_or_else(|| "auto".to_owned(), |value| value.to_string());
        let value = prompt("CPUs (auto or positive integer)", &default)?;
        if value == "auto" || value == "none" {
            return Ok(None);
        }
        if let Ok(value) = value.parse::<u32>()
            && value > 0
        {
            return Ok(Some(value));
        }
        println!("CPUs must be auto or a positive integer");
    }
}

fn secure_directory(path: &Path) -> Result<(), String> {
    reject_symlink(path)?;
    fs::create_dir_all(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect {}: {error}", path.display()))?;
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing symlinked protected profile path: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("protected profile path has no parent: {}", path.display()))?;
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".profile-{}-{sequence}.tmp", std::process::id()));
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
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "could not replace protected profile {}: {error}",
                path.display()
            )
        })
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
    fn rejects_unknown_and_secret_profile_keys() {
        let path = Path::new("profile.json");
        assert!(parse_json(br#"{"version":1,"unknown":true}"#, path).is_err());
        assert!(parse_json(br#"{"version":1,"apiToken":"x"}"#, path).is_err());
    }

    #[test]
    fn parses_complete_nonsecret_profile() {
        let profile = parse_json(
            br#"{
              "version": 1,
              "workspace": "isolated",
              "network": "restricted",
              "githubProxy": true,
              "integrations": {"browser": true, "orca": true, "ghcr": true, "paseoRelay": true},
              "resources": {"memory":"8G", "cpus":4, "rootDisk":"20g", "dockerDisk":"8g"}
            }"#,
            Path::new("profile.json"),
        )
        .unwrap();
        assert_eq!(profile.workspace, WorkspaceMode::Isolated);
        assert_eq!(profile.network, NetworkProfile::Restricted);
        assert_eq!(profile.resources.memory.as_deref(), Some("8g"));
        assert!(profile.browser && profile.orca && profile.ghcr && profile.paseo_relay);
    }

    #[test]
    fn runtime_relay_choice_does_not_change_creation_contract() {
        let base = Profile::default();
        let mut relay = base.clone();
        relay.paseo_relay = true;
        assert_eq!(
            contract_bytes(&base).unwrap(),
            contract_bytes(&relay).unwrap()
        );
    }

    #[test]
    fn runtime_network_choice_does_not_change_creation_contract() {
        let base = Profile::default();
        for network in [NetworkProfile::Restricted, NetworkProfile::Offline] {
            let mut changed = base.clone();
            changed.network = network;
            assert_eq!(
                contract_bytes(&base).unwrap(),
                contract_bytes(&changed).unwrap()
            );
        }

        let mut invalid = base;
        invalid.paseo_relay = true;
        invalid.network = NetworkProfile::Offline;
        assert!(validate(&invalid, Path::new("/tmp/state")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unsafe_protected_profile_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        use std::time::{SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "sbxr-profile-security-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let profile = root.join("profile.json");
        fs::write(&profile, b"{}\n").unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(inspect_protected_file(&profile).is_err());
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(inspect_protected_file(&profile).is_ok());

        let link = root.join("link.json");
        symlink(&profile, &link).unwrap();
        assert!(inspect_protected_file(&link).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_size_syntax_and_minimums() {
        assert_eq!(normalize_size("2048M", "memory").unwrap(), "2048m");
        assert!(size_mib("511m", "memory", 512).is_err());
        assert!(normalize_size("8g;echo bad", "memory").is_err());
    }
}
