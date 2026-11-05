use crate::profile::{NetworkProfile, Overrides, WorkspaceMode};
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Preset {
    MultiAgent,
    Codex,
    Claude,
}

impl Preset {
    fn parse(value: &OsStr) -> Result<Self, String> {
        match value.to_string_lossy().as_ref() {
            "rust-multi-agent" | "multi-agent" | "multi" => Ok(Self::MultiAgent),
            "rust-codex" | "codex" => Ok(Self::Codex),
            "rust-claude" | "claude" => Ok(Self::Claude),
            value => Err(format!(
                "unknown preset '{value}'; expected rust-multi-agent, rust-codex, or rust-claude"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::MultiAgent => "rust-multi-agent",
            Self::Codex => "rust-codex",
            Self::Claude => "rust-claude",
        }
    }

    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::MultiAgent => "multi",
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub(crate) fn primary_agent(self) -> &'static str {
        match self {
            Self::MultiAgent => "codex",
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    pub(crate) fn has_codex(self) -> bool {
        !matches!(self, Self::Claude)
    }

    pub(crate) fn has_claude(self) -> bool {
        !matches!(self, Self::Codex)
    }
}

pub(crate) struct Cli {
    pub(crate) preset: Preset,
    pub(crate) rocksdb: Option<PathBuf>,
    pub(crate) force: bool,
    pub(crate) profile: Overrides,
    pub(crate) action: Action,
}

impl Cli {
    pub(crate) fn parse(
        mut arguments: Vec<OsString>,
        environment: Option<OsString>,
    ) -> Result<Self, String> {
        let mut force = false;
        let mut preset = match environment {
            Some(value) => Preset::parse(&value)?,
            None => Preset::MultiAgent,
        };
        let mut rocksdb = env::var_os("SBXR_ROCKSDB_HOST").map(PathBuf::from);
        let mut profile = Overrides::default();
        let mut filtered = Vec::with_capacity(arguments.len());
        let mut index = 0;
        while index < arguments.len() {
            let argument = &arguments[index];
            let text = argument.to_string_lossy();
            let mut consume_next = |label: &str| -> Result<OsString, String> {
                index += 1;
                arguments
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("{label} requires a value"))
            };
            match text.as_ref() {
                "-y" | "--yes-to-all" | "--force" => {
                    if force {
                        return Err(
                            "-y, --yes-to-all, and --force are aliases; supply only one".to_owned()
                        );
                    }
                    force = true;
                }
                "--preset" => preset = Preset::parse(&consume_next("--preset")?)?,
                "--rocksdb-host" => rocksdb = Some(PathBuf::from("/opt/rocksdb-10.4.2")),
                "--wizard" | "--wizzard" => profile.wizard = true,
                "--config" => profile.config = Some(PathBuf::from(consume_next("--config")?)),
                "--memory" => {
                    let value = consume_next("--memory")?;
                    profile.memory = Some(crate::profile::normalize_size(
                        &value.to_string_lossy(),
                        "memory",
                    )?);
                }
                "--cpus" => {
                    let value = consume_next("--cpus")?;
                    profile.cpus = Some(parse_cpus(&value.to_string_lossy())?);
                }
                "--root-disk" => {
                    let value = consume_next("--root-disk")?;
                    profile.root_disk = Some(crate::profile::normalize_size(
                        &value.to_string_lossy(),
                        "root disk",
                    )?);
                }
                "--docker-disk" => {
                    let value = consume_next("--docker-disk")?;
                    profile.docker_disk = Some(crate::profile::normalize_size(
                        &value.to_string_lossy(),
                        "Docker disk",
                    )?);
                }
                "--network" => {
                    let value = consume_next("--network")?;
                    profile.network = Some(NetworkProfile::parse(&value.to_string_lossy())?);
                }
                "--isolated" => profile.workspace = Some(WorkspaceMode::Isolated),
                "--shared" => profile.workspace = Some(WorkspaceMode::Shared),
                "--browser" => profile.browser = Some(true),
                "--no-browser" => profile.browser = Some(false),
                "--orca" => profile.orca = Some(true),
                "--no-orca" => profile.orca = Some(false),
                "--ghcr" => profile.ghcr = Some(true),
                "--no-ghcr" => profile.ghcr = Some(false),
                "--paseo-relay" => profile.paseo_relay = Some(true),
                "--no-paseo-relay" => profile.paseo_relay = Some(false),
                _ if text.starts_with("--preset=") => {
                    preset = Preset::parse(OsStr::new(text.trim_start_matches("--preset=")))?;
                }
                _ if text.starts_with("--rocksdb-host=") => {
                    let value = text.trim_start_matches("--rocksdb-host=");
                    if value.is_empty() {
                        return Err("--rocksdb-host=PREFIX requires a non-empty prefix".to_owned());
                    }
                    rocksdb = Some(PathBuf::from(value));
                }
                _ if text.starts_with("--config=") => {
                    profile.config = Some(PathBuf::from(text.trim_start_matches("--config=")))
                }
                _ if text.starts_with("--memory=") => {
                    profile.memory = Some(crate::profile::normalize_size(
                        text.trim_start_matches("--memory="),
                        "memory",
                    )?)
                }
                _ if text.starts_with("--cpus=") => {
                    profile.cpus = Some(parse_cpus(text.trim_start_matches("--cpus="))?)
                }
                _ if text.starts_with("--root-disk=") => {
                    profile.root_disk = Some(crate::profile::normalize_size(
                        text.trim_start_matches("--root-disk="),
                        "root disk",
                    )?)
                }
                _ if text.starts_with("--docker-disk=") => {
                    profile.docker_disk = Some(crate::profile::normalize_size(
                        text.trim_start_matches("--docker-disk="),
                        "Docker disk",
                    )?)
                }
                _ if text.starts_with("--network=") => {
                    profile.network = Some(NetworkProfile::parse(
                        text.trim_start_matches("--network="),
                    )?)
                }
                _ => filtered.push(argument.clone()),
            }
            index += 1;
        }
        arguments = filtered;
        let mut action = Action::parse(arguments)?;
        if force && !action.accepts_force() {
            return Err("-y, --yes-to-all, and --force apply only to sandbox lifecycle commands or commit-pgp-sign".to_owned());
        }
        if let Action::Remove {
            force: remove_force,
            ..
        } = &mut action
        {
            *remove_force = force;
        }
        Ok(Self {
            preset,
            rocksdb,
            force,
            profile,
            action,
        })
    }
}

fn parse_cpus(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "--cpus requires a positive integer".to_owned())
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SignSelection {
    Head,
    AllMine,
    Range(String),
}

#[derive(Debug)]
pub(crate) struct SignRequest {
    pub(crate) path: Option<PathBuf>,
    pub(crate) selection: SignSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthProvider {
    OpenAi,
    Anthropic,
    GitHub,
}

impl AuthProvider {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "openai" | "codex" | "chatgpt" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "github" | "gh" => Ok(Self::GitHub),
            _ => Err(format!(
                "unknown authentication provider '{value}'; expected openai, anthropic, or github"
            )),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::GitHub => "github",
        }
    }
}

#[derive(Debug)]
pub(crate) struct VerifyRequest {
    pub(crate) path: Option<PathBuf>,
    pub(crate) live: bool,
}

#[derive(Debug)]
pub(crate) struct AuthRenewRequest {
    pub(crate) provider: AuthProvider,
    pub(crate) path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) enum Action {
    New(Option<PathBuf>),
    Code(Option<PathBuf>),
    Update(Option<PathBuf>),
    Up(Option<PathBuf>),
    Shell(Option<PathBuf>),
    Codex(Option<PathBuf>),
    Claude(Option<PathBuf>),
    Pi(Option<PathBuf>),
    Paseo(Option<PathBuf>),
    Orca(Option<PathBuf>),
    Isolated(Option<PathBuf>),
    Wizard(Option<PathBuf>),
    Profile(Option<PathBuf>),
    AuthImport(Option<PathBuf>),
    AuthStatus(Option<PathBuf>),
    AuthRenew(AuthRenewRequest),
    Audit(Option<PathBuf>),
    CommitPgpSign(SignRequest),
    Verify(VerifyRequest),
    Repair(Option<PathBuf>),
    Diagnose(Option<PathBuf>),
    Review(Option<PathBuf>),
    Inspect(Option<PathBuf>),
    Name(Option<PathBuf>),
    Status(Option<PathBuf>),
    Stop(Option<PathBuf>),
    Remove { path: Option<PathBuf>, force: bool },
    Setup,
    Doctor(Option<PathBuf>),
    Version,
    Help,
}

impl Action {
    fn parse(arguments: Vec<OsString>) -> Result<Self, String> {
        if arguments.is_empty() {
            return Ok(Self::New(None));
        }
        let command = arguments[0].to_string_lossy();
        if matches!(
            command.as_ref(),
            "setup" | "help" | "-h" | "--help" | "-V" | "--version"
        ) {
            if arguments.len() != 1 {
                return Err(format!("{command} does not accept arguments"));
            }
            return Ok(match command.as_ref() {
                "setup" => Self::Setup,
                "-V" | "--version" => Self::Version,
                _ => Self::Help,
            });
        }
        if command == "rm" {
            return Self::parse_remove(&arguments[1..]);
        }
        if command == "commit-pgp-sign" {
            return Self::parse_sign(&arguments[1..]);
        }
        if command == "auth" {
            return Self::parse_auth(&arguments[1..]);
        }
        if command == "verify" {
            return Self::parse_verify(&arguments[1..]);
        }
        if arguments.len() > 2 {
            return Err("expected at most one project path".to_owned());
        }
        let path = arguments.get(1).map(PathBuf::from);
        Ok(match command.as_ref() {
            "new" => Self::New(path),
            "code" | "vscode" => Self::Code(path),
            "update" => Self::Update(path),
            "up" => Self::Up(path),
            "shell" => Self::Shell(path),
            "codex" => Self::Codex(path),
            "claude" => Self::Claude(path),
            "pi" => Self::Pi(path),
            "paseo" => Self::Paseo(path),
            "orca" => Self::Orca(path),
            "isolated" => Self::Isolated(path),
            "wizard" | "wizzard" => Self::Wizard(path),
            "profile" => Self::Profile(path),
            "auth-import" => Self::AuthImport(path),
            "auth-status" => Self::AuthStatus(path),
            "audit" => Self::Audit(path),
            "repair" => Self::Repair(path),
            "diagnose" => Self::Diagnose(path),
            "review" => Self::Review(path),
            "inspect" => Self::Inspect(path),
            "name" => Self::Name(path),
            "status" => Self::Status(path),
            "stop" => Self::Stop(path),
            "doctor" => Self::Doctor(path),
            _ if arguments.len() == 1 => Self::New(Some(PathBuf::from(&arguments[0]))),
            _ => return Err(format!("unknown command: {command}")),
        })
    }

