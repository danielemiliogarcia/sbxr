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

const PROTECTED_HOST_STORES: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".config/gh",
    ".docker",
    ".aws",
    ".kube",
    ".azure",
    ".config/gcloud",
    ".codex",
    ".claude",
    ".claude.json",
    ".pi",
    ".netrc",
    ".git-credentials",
    ".password-store",
    ".local/share/keyrings",
    ".local/share/gnupg",
    ".config/sbxr",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".npmrc",
    ".yarnrc.yml",
    ".pypirc",
    ".vault-token",
    ".config/op",
    ".config/hub",
    ".terraform.d",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/microsoft-edge",
    ".config/microsoft-edge-beta",
    ".config/microsoft-edge-dev",
    ".config/vivaldi",
    ".config/opera",
    ".config/ungoogled-chromium",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Chromium",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Vivaldi",
    "Library/Application Support/com.operasoftware.Opera",
    "Library/Application Support/Firefox",
    "Library/Safari",
    "Library/Cookies",
    "Library/WebKit",
    "Library/Keychains",
    "AppData/Local/Google/Chrome",
    "AppData/Local/Chromium",
    "AppData/Local/BraveSoftware",
    "AppData/Local/Microsoft/Edge",
    "AppData/Local/Vivaldi",
    "AppData/Local/Mozilla",
    "AppData/Local/Microsoft/Windows/WebCache",
    "AppData/Roaming/Opera Software",
    "AppData/Roaming/Mozilla",
    "AppData/Local/Microsoft/Credentials",
    "AppData/Roaming/Microsoft/Credentials",
    ".config/pypoetry/auth.toml",
    ".gem/credentials",
    ".config/glab-cli/config.yml",
    ".config/helm/registry/config.json",
    ".pulumi/credentials.json",
    ".config/fish/fish_variables",
    ".config/git/credentials",
    ".config/containers/auth.json",
    ".config/rclone",
    ".m2/settings.xml",
    ".gradle/gradle.properties",
    // Login and environment files are where exported tokens usually live.
    // They are never needed for aliases, prompts, or executables.
    ".profile",
    ".bash_profile",
    ".bash_login",
    ".zprofile",
    ".zshenv",
    ".zlogin",
    ".env",
];

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellProfile {
    Familiar,
    Minimal,
    Custom,
}

