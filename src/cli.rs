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
            Self::MultiAgent | Self::Codex => "codex",
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
    pub(crate) action: Action,
}

impl Cli {
    pub(crate) fn parse(
        mut arguments: Vec<OsString>,
        environment: Option<OsString>,
    ) -> Result<Self, String> {
        let mut preset = match environment {
            Some(value) => Preset::parse(&value)?,
            None => Preset::MultiAgent,
        };
        let mut rocksdb = env::var_os("SBXR_ROCKSDB_HOST").map(PathBuf::from);
        loop {
            if arguments.first().is_some_and(|value| value == "--preset") {
                if arguments.len() < 2 {
                    return Err("--preset requires a value".to_owned());
                }
                preset = Preset::parse(&arguments[1])?;
                arguments.drain(0..2);
            } else if let Some(value) = arguments
                .first()
                .and_then(|value| value.to_str())
                .and_then(|value| value.strip_prefix("--preset="))
                .map(OsString::from)
            {
                preset = Preset::parse(&value)?;
                arguments.remove(0);
            } else if arguments
                .first()
                .is_some_and(|value| value == "--rocksdb-host")
            {
                rocksdb = Some(PathBuf::from("/opt/rocksdb-10.4.2"));
                arguments.remove(0);
            } else if let Some(value) = arguments
                .first()
                .and_then(|value| value.to_str())
                .and_then(|value| value.strip_prefix("--rocksdb-host="))
            {
                if value.is_empty() {
                    return Err("--rocksdb-host=PREFIX requires a non-empty prefix".to_owned());
                }
                rocksdb = Some(PathBuf::from(value));
                arguments.remove(0);
            } else {
                break;
            }
        }
        Ok(Self {
            preset,
            rocksdb,
            action: Action::parse(arguments)?,
        })
    }
}

#[derive(Debug)]
pub(crate) enum Action {
    New(Option<PathBuf>),
    Code(Option<PathBuf>),
    Up(Option<PathBuf>),
    Shell(Option<PathBuf>),
    Codex(Option<PathBuf>),
    Claude(Option<PathBuf>),
    Pi(Option<PathBuf>),
    AuthImport(Option<PathBuf>),
    AuthStatus(Option<PathBuf>),
    Audit(Option<PathBuf>),
    Review(Option<PathBuf>),
    Name(Option<PathBuf>),
    Status(Option<PathBuf>),
    Stop(Option<PathBuf>),
    Remove { path: Option<PathBuf>, force: bool },
    Setup,
    Doctor,
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
            "setup" | "doctor" | "help" | "-h" | "--help"
        ) {
            if arguments.len() != 1 {
                return Err(format!("{command} does not accept arguments"));
            }
            return Ok(match command.as_ref() {
                "setup" => Self::Setup,
                "doctor" => Self::Doctor,
                _ => Self::Help,
            });
        }
        if command == "rm" {
            return Self::parse_remove(&arguments[1..]);
        }
        if arguments.len() > 2 {
            return Err("expected at most one project path".to_owned());
        }
        let path = arguments.get(1).map(PathBuf::from);
        Ok(match command.as_ref() {
            "new" => Self::New(path),
            "code" | "vscode" => Self::Code(path),
            "up" => Self::Up(path),
            "shell" => Self::Shell(path),
            "codex" => Self::Codex(path),
            "claude" => Self::Claude(path),
            "pi" => Self::Pi(path),
            "auth-import" => Self::AuthImport(path),
            "auth-status" => Self::AuthStatus(path),
            "audit" => Self::Audit(path),
            "review" => Self::Review(path),
            "name" => Self::Name(path),
            "status" => Self::Status(path),
            "stop" => Self::Stop(path),
            _ if arguments.len() == 1 => Self::New(Some(PathBuf::from(&arguments[0]))),
            _ => return Err(format!("unknown command: {command}")),
        })
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

    pub(crate) fn path(&self) -> Option<&Path> {
        match self {
            Self::New(path)
            | Self::Code(path)
            | Self::Up(path)
            | Self::Shell(path)
            | Self::Codex(path)
            | Self::Claude(path)
            | Self::Pi(path)
            | Self::AuthImport(path)
            | Self::AuthStatus(path)
            | Self::Audit(path)
            | Self::Review(path)
            | Self::Name(path)
            | Self::Status(path)
            | Self::Stop(path) => path.as_deref(),
            Self::Remove { path, .. } => path.as_deref(),
            Self::Setup | Self::Doctor | Self::Help => None,
        }
    }

    pub(crate) fn required_host_commands(&self) -> &'static [&'static str] {
        match self {
            Self::New(_) | Self::Code(_) => &["sbx", "code", "ssh"],
            Self::Review(_) => &["sbx", "code", "ssh", "git"],
            Self::Up(_)
            | Self::Shell(_)
            | Self::Codex(_)
            | Self::Claude(_)
            | Self::Pi(_)
            | Self::AuthImport(_)
            | Self::AuthStatus(_)
            | Self::Audit(_)
            | Self::Status(_)
            | Self::Stop(_)
            | Self::Remove { .. } => &["sbx"],
            Self::Name(_) | Self::Setup | Self::Doctor | Self::Help => &[],
        }
    }

    pub(crate) fn needs_vscode_remote_ssh(&self) -> bool {
        matches!(self, Self::New(_) | Self::Code(_) | Self::Review(_))
    }

    pub(crate) fn needs_kvm(&self) -> bool {
        matches!(
            self,
            Self::New(_)
                | Self::Code(_)
                | Self::Up(_)
                | Self::Shell(_)
                | Self::Codex(_)
                | Self::Claude(_)
                | Self::Pi(_)
                | Self::AuthImport(_)
                | Self::AuthStatus(_)
                | Self::Audit(_)
                | Self::Review(_)
        )
    }
}