    fn parse_auth(arguments: &[OsString]) -> Result<Self, String> {
        if arguments.first().and_then(|value| value.to_str()) != Some("renew") {
            return Err("usage: sbxr auth renew openai|anthropic|github [PATH]".to_owned());
        }
        let provider = arguments
            .get(1)
            .and_then(|value| value.to_str())
            .ok_or("usage: sbxr auth renew openai|anthropic|github [PATH]")?;
        if arguments.len() > 3 {
            return Err("auth renew accepts one provider and at most one project path".to_owned());
        }
        Ok(Self::AuthRenew(AuthRenewRequest {
            provider: AuthProvider::parse(provider)?,
            path: arguments.get(2).map(PathBuf::from),
        }))
    }

    fn parse_verify(arguments: &[OsString]) -> Result<Self, String> {
        let mut path = None;
        let mut live = false;
        for argument in arguments {
            if argument == "--live" {
                if live {
                    return Err("verify accepts --live only once".to_owned());
                }
                live = true;
            } else if argument.to_string_lossy().starts_with('-') {
                return Err(format!(
                    "unknown verify option: {}",
                    argument.to_string_lossy()
                ));
            } else if path.is_some() {
                return Err("verify accepts at most one project path".to_owned());
            } else {
                path = Some(PathBuf::from(argument));
            }
        }
        Ok(Self::Verify(VerifyRequest { path, live }))
    }