impl ShellProfile {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "familiar" | "default" => Ok(Self::Familiar),
            "minimal" | "clean" | "none" => Ok(Self::Minimal),
            "custom" => Ok(Self::Custom),
            _ => Err(format!(
                "unknown shell profile '{value}'; expected familiar, minimal, or custom"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Familiar => "familiar",
            Self::Minimal => "minimal",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellProgram {
    Bash,
    Zsh,
    Fish,
}

impl ShellProgram {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            _ => Err(format!(
                "unknown login shell '{value}'; expected bash, zsh, or fish"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }

    pub(crate) fn login_arguments(self) -> &'static [&'static str] {
        match self {
            Self::Bash => &["bash", "-l"],
            Self::Zsh => &["zsh", "-l"],
            Self::Fish => &["fish", "-l"],
        }
    }

    pub(crate) fn guest_path(self) -> &'static str {
        match self {
            Self::Bash => "/bin/bash",
            Self::Zsh => "/usr/bin/zsh",
            Self::Fish => "/usr/bin/fish",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptStyle {
    Classic,
    Plain,
    Custom,
}

impl PromptStyle {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "classic" | "default" => Ok(Self::Classic),
            "plain" | "minimal" => Ok(Self::Plain),
            "custom" => Ok(Self::Custom),
            _ => Err(format!(
                "unknown prompt style '{value}'; expected classic, plain, or custom"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Plain => "plain",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShellConfig {
    pub(crate) profile: ShellProfile,
    pub(crate) program: ShellProgram,
    pub(crate) prompt: PromptStyle,
    pub(crate) prompt_file: Option<String>,
    pub(crate) alias_files: Vec<String>,
    pub(crate) bin_dirs: Vec<String>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            profile: ShellProfile::Familiar,
            program: ShellProgram::Bash,
            prompt: PromptStyle::Classic,
            prompt_file: None,
            alias_files: Vec::new(),
            bin_dirs: Vec::new(),
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
    pub(crate) shell: ShellConfig,
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
            shell: ShellConfig::default(),
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
    pub(crate) shell_profile: Option<ShellProfile>,
    pub(crate) shell_program: Option<ShellProgram>,
    pub(crate) prompt: Option<PromptStyle>,
    pub(crate) prompt_file: Option<String>,
    pub(crate) alias_files: Vec<String>,
    pub(crate) bin_dirs: Vec<String>,
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
            || self.shell_profile.is_some()
            || self.shell_program.is_some()
            || self.prompt.is_some()
            || self.prompt_file.is_some()
            || !self.alias_files.is_empty()
            || !self.bin_dirs.is_empty()
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
        warn_empty_custom_shell(&profile);
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
    let shell_profile = prompt_choice(
        "Host shell personalization",
        current.shell.profile.name(),
        &["familiar", "minimal", "custom"],
    )?;
    let shell_program = prompt_choice(
        "Login shell",
        current.shell.program.name(),
        &["bash", "zsh", "fish"],
    )?;
    let prompt_style = prompt_choice(
        "Prompt style",
        current.shell.prompt.name(),
        &["classic", "plain", "custom"],
    )?;
    let prompt_file = if prompt_style == "custom" {
        Some(prompt_home_path(
            "Custom prompt file relative to your home",
            current.shell.prompt_file.as_deref(),
        )?)
    } else {
        None
    };
    let (alias_files, bin_dirs) = if shell_profile == "custom" {
        (
            prompt_home_paths(
                "Alias files (comma-separated, relative to your home)",
                &current.shell.alias_files,
            )?,
            prompt_home_paths(
                "Executable directories (comma-separated, relative to your home)",
                &current.shell.bin_dirs,
            )?,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    let profile = Profile {
        workspace: WorkspaceMode::parse(&workspace)?,
        network: NetworkProfile::parse(&network)?,
        browser,
        orca,
        ghcr,
        paseo_relay,
        github_proxy: true,
        shell: ShellConfig {
            profile: ShellProfile::parse(&shell_profile)?,
            program: ShellProgram::parse(&shell_program)?,
            prompt: PromptStyle::parse(&prompt_style)?,
            prompt_file,
            alias_files,
            bin_dirs,
        },
        resources: Resources {
            memory,
            cpus,
            root_disk,
            docker_disk,
        },
    };
    validate(&profile, &context.state_root)?;
    warn_empty_custom_shell(&profile);
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
    if let Some(value) = overrides.shell_profile {
        profile.shell.profile = value;
        if value != ShellProfile::Custom {
            profile.shell.alias_files.clear();
            profile.shell.bin_dirs.clear();
        }
    }
    if let Some(value) = overrides.shell_program {
        profile.shell.program = value;
    }
    if let Some(value) = overrides.prompt {
        profile.shell.prompt = value;
        if value != PromptStyle::Custom && overrides.prompt_file.is_none() {
            profile.shell.prompt_file = None;
        }
    }
    if let Some(value) = &overrides.prompt_file {
        profile.shell.prompt = PromptStyle::Custom;
        profile.shell.prompt_file = Some(value.clone());
    }
    if !overrides.alias_files.is_empty() {
        profile.shell.profile = ShellProfile::Custom;
        profile.shell.alias_files = overrides.alias_files.clone();
    }
    if !overrides.bin_dirs.is_empty() {
        profile.shell.profile = ShellProfile::Custom;
        profile.shell.bin_dirs = overrides.bin_dirs.clone();
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
            "shell",
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
    let shell = match object.get("shell") {
        None => Map::new(),
        Some(Value::Object(value)) => value.clone(),
        Some(_) => return Err("shell must be an object".to_owned()),
    };
    reject_unknown(
        &shell,
        &[
            "profile",
            "program",
            "prompt",
            "promptFile",
            "aliasFiles",
            "binDirs",
        ],
        "shell",
    )?;
    let shell_profile = enum_value(
        &shell,
        "profile",
        ShellProfile::Familiar,
        ShellProfile::parse,
    )?;
    let shell_program = enum_value(&shell, "program", ShellProgram::Bash, ShellProgram::parse)?;
    let prompt = enum_value(&shell, "prompt", PromptStyle::Classic, PromptStyle::parse)?;
    let prompt_file = optional_home_path(&shell, "promptFile")?;
    let alias_files = home_path_array(&shell, "aliasFiles")?;
    let bin_dirs = home_path_array(&shell, "binDirs")?;

    Ok(Profile {
        workspace,
        network,
        browser: bool_value(&integrations, "browser")?,
        orca: bool_value(&integrations, "orca")?,
        ghcr: bool_value(&integrations, "ghcr")?,
        paseo_relay: bool_value(&integrations, "paseoRelay")?,
        github_proxy,
        shell: ShellConfig {
            profile: shell_profile,
            program: shell_program,
            prompt,
            prompt_file,
            alias_files,
            bin_dirs,
        },
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

fn enum_value<T: Copy>(
    object: &Map<String, Value>,
    key: &str,
    default: T,
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    match object.get(key) {
        None => Ok(default),
        Some(Value::String(value)) => parse(value),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn optional_home_path(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => normalize_home_path(value, key).map(Some),
        Some(_) => Err(format!("{key} must be a home-relative path or null")),
    }
}

fn home_path_array(object: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = object.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{key} must be an array of home-relative paths"))?;
    if array.len() > 32 {
        return Err(format!("{key} accepts at most 32 paths"));
    }
    let mut paths = Vec::with_capacity(array.len());
    for value in array {
        let path = value
            .as_str()
            .ok_or_else(|| format!("{key} entries must be strings"))?;
        let path = normalize_home_path(path, key)?;
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

pub(crate) fn normalize_home_path(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.starts_with('~') && !trimmed.starts_with("~/") {
        return Err(format!(
            "{label} must use ~/ for a path below the current user's home: {value}"
        ));
    }
    let candidate = Path::new(trimmed);
    let relative_path = if candidate.is_absolute() {
        let home = crate::process::home_dir()?;
        candidate
            .strip_prefix(&home)
            .map_err(|_| format!("{label} must stay below the host home directory: {value}"))?
    } else {
        Path::new(trimmed.strip_prefix("~/").unwrap_or(trimmed))
    };
    let traversal = || {
        format!(
            "{label} must stay below the host home directory and contain no traversal components: {value}"
        )
    };
    if trimmed.contains(['\n', '\r', '\0']) {
        return Err(traversal());
    }
    // Rebuild from the normal components so `bin/`, `bin/.` and `./bin`
    // all store as `bin`; later dedupe and path rewriting compare strings.
    let mut normalized = Vec::new();
    for component in relative_path.components() {
        match component {
            std::path::Component::Normal(part) => normalized.push(
                part.to_str()
                    .ok_or_else(|| format!("{label} must be valid UTF-8: {value}"))?,
            ),
            std::path::Component::CurDir => {}
            _ => return Err(traversal()),
        }
    }
    if normalized.is_empty() {
        return Err(traversal());
    }
    Ok(normalized.join("/"))
}

pub(crate) fn validate_shell_mirror_path(
    relative: &str,
    home: &Path,
    state_root: &Path,
) -> Result<(), String> {
    let selected = Path::new(relative);
    for protected in PROTECTED_HOST_STORES {
        if paths_overlap(selected, Path::new(protected)) {
            return Err(format!(
                "refusing host shell path '{relative}': it is inside, or would contain, protected credential store {protected}"
            ));
        }
    }
    let absolute_state = if state_root.is_absolute() {
        state_root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("could not resolve the protected state path: {error}"))?
            .join(state_root)
    };
    if let Ok(relative_state) = absolute_state.strip_prefix(home)
        && !relative_state.as_os_str().is_empty()
        && paths_overlap(selected, relative_state)
    {
        return Err(format!(
            "refusing host shell path '{relative}': it is inside, or would contain, the protected sbxr state directory"
        ));
    }
    Ok(())
}

/// Rejects a selection whose *resolved* location overlaps a protected store.
/// The lexical check alone misses symlinks: `~/.ssh -> ~/dotfiles/ssh` plus
/// `--bin-dir dotfiles`, or `~/link -> ~/.aws` plus `--alias-file link/x`.
pub(crate) fn validate_resolved_shell_path(
    relative: &str,
    home: &Path,
    state_root: &Path,
) -> Result<(), String> {
    let resolved = std::fs::canonicalize(home.join(relative)).map_err(|error| {
        format!("selected host shell path '{relative}' cannot be resolved: {error}")
    })?;
    let canonical_home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let inside = resolved.strip_prefix(&canonical_home).map_err(|_| {
        format!(
            "refusing host shell path '{relative}': it resolves outside the host home to {}",
            resolved.display()
        )
    })?;
    let inside = inside.to_str().ok_or_else(|| {
        format!("refusing host shell path '{relative}': its resolved path is not valid UTF-8")
    })?;
    if inside.is_empty() {
        return Err(format!(
            "refusing host shell path '{relative}': it resolves to the whole host home"
        ));
    }
    validate_shell_mirror_path(inside, home, state_root)?;
    let state = std::fs::canonicalize(state_root).ok();
    for protected in PROTECTED_HOST_STORES
        .iter()
        .filter_map(|protected| std::fs::canonicalize(home.join(protected)).ok())
        .chain(state)
    {
        if paths_overlap(&resolved, &protected) {
            return Err(format!(
                "refusing host shell path '{relative}': it resolves to {}, which overlaps the protected location {}",
                resolved.display(),
                protected.display()
            ));
        }
    }
    Ok(())
}

/// Compared case-insensitively: macOS home volumes usually are, so `.SSH`
/// names `.ssh` there. On Linux this only over-blocks a differently cased
/// sibling, which is the safe direction.
fn paths_overlap(selected: &Path, protected: &Path) -> bool {
    let fold = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    let (selected, protected) = (fold(selected), fold(protected));
    selected == protected || selected.starts_with(&protected) || protected.starts_with(&selected)
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
        "shell": {
            "profile": profile.shell.profile.name(),
            "program": profile.shell.program.name(),
            "prompt": profile.shell.prompt.name(),
            "promptFile": profile.shell.prompt_file,
            "aliasFiles": profile.shell.alias_files,
            "binDirs": profile.shell.bin_dirs,
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
        "shell": {
            "program": profile.shell.program.name(),
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
    if profile.shell.prompt == PromptStyle::Custom && profile.shell.prompt_file.is_none() {
        return Err("custom prompt style requires shell.promptFile or --prompt-file".to_owned());
    }
    if profile.shell.prompt != PromptStyle::Custom && profile.shell.prompt_file.is_some() {
        return Err("shell.promptFile is only valid with shell.prompt=custom".to_owned());
    }
    if profile.shell.profile != ShellProfile::Custom
        && (!profile.shell.alias_files.is_empty() || !profile.shell.bin_dirs.is_empty())
    {
        return Err(
            "shell.aliasFiles and shell.binDirs are only valid with shell.profile=custom"
                .to_owned(),
        );
    }
    for path in profile
        .shell
        .alias_files
        .iter()
        .chain(profile.shell.prompt_file.iter())
    {
        if matches!(
            path.as_str(),
            ".bashrc" | ".zshrc" | ".config/fish/config.fish"
        ) {
            return Err(format!(
                "{path} is a managed shell startup file; select a dedicated alias or prompt file instead"
            ));
        }
    }
    let home = crate::process::home_dir()?;
    for path in profile
        .shell
        .alias_files
        .iter()
        .chain(profile.shell.bin_dirs.iter())
        .chain(profile.shell.prompt_file.iter())
    {
        validate_shell_mirror_path(path, &home, state_root)?;
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

fn warn_empty_custom_shell(profile: &Profile) {
    if profile.shell.profile == ShellProfile::Custom
        && profile.shell.alias_files.is_empty()
        && profile.shell.bin_dirs.is_empty()
        && profile.shell.prompt != PromptStyle::Custom
    {
        println!(
            "WARN custom shell profile selects no host files; it behaves like minimal until paths are added"
        );
    }
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

fn prompt_home_path(label: &str, current: Option<&str>) -> Result<String, String> {
    loop {
        let value = prompt(label, current.unwrap_or("~/.config/sbxr/prompt"))?;
        match normalize_home_path(&value, label) {
            Ok(value) => return Ok(value),
            Err(error) => println!("{error}"),
        }
    }
}

fn prompt_home_paths(label: &str, current: &[String]) -> Result<Vec<String>, String> {
    loop {
        let default = if current.is_empty() {
            "none".to_owned()
        } else {
            current.join(",")
        };
        let value = prompt(label, &default)?;
        if matches!(value.as_str(), "none" | "-") {
            return Ok(Vec::new());
        }
        let mut paths = Vec::new();
        let mut error = None;
        for value in value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match normalize_home_path(value, label) {
                Ok(value) if !paths.contains(&value) => paths.push(value),
                Ok(_) => {}
                Err(message) => {
                    error = Some(message);
                    break;
                }
            }
        }
        if let Some(error) = error {
            println!("{error}");
        } else if paths.len() > 32 {
            println!("at most 32 paths are allowed");
        } else {
            return Ok(paths);
        }
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
              "shell": {
                "profile":"custom", "program":"zsh", "prompt":"custom",
                "promptFile":"~/.config/zsh/prompt.zsh",
                "aliasFiles":["~/.config/zsh/aliases.zsh"],
                "binDirs":["scripts"]
              },
              "resources": {"memory":"8G", "cpus":4, "rootDisk":"20g", "dockerDisk":"8g"}
            }"#,
            Path::new("profile.json"),
        )
        .unwrap();
        assert_eq!(profile.workspace, WorkspaceMode::Isolated);
        assert_eq!(profile.network, NetworkProfile::Restricted);
        assert_eq!(profile.resources.memory.as_deref(), Some("8g"));
        assert!(profile.browser && profile.orca && profile.ghcr && profile.paseo_relay);
        assert_eq!(profile.shell.profile, ShellProfile::Custom);
        assert_eq!(profile.shell.program, ShellProgram::Zsh);
        assert_eq!(profile.shell.prompt, PromptStyle::Custom);
        assert_eq!(
            profile.shell.prompt_file.as_deref(),
            Some(".config/zsh/prompt.zsh")
        );
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

    #[test]
    fn only_the_shell_program_changes_the_creation_contract() {
        let base = Profile::default();
        let mut overlay = base.clone();
        overlay.shell.profile = ShellProfile::Custom;
        overlay.shell.alias_files = vec![".config/my-aliases".to_owned()];
        overlay.shell.bin_dirs = vec!["scripts".to_owned()];
        assert_eq!(
            contract_bytes(&base).unwrap(),
            contract_bytes(&overlay).unwrap()
        );

        overlay.shell.program = ShellProgram::Fish;
        assert_ne!(
            contract_bytes(&base).unwrap(),
            contract_bytes(&overlay).unwrap()
        );
    }

    #[test]
    fn explicit_shell_paths_are_custom_only() {
        let mut profile = Profile::default();
        profile.shell.alias_files = vec![".config/my-aliases".to_owned()];
        assert!(validate(&profile, Path::new("/tmp/state")).is_err());

        apply_overrides(
            &mut profile,
            &Overrides {
                alias_files: vec![".config/my-aliases".to_owned()],
                bin_dirs: vec!["scripts".to_owned()],
                ..Overrides::default()
            },
        );
        assert_eq!(profile.shell.profile, ShellProfile::Custom);

        apply_overrides(
            &mut profile,
            &Overrides {
                shell_profile: Some(ShellProfile::Familiar),
                ..Overrides::default()
            },
        );
        assert!(profile.shell.alias_files.is_empty());
        assert!(profile.shell.bin_dirs.is_empty());
    }

    #[test]
    fn accepts_only_paths_below_the_host_home() {
        assert_eq!(
            normalize_home_path("~/.config/zsh/aliases", "test").unwrap(),
            ".config/zsh/aliases"
        );
        let absolute = crate::process::home_dir()
            .unwrap()
            .join(".config/zsh/aliases");
        assert_eq!(
            normalize_home_path(&absolute.to_string_lossy(), "test").unwrap(),
            ".config/zsh/aliases"
        );
        assert!(normalize_home_path("../outside", "test").is_err());
        assert!(normalize_home_path("/etc/profile", "test").is_err());
        assert!(normalize_home_path("~", "test").is_err());
        assert!(normalize_home_path("~other/file", "test").is_err());
        assert_eq!(normalize_home_path("bin/", "test").unwrap(), "bin");
        assert_eq!(normalize_home_path("./bin/./x/", "test").unwrap(), "bin/x");
        assert_eq!(normalize_home_path("~/bin//", "test").unwrap(), "bin");
    }

    #[test]
    fn startup_and_token_files_are_protected() {
        let home = crate::process::home_dir().unwrap();
        let state = std::env::temp_dir().join("sbxr-state-outside-home");
        for path in [
            ".profile",
            ".zshenv",
            ".env",
            ".config/fish/fish_variables",
            ".config/fish",
            ".m2/settings.xml",
            ".SSH",
        ] {
            assert!(
                validate_shell_mirror_path(path, &home, &state).is_err(),
                "{path} should be protected"
            );
        }
    }

    #[test]
    fn rejects_shell_paths_that_overlap_credential_stores() {
        let home = crate::process::home_dir().unwrap();
        let state = home.join("projects/protected-sbxr-state");
        for path in [
            ".ssh",
            ".ssh/config",
            ".config",
            ".config/gh/hosts.yml",
            ".local/share",
            ".codex/rules",
            "projects",
            "projects/protected-sbxr-state/profiles",
            ".npmrc",
            ".cargo",
            ".cargo/credentials.toml",
            ".mozilla/firefox",
            ".config/BraveSoftware/Brave-Browser/Default/Cookies",
            ".config/microsoft-edge/Default",
            "Library/Application Support/Google/Chrome/Default",
            "Library/Safari",
            "AppData/Local/Microsoft/Edge/User Data",
            "AppData/Roaming/Mozilla/Firefox",
            ".config/pypoetry/auth.toml",
            ".gem/credentials",
        ] {
            assert!(
                validate_shell_mirror_path(path, &home, &state).is_err(),
                "{path} should be protected"
            );
        }
        for path in ["bin", ".config/zsh/aliases.zsh", ".local/bin", ".cargo/bin"] {
            assert!(
                validate_shell_mirror_path(path, &home, &state).is_ok(),
                "{path} should be allowed"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolved_shell_paths_cannot_leave_the_host_home() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-profile-resolved-home-{}-{unique}",
            std::process::id()
        ));
        let home = root.join("home");
        let outside = root.join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("token"), b"must-not-copy").unwrap();
        symlink(&outside, home.join("bin")).unwrap();

        let error = validate_resolved_shell_path("bin", &home, &home.join("state"))
            .expect_err("a resolved shell path outside the home must be rejected");
        assert!(error.contains("outside the host home"), "{error}");
        fs::remove_dir_all(root).unwrap();
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