pub(crate) fn print_usage() {
    println!(
        r#"sbxr — project-scoped Rust development sandboxes (Rust implementation)

Usage:
  sbxr [--preset PRESET] [--rocksdb-host[=PREFIX]] [new] [PATH]
  sbxr [new] [PATH]        Create/reuse the sandbox and open VS Code
  sbxr vscode [PATH]       Explicit alias for `sbxr new`
  sbxr code [PATH]         Legacy alias for `sbxr new`
  sbxr up [PATH]           Create/reuse without attaching
  sbxr shell [PATH]        Open a login shell in the sandbox
  sbxr codex [PATH]        Run Codex with a ChatGPT subscription
  sbxr claude [PATH]       Run Claude Code with a Claude subscription
  sbxr pi [PATH]           Run Pi with detected Codex/Claude subscriptions
  sbxr auth-import [PATH]  Explicitly import trusted host OAuth caches
  sbxr auth-status [PATH]  Show subscription login status
  sbxr audit [PATH]        Run cargo-audit, cargo-deny, and cargo-vet
  sbxr review [PATH]       Open a no-OAuth clone-mode review sandbox
  sbxr name [PATH]         Print the deterministic sandbox name
  sbxr status [PATH]       Check whether the project sandbox exists
  sbxr stop [PATH]         Stop the project sandbox
  sbxr rm [--force] [PATH] Remove sandbox state; host files remain
  sbxr setup               Configure missing login, OAuth, and Remote-SSH
  sbxr doctor              Check host prerequisites and vendored kits
  sbxr help

PATH defaults to the current directory. PRESET is rust-multi-agent (default),
rust-codex, or rust-claude; SBXR_PRESET sets the same default. The generated
name is preset-specific unless SBXR_NAME overrides it. SBXR_KIT_ROOT
overrides the vendored kit directory. Set SBXR_SYNC_AGENT_CONFIG=0 or
SBXR_SYNC_VSCODE_EXTENSIONS=0 to disable the corresponding host mirror.
--rocksdb-host is opt-in and mounts /opt/rocksdb-10.4.2 read-only; use
--rocksdb-host=PREFIX or SBXR_ROCKSDB_HOST for another installation."#
    );
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
            Action::parse(vec!["status".into()]).unwrap(),
            Action::Status(None)
        ));
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
    }

    #[test]
    fn selects_action_specific_preflight_dependencies() {
        let new = Action::New(None);
        assert_eq!(new.required_host_commands(), ["sbx", "code", "ssh"]);
        assert!(new.needs_vscode_remote_ssh());

        let review = Action::Review(None);
        assert_eq!(
            review.required_host_commands(),
            ["sbx", "code", "ssh", "git"]
        );
        assert!(review.needs_vscode_remote_ssh());

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

        let status = Action::Status(None);
        assert_eq!(status.required_host_commands(), ["sbx"]);
        assert!(!status.needs_kvm());
    }

    #[test]
    fn defaults_to_multi_agent_and_parses_presets() {
        let default = Cli::parse(vec![], None).unwrap();
        assert_eq!(default.preset, Preset::MultiAgent);
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
}