    fn parse_sign(arguments: &[OsString]) -> Result<Self, String> {
        let mut path = None;
        let mut selection = SignSelection::Head;
        let mut index = 0;
        while index < arguments.len() {
            let argument = &arguments[index];
            let text = argument.to_string_lossy();
            if text == "--all-mine" {
                if selection != SignSelection::Head {
                    return Err("--all-mine and --range are mutually exclusive".to_owned());
                }
                selection = SignSelection::AllMine;
            } else if text == "--range" {
                if selection != SignSelection::Head {
                    return Err("--all-mine and --range are mutually exclusive".to_owned());
                }
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--range requires BASE..HEAD")?
                    .to_str()
                    .ok_or("--range must be valid UTF-8")?;
                if value.is_empty() {
                    return Err("--range requires BASE..HEAD".to_owned());
                }
                selection = SignSelection::Range(value.to_owned());
            } else if let Some(value) = text.strip_prefix("--range=") {
                if selection != SignSelection::Head {
                    return Err("--all-mine and --range are mutually exclusive".to_owned());
                }
                if value.is_empty() {
                    return Err("--range requires BASE..HEAD".to_owned());
                }
                selection = SignSelection::Range(value.to_owned());
            } else if text.starts_with('-') {
                return Err(format!("unknown commit-pgp-sign option: {text}"));
            } else if path.is_some() {
                return Err("commit-pgp-sign accepts at most one project path".to_owned());
            } else {
                path = Some(PathBuf::from(argument));
            }
            index += 1;
        }
        Ok(Self::CommitPgpSign(SignRequest { path, selection }))
    }

    fn parse_remove(arguments: &[OsString]) -> Result<Self, String> {
        let mut path = None;
        let mut force = false;
        for argument in arguments {
            if argument == "--force" {
                if force {
                    return Err("rm accepts --force only once".to_owned());
                }
                force = true;
            } else if argument.to_string_lossy().starts_with('-') {
                return Err(format!("unknown rm option: {}", argument.to_string_lossy()));
            } else if path.is_some() {
                return Err("rm accepts at most one project path".to_owned());
            } else {
                path = Some(PathBuf::from(argument));
            }
        }
        Ok(Self::Remove { path, force })
    }

    fn accepts_force(&self) -> bool {
        matches!(
            self,
            Self::New(_)
                | Self::Code(_)
                | Self::Update(_)
                | Self::Up(_)
                | Self::Shell(_)
                | Self::Codex(_)
                | Self::Claude(_)
                | Self::Pi(_)
                | Self::Paseo(_)
                | Self::Orca(_)
                | Self::Isolated(_)
                | Self::Wizard(_)
                | Self::AuthStatus(_)
                | Self::AuthRenew(_)
                | Self::Audit(_)
                | Self::Repair(_)
                | Self::CommitPgpSign(_)
                | Self::Review(_)
                | Self::Remove { .. }
        )
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        match self {
            Self::New(path)
            | Self::Code(path)
            | Self::Update(path)
            | Self::Up(path)
            | Self::Shell(path)
            | Self::Codex(path)
            | Self::Claude(path)
            | Self::Pi(path)
            | Self::Paseo(path)
            | Self::Orca(path)
            | Self::Isolated(path)
            | Self::Wizard(path)
            | Self::Profile(path)
            | Self::AuthImport(path)
            | Self::AuthStatus(path)
            | Self::Audit(path)
            | Self::Repair(path)
            | Self::Diagnose(path)
            | Self::Review(path)
            | Self::Inspect(path)
            | Self::Name(path)
            | Self::Status(path)
            | Self::Stop(path) => path.as_deref(),
            Self::Verify(request) => request.path.as_deref(),
            Self::AuthRenew(request) => request.path.as_deref(),
            Self::CommitPgpSign(request) => request.path.as_deref(),
            Self::Remove { path, .. } => path.as_deref(),
            Self::Doctor(path) => path.as_deref(),
            Self::Setup | Self::Version | Self::Help => None,
        }
    }

    pub(crate) fn required_host_commands(&self) -> &'static [&'static str] {
        match self {
            Self::New(_)
            | Self::Code(_)
            | Self::Update(_)
            | Self::Isolated(_)
            | Self::Wizard(_) => &["sbx", "code", "ssh"],
            Self::Review(_) => &["sbx", "code", "ssh", "git"],
            Self::CommitPgpSign(_) => &["git"],
            Self::AuthRenew(request) if request.provider == AuthProvider::GitHub => &["sbx", "gh"],
            Self::AuthRenew(_) => &["sbx"],
            Self::Up(_)
            | Self::Shell(_)
            | Self::Codex(_)
            | Self::Claude(_)
            | Self::Pi(_)
            | Self::Paseo(_)
            | Self::Orca(_)
            | Self::AuthImport(_)
            | Self::AuthStatus(_)
            | Self::Audit(_)
            | Self::Verify(_)
            | Self::Repair(_)
            | Self::Status(_)
            | Self::Stop(_)
            | Self::Remove { .. } => &["sbx"],
            Self::Inspect(_)
            | Self::Profile(_)
            | Self::Name(_)
            | Self::Setup
            | Self::Doctor(_)
            | Self::Diagnose(_)
            | Self::Version
            | Self::Help => &[],
        }
    }

    pub(crate) fn needs_vscode_remote_ssh(&self) -> bool {
        matches!(
            self,
            Self::New(_)
                | Self::Code(_)
                | Self::Update(_)
                | Self::Isolated(_)
                | Self::Wizard(_)
                | Self::Review(_)
        )
    }

    pub(crate) fn needs_kvm(&self) -> bool {
        matches!(
            self,
            Self::New(_)
                | Self::Code(_)
                | Self::Update(_)
                | Self::Up(_)
                | Self::Shell(_)
                | Self::Codex(_)
                | Self::Claude(_)
                | Self::Pi(_)
                | Self::Paseo(_)
                | Self::Orca(_)
                | Self::Isolated(_)
                | Self::Wizard(_)
                | Self::AuthImport(_)
                | Self::AuthStatus(_)
                | Self::AuthRenew(_)
                | Self::Audit(_)
                | Self::Verify(_)
                | Self::Repair(_)
                | Self::Review(_)
        )
    }
}

pub(crate) fn print_usage() {
    println!(
        r#"sbxr — project-scoped Rust development sandboxes (Rust implementation)

Usage:
  sbxr [OPTIONS] [-y|--yes-to-all|--force] [new] [PATH]
  sbxr [new] [PATH]        Create/reuse the sandbox and open VS Code
  sbxr vscode [PATH]       Explicit alias for `sbxr new`
  sbxr code [PATH]         Legacy alias for `sbxr new`
  sbxr update [PATH]       Re-mirror host integrations and open VS Code
  sbxr up [PATH]           Create/reuse without attaching
  sbxr shell [PATH]        Open a login shell in the sandbox
  sbxr codex [PATH]        Run Codex with a ChatGPT subscription
  sbxr claude [PATH]       Run Claude Code with a Claude subscription
  sbxr pi [PATH]           Run Pi with detected Codex/Claude subscriptions
  sbxr paseo [PATH]        Start the sandbox Paseo daemon and print its SSH URL
  sbxr orca [PATH]         Enable/start Orca Server and print its Desktop pairing URL
  sbxr isolated [PATH]     Create/reuse a long-lived mountless clone sandbox
  sbxr wizard [PATH]       Interactively choose and save project options, then open it
  sbxr profile [PATH]      Print the protected nonsecret project profile
  sbxr auth-import [PATH]  Explain the removal of sandbox credential imports
  sbxr auth-status [PATH]  Show subscription login status
  sbxr auth renew PROVIDER [PATH]
                           Renew openai, anthropic, or github through its proxy flow
  sbxr audit [PATH]        Run cargo-audit, cargo-deny, and cargo-vet
  sbxr commit-pgp-sign [PATH]
                           Amend HEAD on the host with an OpenPGP signature
  sbxr commit-pgp-sign --all-mine [PATH]
                           Sign your unsigned commits ahead of the upstream
  sbxr commit-pgp-sign --range BASE..HEAD [PATH]
                           Sign your unsigned commits in an explicit range
  sbxr verify [PATH]       Audit runtime identity, credentials, tools, and SSH agent
  sbxr verify --live [PATH]
                           Also make minimal Codex, Claude, and Pi model requests
  sbxr repair [PATH]       Reconcile missing/outdated integrations and verify them
  sbxr diagnose [PATH]     Summarize the newest redacted diagnostic report
  sbxr review [PATH]       Open a no-OAuth clone-mode review sandbox
  sbxr inspect [PATH]      Show the protected environment declaration
  sbxr name [PATH]         Print the deterministic sandbox name
  sbxr status [PATH]       Check whether the project sandbox exists
  sbxr stop [PATH]         Stop the project sandbox
  sbxr rm [--force] [PATH] Remove sandbox state; host files remain
  sbxr setup               Configure missing login, OAuth, and Remote-SSH
  sbxr doctor [PATH]       Check host prerequisites; with PATH compare guest versions
  sbxr --version           Print the installed sbxr version
  sbxr help

Options:
  --wizard, --wizzard      Ask for all optional settings; normal launches stay fast
  --config PATH            Load, validate, and save a nonsecret JSON profile
  --shared | --isolated    Shared host workspace (default) or mountless Git clone
  --network PROFILE        development (default), restricted, or offline
  --memory SIZE            VM memory, for example 8g
  --cpus N                 VM CPU count
  --root-disk SIZE         VM root disk size, for example 20g
  --docker-disk SIZE       Nested Docker disk size, for example 8g
  --browser | --no-browser Enable/disable the isolated Playwright MCP
  --orca | --no-orca       Enable/disable the Orca remote server
  --ghcr | --no-ghcr       Enable/disable host-proxied private GHCR credentials
  --paseo-relay | --no-paseo-relay
                           Enable/disable the authenticated remote Paseo relay
  --preset PRESET          rust-multi-agent, rust-codex, or rust-claude
  --rocksdb-host[=PREFIX]  Mount a trusted host RocksDB prefix read-only

PATH defaults to the current directory. PRESET is rust-multi-agent (default),
rust-codex, or rust-claude; SBXR_PRESET sets the same default. The generated
name is preset-specific unless SBXR_NAME overrides it. SBXR_KIT_ROOT
overrides the vendored kit directory. SBXR_STATE_ROOT overrides the protected
host-side environment-definition directory. Set SBXR_SYNC_AGENT_CONFIG=0 or
SBXR_SYNC_VSCODE_EXTENSIONS=0 to disable the corresponding host mirror.
-y, --yes-to-all, and --force are equivalent. They pass --force to Docker's
environment command, skip its plan-approval prompt, and approve a planned
commit-pgp-sign history rewrite. Hardware signing prompts are never skipped.
Flags may appear before or after the command. Use `sbxr up -y` for creation
without opening VS Code.
GitHub HTTPS/API authentication is host-proxied by default. SSH-agent forwarding
remains enabled for Git SSH access. Profiles contain no credentials and are saved
under the protected SBXR_STATE_ROOT. Optional integrations are off by default.
--rocksdb-host is opt-in and mounts /opt/rocksdb-10.4.2 read-only; use
--rocksdb-host=PREFIX or SBXR_ROCKSDB_HOST for another installation."#
    );
}

pub(crate) fn print_version() {
    println!("sbxr {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_bare_paths() {
        assert!(matches!(Action::parse(vec![]).unwrap(), Action::New(None)));
        assert!(matches!(
            Action::parse(vec!["up".into(), "/tmp/project".into()]).unwrap(),
            Action::Up(Some(_))
        ));
        assert!(matches!(
            Action::parse(vec!["vscode".into(), "/tmp/project".into()]).unwrap(),
            Action::Code(Some(_))
        ));
        assert!(matches!(
            Action::parse(vec!["update".into(), "/tmp/project".into()]).unwrap(),
            Action::Update(Some(_))
        ));
        assert!(matches!(
            Action::parse(vec!["status".into()]).unwrap(),
            Action::Status(None)
        ));
        assert!(matches!(
            Action::parse(vec!["inspect".into(), "/tmp/project".into()]).unwrap(),
            Action::Inspect(Some(_))
        ));
        assert!(matches!(
            Action::parse(vec!["verify".into(), "/tmp/project".into()]).unwrap(),
            Action::Verify(VerifyRequest {
                path: Some(_),
                live: false
            })
        ));
        assert!(matches!(
            Action::parse(vec!["verify".into(), "--live".into()]).unwrap(),
            Action::Verify(VerifyRequest {
                path: None,
                live: true
            })
        ));
        assert!(matches!(
            Action::parse(vec![
                "auth".into(),
                "renew".into(),
                "anthropic".into(),
                "/tmp/project".into()
            ])
            .unwrap(),
            Action::AuthRenew(AuthRenewRequest {
                provider: AuthProvider::Anthropic,
                path: Some(_)
            })
        ));
        assert!(matches!(
            Action::parse(vec!["commit-pgp-sign".into()]).unwrap(),
            Action::CommitPgpSign(SignRequest {
                path: None,
                selection: SignSelection::Head
            })
        ));
        assert!(matches!(
            Action::parse(vec![
                "commit-pgp-sign".into(),
                "--all-mine".into(),
                "/tmp/project".into()
            ])
            .unwrap(),
            Action::CommitPgpSign(SignRequest {
                path: Some(_),
                selection: SignSelection::AllMine
            })
        ));
        assert!(matches!(
            Action::parse(vec![
                "commit-pgp-sign".into(),
                "--range=origin/main..HEAD".into()
            ])
            .unwrap(),
            Action::CommitPgpSign(SignRequest {
                selection: SignSelection::Range(_),
                ..
            })
        ));
        assert!(
            Action::parse(vec![
                "commit-pgp-sign".into(),
                "--all-mine".into(),
                "--range".into(),
                "HEAD~2..HEAD".into()
            ])
            .is_err()
        );
        assert!(matches!(
            Action::parse(vec!["rm".into(), "--force".into()]).unwrap(),
            Action::Remove {
                path: None,
                force: true
            }
        ));
        assert!(matches!(
            Action::parse(vec!["rm".into(), "/tmp/project".into(), "--force".into()]).unwrap(),
            Action::Remove {
                path: Some(_),
                force: true
            }
        ));
        assert!(Action::parse(vec!["rm".into(), "--unknown".into()]).is_err());
        assert!(matches!(
            Action::parse(vec![".".into()]).unwrap(),
            Action::New(Some(_))
        ));
        assert!(matches!(
            Action::parse(vec!["setup".into()]).unwrap(),
            Action::Setup
        ));
        assert!(Action::parse(vec!["setup".into(), ".".into()]).is_err());
        assert!(matches!(
            Action::parse(vec!["--version".into()]).unwrap(),
            Action::Version
        ));
        assert!(matches!(
            Action::parse(vec!["-V".into()]).unwrap(),
            Action::Version
        ));
        assert!(Action::parse(vec!["--version".into(), ".".into()]).is_err());
    }

    #[test]
    fn selects_action_specific_preflight_dependencies() {
        let new = Action::New(None);
        assert_eq!(new.required_host_commands(), ["sbx", "code", "ssh"]);
        assert!(new.needs_vscode_remote_ssh());

        let update = Action::Update(None);
        assert_eq!(update.required_host_commands(), ["sbx", "code", "ssh"]);
        assert!(update.needs_vscode_remote_ssh());
        assert!(update.needs_kvm());

        let review = Action::Review(None);
        assert_eq!(
            review.required_host_commands(),
            ["sbx", "code", "ssh", "git"]
        );
        assert!(review.needs_vscode_remote_ssh());

        let sign = Action::CommitPgpSign(SignRequest {
            path: None,
            selection: SignSelection::Head,
        });
        assert_eq!(sign.required_host_commands(), ["git"]);
        assert!(!sign.needs_vscode_remote_ssh());
        assert!(!sign.needs_kvm());

        let verify = Action::Verify(VerifyRequest {
            path: None,
            live: false,
        });
        assert_eq!(verify.required_host_commands(), ["sbx"]);
        assert!(!verify.needs_vscode_remote_ssh());
        assert!(verify.needs_kvm());

        let github_renew = Action::AuthRenew(AuthRenewRequest {
            provider: AuthProvider::GitHub,
            path: None,
        });
        assert_eq!(github_renew.required_host_commands(), ["sbx", "gh"]);

        let openai_renew = Action::AuthRenew(AuthRenewRequest {
            provider: AuthProvider::OpenAi,
            path: None,
        });
        assert_eq!(openai_renew.required_host_commands(), ["sbx"]);

        let shell = Action::Shell(None);
        assert_eq!(shell.required_host_commands(), ["sbx"]);
        assert!(!shell.needs_vscode_remote_ssh());
        assert!(shell.needs_kvm());

        let remove = Action::Remove {
            path: None,
            force: false,
        };
        assert_eq!(remove.required_host_commands(), ["sbx"]);
        assert!(!remove.needs_kvm());

        let name = Action::Name(None);
        assert!(name.required_host_commands().is_empty());
        assert!(!name.needs_kvm());

        let inspect = Action::Inspect(None);
        assert!(inspect.required_host_commands().is_empty());
        assert!(!inspect.needs_kvm());

        let version = Action::Version;
        assert!(version.required_host_commands().is_empty());
        assert!(!version.needs_kvm());

        let status = Action::Status(None);
        assert_eq!(status.required_host_commands(), ["sbx"]);
        assert!(!status.needs_kvm());
    }

    #[test]
    fn defaults_to_multi_agent_and_parses_presets() {
        let default = Cli::parse(vec![], None).unwrap();
        assert_eq!(default.preset, Preset::MultiAgent);
        assert!(!default.force);
        let codex = Cli::parse(
            vec!["--preset".into(), "rust-codex".into(), "up".into()],
            None,
        )
        .unwrap();
        assert_eq!(codex.preset, Preset::Codex);
        assert!(matches!(codex.action, Action::Up(None)));
        let claude = Cli::parse(vec![], Some("claude".into())).unwrap();
        assert_eq!(claude.preset, Preset::Claude);
        let rocksdb = Cli::parse(
            vec!["--rocksdb-host".into(), "name".into(), ".".into()],
            None,
        )
        .unwrap();
        assert_eq!(rocksdb.rocksdb, Some(PathBuf::from("/opt/rocksdb-10.4.2")));
    }

    #[test]
    fn parses_force_aliases_before_or_after_creation_commands() {
        for arguments in [
            vec!["-y".into(), "new".into(), "/tmp/project".into()],
            vec!["new".into(), "--yes-to-all".into(), "/tmp/project".into()],
            vec!["new".into(), "/tmp/project".into(), "--force".into()],
        ] {
            let parsed = Cli::parse(arguments, None).unwrap();
            assert!(parsed.force);
            assert!(matches!(parsed.action, Action::New(Some(_))));
        }

        let remove =
            Cli::parse(vec!["rm".into(), "-y".into(), "/tmp/project".into()], None).unwrap();
        assert!(matches!(
            remove.action,
            Action::Remove {
                path: Some(_),
                force: true
            }
        ));
        let sign = Cli::parse(
            vec![
                "commit-pgp-sign".into(),
                "--all-mine".into(),
                "-y".into(),
                "/tmp/project".into(),
            ],
            None,
        )
        .unwrap();
        assert!(sign.force);
        assert!(matches!(sign.action, Action::CommitPgpSign(_)));
        assert!(Cli::parse(vec!["status".into(), "-y".into()], None).is_err());
        assert!(Cli::parse(vec!["new".into(), "-y".into(), "--force".into()], None).is_err());
    }

    #[test]
    fn parses_profile_options_anywhere_and_wizard_alias() {
        let parsed = Cli::parse(
            vec![
                "new".into(),
                "--memory=8G".into(),
                "/tmp/project".into(),
                "--cpus".into(),
                "4".into(),
                "--network=restricted".into(),
                "--browser".into(),
                "--orca".into(),
                "--ghcr".into(),
                "--isolated".into(),
                "--wizzard".into(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(parsed.profile.memory.as_deref(), Some("8g"));
        assert_eq!(parsed.profile.cpus, Some(4));
        assert_eq!(parsed.profile.network, Some(NetworkProfile::Restricted));
        assert_eq!(parsed.profile.workspace, Some(WorkspaceMode::Isolated));
        assert_eq!(parsed.profile.browser, Some(true));
        assert_eq!(parsed.profile.orca, Some(true));
        assert_eq!(parsed.profile.ghcr, Some(true));
        assert!(parsed.profile.wizard);
        assert!(matches!(parsed.action, Action::New(Some(_))));
    }
}
