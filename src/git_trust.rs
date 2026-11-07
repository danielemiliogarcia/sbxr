use crate::process;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ConfigSource {
    Common,
    GitDir,
    Worktree,
}

impl ConfigSource {
    fn label(self) -> &'static str {
        match self {
            Self::Common => "local",
            Self::GitDir => "git-dir",
            Self::Worktree => "worktree",
        }
    }

    fn unset_scope(self) -> &'static str {
        match self {
            Self::Common | Self::GitDir => "--local",
            Self::Worktree => "--worktree",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FindingKind {
    Dangerous,
    Unknown,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Finding {
    kind: FindingKind,
    source: ConfigSource,
    key: String,
    reason: &'static str,
}

#[derive(Clone, Debug)]
pub(crate) struct RepositoryLayout {
    pub(crate) root: PathBuf,
    pub(crate) git_dir: PathBuf,
    pub(crate) common_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct Audit {
    pub(crate) layout: RepositoryLayout,
    findings: Vec<Finding>,
    hooks: Vec<String>,
}

impl Audit {
    pub(crate) fn has_attention(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.kind == FindingKind::Dangerous)
            || !self.hooks.is_empty()
    }

    pub(crate) fn has_unknown(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.kind == FindingKind::Unknown)
    }

    pub(crate) fn health_detail(&self) -> &'static str {
        if self.has_attention() {
            "repository config or hooks can affect plain host Git"
        } else if self.has_unknown() {
            "no known executable settings; unrecognized keys block sbxr signing"
        } else {
            "no executable repository config or hooks"
        }
    }

    pub(crate) fn signing_allowed(&self) -> bool {
        self.findings.is_empty()
    }

    pub(crate) fn print(&self) {
        for finding in &self.findings {
            let label = match finding.kind {
                FindingKind::Dangerous => "WARN",
                FindingKind::Unknown => "info",
            };
            println!(
                "{label} host Git {} config key {}: {}",
                finding.source.label(),
                finding.key,
                finding.reason
            );
            println!(
                "     remove: git -C {} config {} --unset-all {}",
                shell_quote(&self.layout.root),
                finding.source.unset_scope(),
                shell_quote_str(&finding.key)
            );
        }
        for hook in &self.hooks {
            println!("WARN executable repository hook {hook}; plain host Git may execute it");
            println!(
                "     inspect or remove its executable bit: {}",
                shell_quote(&self.layout.common_dir.join("hooks").join(hook))
            );
        }
        if self.findings.is_empty() && self.hooks.is_empty() {
            println!("ok   host Git repository config has no executable local settings or hooks");
        }
    }

    pub(crate) fn rejection(&self) -> String {
        let mut causes = Vec::new();
        if !self.findings.is_empty() {
            causes.push(format!(
                "config keys: {}",
                self.findings
                    .iter()
                    .map(|finding| finding.key.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.hooks.is_empty() {
            causes.push(format!("executable hooks: {}", self.hooks.join(", ")));
        }
        format!(
            "refusing host signing because repository-controlled Git inputs are not allowlisted: {}\nRun `sbxr status {}` for safe inspection and removal guidance. No host commit was changed.",
            causes.join("; "),
            shell_quote(&self.layout.root)
        )
    }
}

pub(crate) fn repository_root(requested: &Path) -> Result<PathBuf, String> {
    discover_layout(requested)?
        .map(|layout| layout.root)
        .ok_or_else(|| format!("no Git repository contains {}", requested.display()))
}

pub(crate) fn audit_with_trusted_host(requested: &Path) -> Result<Option<Audit>, String> {
    if discover_layout(requested)?.is_none() {
        return Ok(None);
    }
    let git_program = trusted_program("git", None, requested)?;
    let neutral = NeutralDirectory::create()?;
    let signing_key = trusted_config_value(&git_program, neutral.path(), "user.signingKey")?;
    audit(requested, signing_key.as_deref())
}

#[derive(Clone, Debug)]
pub(crate) struct HostConfig {
    git_program: PathBuf,
    gpg_program: PathBuf,
    pub(crate) user_name: String,
    pub(crate) user_email: String,
    pub(crate) signing_key: String,
}

impl HostConfig {
    pub(crate) fn load_for_signing(repository: &Path) -> Result<Self, String> {
        let git_program = trusted_program("git", None, repository)?;
        let neutral = NeutralDirectory::create()?;
        let user_name = trusted_config_value(&git_program, neutral.path(), "user.name")?
            .ok_or("host OpenPGP signing requires user.name in global or system Git config")?;
        let user_email = trusted_config_value(&git_program, neutral.path(), "user.email")?
            .ok_or("host OpenPGP signing requires user.email in global or system Git config")?;
        let signing_key = trusted_config_value(&git_program, neutral.path(), "user.signingKey")?
            .ok_or(
                "host OpenPGP signing requires user.signingKey in global or system Git config",
            )?;
        for (name, value) in [
            ("user.name", &user_name),
            ("user.email", &user_email),
            ("user.signingKey", &signing_key),
        ] {
            if value.is_empty() || value.contains(['\0', '\r', '\n']) {
                return Err(format!(
                    "trusted host Git {name} is empty or contains a newline"
                ));
            }
        }
        let configured =
            trusted_config_value(&git_program, neutral.path(), "gpg.openpgp.program")?.or(
                trusted_config_value(&git_program, neutral.path(), "gpg.program")?,
            );
        let gpg_program = trusted_program("gpg", configured.as_deref(), repository)?;
        Ok(Self {
            git_program,
            gpg_program,
            user_name,
            user_email,
            signing_key,
        })
    }

    pub(crate) fn command(&self, repository: &Path) -> Command {
        let mut command = Command::new(&self.git_program);
        scrub_git_environment(&mut command);
        command
            .arg("-C")
            .arg(repository)
            .args(["-c", null_hooks_config()])
            .args(["-c", "core.fsmonitor=false"])
            .args(["-c", "core.pager="])
            .args(["-c", "gpg.format=openpgp"])
            .arg("-c")
            .arg(format!("gpg.program={}", self.gpg_program.display()))
            .arg("-c")
            .arg(format!(
                "gpg.openpgp.program={}",
                self.gpg_program.display()
            ))
            .arg("-c")
            .arg(format!("user.name={}", self.user_name))
            .arg("-c")
            .arg(format!("user.email={}", self.user_email))
            .arg("-c")
            .arg(format!("user.signingKey={}", self.signing_key))
            .args(["-c", "commit.gpgSign=false"])
            .env("GIT_PAGER", "")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", trusted_noop_program())
            .env("GIT_SEQUENCE_EDITOR", trusted_noop_program())
            .env("GIT_ASKPASS", trusted_false_program())
            .env("GIT_NO_REPLACE_OBJECTS", "1");
        command
    }

    pub(crate) fn ensure_programs_outside(&self, repository: &Path) -> Result<(), String> {
        for (name, path) in [("Git", &self.git_program), ("GPG", &self.gpg_program)] {
            if path.starts_with(repository) {
                return Err(format!(
                    "refusing trusted {name} program inside the shared repository: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn audit(
    requested: &Path,
    trusted_signing_key: Option<&str>,
) -> Result<Option<Audit>, String> {
    let Some(layout) = discover_layout(requested)? else {
        return Ok(None);
    };
    let git_program = trusted_program("git", None, &layout.root)?;
    let mut entries = Vec::new();
    let common_config = layout.common_dir.join("config");
    read_config(
        &git_program,
        &common_config,
        ConfigSource::Common,
        &mut entries,
    )?;
    let git_config = layout.git_dir.join("config");
    if git_config != common_config {
        read_config(
            &git_program,
            &git_config,
            ConfigSource::GitDir,
            &mut entries,
        )?;
    }
    read_config(
        &git_program,
        &layout.git_dir.join("config.worktree"),
        ConfigSource::Worktree,
        &mut entries,
    )?;

    let mut findings = entries
        .into_iter()
        .filter_map(|(source, key, value)| {
            classify(&key, &value, trusted_signing_key).map(|(kind, reason)| Finding {
                kind,
                source,
                key,
                reason,
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    findings.sort();
    let hooks = executable_hooks(&layout.common_dir)?;
    Ok(Some(Audit {
        layout,
        findings,
        hooks,
    }))
}

fn discover_layout(requested: &Path) -> Result<Option<RepositoryLayout>, String> {
    let requested = requested.canonicalize().map_err(|error| {
        format!(
            "project path {} is unavailable for Git trust inspection: {error}",
            requested.display()
        )
    })?;
    let mut cursor = requested.as_path();
    let (root, dot_git, metadata) = loop {
        let candidate = cursor.join(".git");
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing symlinked repository metadata path: {}",
                    candidate.display()
                ));
            }
            Ok(metadata) if metadata.is_dir() || metadata.is_file() => {
                break (cursor.to_owned(), candidate, metadata);
            }
            Ok(_) => {
                return Err(format!(
                    "repository metadata is not a regular file or directory: {}",
                    candidate.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect repository metadata {}: {error}",
                    candidate.display()
                ));
            }
        }
        let Some(parent) = cursor.parent() else {
            return Ok(None);
        };
        cursor = parent;
    };

    let git_dir = if metadata.is_dir() {
        dot_git.clone()
    } else {
        let contents = fs::read_to_string(&dot_git)
            .map_err(|error| format!("could not read {}: {error}", dot_git.display()))?;
        let target = contents
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("gitdir: "))
            .ok_or_else(|| format!("invalid Git metadata file: {}", dot_git.display()))?;
        let target = PathBuf::from(target);
        let target = if target.is_absolute() {
            target
        } else {
            root.join(target)
        };
        canonical_real_directory(&target, "Git directory")?
    };
    let git_dir = canonical_real_directory(&git_dir, "Git directory")?;

    let common_dir_file = git_dir.join("commondir");
    let common_dir = match regular_file_state(&common_dir_file)? {
        FileState::Missing => git_dir.clone(),
        FileState::Regular => {
            let value = fs::read_to_string(&common_dir_file).map_err(|error| {
                format!("could not read {}: {error}", common_dir_file.display())
            })?;
            let value = PathBuf::from(value.trim());
            let value = if value.is_absolute() {
                value
            } else {
                git_dir.join(value)
            };
            canonical_real_directory(&value, "common Git directory")?
        }
    };

    if !git_dir.starts_with(&root) || !common_dir.starts_with(&root) {
        validate_linked_worktree(&dot_git, &git_dir, &common_dir)?;
    }
    Ok(Some(RepositoryLayout {
        root,
        git_dir,
        common_dir,
    }))
}

fn validate_linked_worktree(
    dot_git: &Path,
    git_dir: &Path,
    common_dir: &Path,
) -> Result<(), String> {
    if !dot_git.is_file() || !git_dir.starts_with(common_dir.join("worktrees")) {
        return Err(format!(
            "out-of-tree Git directory is not a registered linked worktree: {}",
            git_dir.display()
        ));
    }
    let backlink = git_dir.join("gitdir");
    if regular_file_state(&backlink)? != FileState::Regular {
        return Err(format!(
            "linked worktree has no regular gitdir backlink: {}",
            backlink.display()
        ));
    }
    let recorded = fs::read_to_string(&backlink)
        .map_err(|error| format!("could not read {}: {error}", backlink.display()))?;
    let recorded = PathBuf::from(recorded.trim());
    let recorded = recorded
        .canonicalize()
        .map_err(|error| format!("linked worktree backlink is unavailable: {error}"))?;
    let expected = dot_git
        .canonicalize()
        .map_err(|error| format!("worktree Git file is unavailable: {error}"))?;
    if recorded != expected {
        return Err(format!(
            "linked worktree backlink does not identify {}",
            dot_git.display()
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileState {
    Missing,
    Regular,
}

fn regular_file_state(path: &Path) -> Result<FileState, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing symbolic link in repository metadata: {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_file() => Ok(FileState::Regular),
        Ok(_) => Err(format!(
            "repository metadata is not a regular file: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileState::Missing),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn canonical_real_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "{label} is not a real directory: {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("could not canonicalize {label} {}: {error}", path.display()))
}

fn read_config(
    git_program: &Path,
    path: &Path,
    source: ConfigSource,
    output: &mut Vec<(ConfigSource, String, String)>,
) -> Result<(), String> {
    match regular_file_state(path)? {
        FileState::Missing => return Ok(()),
        FileState::Regular => {}
    }
    let result = Command::new(git_program)
        .args(["config", "--file"])
        .arg(path)
        .args(["--no-includes", "--list", "--null"])
        .env("GIT_PAGER", "")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("could not parse Git config {}: {error}", path.display()))?;
    if !result.status.success() {
        return Err(format!(
            "could not safely parse Git config {}: {}",
            path.display(),
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    for record in result.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
            return Err(format!(
                "Git config {} returned a malformed record",
                path.display()
            ));
        };
        let key = std::str::from_utf8(&record[..separator])
            .map_err(|_| format!("Git config {} contains a non-UTF-8 key", path.display()))?
            .to_ascii_lowercase();
        if !safe_key(&key) {
            return Err(format!(
                "Git config {} contains an unsafe key",
                path.display()
            ));
        }
        let value = String::from_utf8_lossy(&record[separator + 1..]).into_owned();
        output.push((source, key, value));
    }
    Ok(())
}

fn safe_key(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'/' | b'~')
        })
}

fn classify(
    key: &str,
    value: &str,
    trusted_signing_key: Option<&str>,
) -> Option<(FindingKind, &'static str)> {
    let dangerous = dangerous_reason(key, value, trusted_signing_key);
    if let Some(reason) = dangerous {
        return Some((FindingKind::Dangerous, reason));
    }
    if allowlisted(key, value, trusted_signing_key) {
        None
    } else {
        Some((
            FindingKind::Unknown,
            "not recognized by sbxr's signing allowlist",
        ))
    }
}

fn dangerous_reason(
    key: &str,
    value: &str,
    trusted_signing_key: Option<&str>,
) -> Option<&'static str> {
    if matches!(
        key,
        "core.fsmonitor"
            | "core.hookspath"
            | "core.sshcommand"
            | "core.pager"
            | "core.editor"
            | "core.askpass"
            | "core.gitproxy"
            | "core.worktree"
            | "core.alternaterefscommand"
            | "core.attributesfile"
            | "sequence.editor"
            | "diff.external"
            | "interactive.difffilter"
            | "gpg.program"
    ) {
        return Some("can redirect Git to a host path or executable command");
    }
    if key.starts_with("pager.")
        || key.starts_with("credential.")
        || key.starts_with("include.")
        || key.starts_with("includeif.")
        || key.starts_with("uploadpack.")
        || key.starts_with("receive.")
        || key.starts_with("url.")
        || scoped_suffix(key, "gpg.", ".program")
        || scoped_suffix(key, "diff.", ".textconv")
        || scoped_suffix(key, "diff.", ".command")
        || scoped_suffix(key, "filter.", ".clean")
        || scoped_suffix(key, "filter.", ".smudge")
        || scoped_suffix(key, "filter.", ".process")
        || scoped_suffix(key, "filter.", ".required")
        || scoped_suffix(key, "merge.", ".driver")
        || scoped_suffix(key, "remote.", ".uploadpack")
        || scoped_suffix(key, "remote.", ".receivepack")
        || scoped_suffix(key, "remote.", ".proxy")
        || scoped_suffix(key, "remote.", ".vcs")
        || scoped_suffix(key, "protocol.", ".allow")
        || scoped_suffix(key, "tar.", ".command")
    {
        return Some("can load configuration or execute an external helper");
    }
    if key.starts_with("alias.") && value.trim_start().starts_with('!') {
        return Some("bang aliases execute a shell command");
    }
    if (scoped_suffix(key, "remote.", ".url")
        || scoped_suffix(key, "remote.", ".pushurl")
        || scoped_suffix(key, "submodule.", ".url"))
        && remote_url_can_invoke_helper(value)
    {
        return Some("the URL can select an external Git remote helper");
    }
    if scoped_suffix(key, "submodule.", ".update") && value.trim_start().starts_with('!') {
        return Some("bang submodule update strategies execute a command");
    }
    if key == "user.signingkey" && trusted_signing_key.is_none_or(|trusted| value.trim() != trusted)
    {
        return Some("repository signing key differs from trusted host configuration");
    }
    None
}

fn remote_url_can_invoke_helper(value: &str) -> bool {
    let value = value.trim_start();
    let lower = value.to_ascii_lowercase();
    if let Some((helper, _)) = lower.split_once("::") {
        if !helper.is_empty()
            && helper
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            return true;
        }
    }
    let Some((scheme, _)) = lower.split_once("://") else {
        return false;
    };
    !matches!(scheme, "http" | "https" | "ssh" | "git" | "file")
}

fn allowlisted(key: &str, value: &str, trusted_signing_key: Option<&str>) -> bool {
    if matches!(
        key,
        "core.repositoryformatversion"
            | "core.filemode"
            | "core.bare"
            | "core.logallrefupdates"
            | "core.ignorecase"
            | "core.precomposeunicode"
            | "core.symlinks"
            | "core.autocrlf"
            | "core.eol"
            | "core.safecrlf"
            | "core.quotepath"
            | "core.sparsecheckout"
            | "core.sparsecheckoutcone"
            | "core.untrackedcache"
            | "extensions.objectformat"
            | "extensions.refstorage"
            | "extensions.worktreeconfig"
            | "remote.pushdefault"
            | "pull.rebase"
            | "pull.ff"
            | "push.default"
            | "push.autosetupremote"
            | "push.followtags"
            | "fetch.prune"
            | "fetch.prunetags"
            | "fetch.recursesubmodules"
            | "init.defaultbranch"
            | "user.name"
            | "user.email"
            | "commit.gpgsign"
            | "tag.gpgsign"
            | "gpg.format"
            | "lfs.repositoryformatversion"
    ) {
        return true;
    }
    if key == "user.signingkey" {
        return trusted_signing_key.is_some_and(|trusted| value.trim() == trusted);
    }
    for suffix in [
        ".url",
        ".pushurl",
        ".fetch",
        ".push",
        ".tagopt",
        ".prune",
        ".mirror",
        ".promisor",
        ".partialclonefilter",
    ] {
        if scoped_suffix(key, "remote.", suffix) {
            return true;
        }
    }
    for suffix in [
        ".remote",
        ".merge",
        ".rebase",
        ".pushremote",
        ".description",
        ".vscode-merge-base",
    ] {
        if scoped_suffix(key, "branch.", suffix) {
            return true;
        }
    }
    for suffix in [
        ".url",
        ".active",
        ".branch",
        ".ignore",
        ".fetchrecursesubmodules",
        ".shallow",
    ] {
        if scoped_suffix(key, "submodule.", suffix) {
            return true;
        }
    }
    if scoped_suffix(key, "submodule.", ".update") {
        return matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "checkout" | "rebase" | "merge" | "none"
        );
    }
    (key.starts_with("lfs.") && key.ends_with(".access"))
        || key.starts_with("gc.")
        || key.starts_with("maintenance.")
        || key.starts_with("rerere.")
        || key.starts_with("rebase.")
}

fn scoped_suffix(key: &str, prefix: &str, suffix: &str) -> bool {
    key.strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
        .is_some_and(|subsection| !subsection.is_empty())
}

fn executable_hooks(common_dir: &Path) -> Result<Vec<String>, String> {
    let hooks_dir = common_dir.join("hooks");
    let metadata = match fs::symlink_metadata(&hooks_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "could not inspect {}: {error}",
                hooks_dir.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "repository hooks path is not a real directory: {}",
            hooks_dir.display()
        ));
    }
    let mut hooks = Vec::new();
    for entry in fs::read_dir(&hooks_dir)
        .map_err(|error| format!("could not read {}: {error}", hooks_dir.display()))?
    {
        let entry = entry.map_err(|error| format!("could not read repository hook: {error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".sample") {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("could not inspect repository hook {name}: {error}"))?;
        let executable =
            metadata.file_type().is_symlink() || (metadata.is_file() && is_executable(&metadata));
        if executable {
            hooks.push(name);
        }
    }
    hooks.sort();
    Ok(hooks)
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}

fn trusted_config_value(
    git_program: &Path,
    neutral: &Path,
    key: &str,
) -> Result<Option<String>, String> {
    let mut command = Command::new(git_program);
    scrub_git_environment(&mut command);
    let output = command
        .arg("-C")
        .arg(neutral)
        .args(["config", "--get", key])
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not inspect trusted host Git {key}: {error}"))?;
    if output.status.success() {
        let value = String::from_utf8(output.stdout)
            .map_err(|_| format!("trusted host Git {key} is not UTF-8"))?;
        return Ok(Some(value.trim_end_matches(['\r', '\n']).to_owned()));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!("could not inspect trusted host Git {key}"))
}

fn trusted_program(
    fallback: &str,
    configured: Option<&str>,
    repository: &Path,
) -> Result<PathBuf, String> {
    let raw = configured.unwrap_or(fallback).trim();
    if raw.is_empty() || raw.contains(['\0', '\r', '\n']) {
        return Err(format!("trusted {fallback} program is empty or invalid"));
    }
    let candidate = PathBuf::from(raw);
    let path = if candidate.is_absolute() || candidate.components().count() > 1 {
        candidate
    } else {
        if !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(format!(
                "trusted {fallback} program must be one executable path without arguments: {raw}"
            ));
        }
        process::command_path(raw)
            .ok_or_else(|| format!("trusted {fallback} program is unavailable: {raw}"))?
    };
    let path = path.canonicalize().map_err(|error| {
        format!(
            "trusted {fallback} program {} is unavailable: {error}",
            path.display()
        )
    })?;
    let metadata = fs::metadata(&path).map_err(|error| {
        format!(
            "could not inspect trusted program {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || !is_executable(&metadata) {
        return Err(format!(
            "trusted program is not executable: {}",
            path.display()
        ));
    }
    if path.starts_with(repository) {
        return Err(format!(
            "refusing trusted {fallback} program inside the shared repository: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn scrub_git_environment(command: &mut Command) {
    for (key, _) in env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
}

fn null_hooks_config() -> &'static str {
    if cfg!(windows) {
        "core.hooksPath=NUL"
    } else {
        "core.hooksPath=/dev/null"
    }
}

fn trusted_noop_program() -> &'static str {
    if cfg!(windows) {
        "cmd /c exit 0"
    } else {
        "/bin/true"
    }
}

fn trusted_false_program() -> &'static str {
    if cfg!(windows) {
        "cmd /c exit 1"
    } else {
        "/bin/false"
    }
}

struct NeutralDirectory {
    path: PathBuf,
}

impl NeutralDirectory {
    fn create() -> Result<Self, String> {
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            env::temp_dir().join(format!("sbxr-git-config-{}-{sequence}", std::process::id()));
        fs::create_dir(&path)
            .map_err(|error| format!("could not create neutral Git directory: {error}"))?;
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not protect neutral Git directory: {error}"))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for NeutralDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn shell_quote(path: &Path) -> String {
    shell_quote_str(&path.to_string_lossy())
}

fn shell_quote_str(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRepository {
        root: PathBuf,
    }

    impl TestRepository {
        fn create(label: &str, config: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "sbxr-git-trust-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(root.join(".git/hooks")).unwrap();
            fs::write(root.join(".git/config"), config).unwrap();
            Self { root }
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn kind(key: &str, value: &str) -> Option<FindingKind> {
        classify(key, value, Some("TRUSTED")).map(|finding| finding.0)
    }

    #[test]
    fn key_classification_is_ascii_case_insensitive() {
        assert_eq!(
            kind(&"GPG.Program".to_ascii_lowercase(), "/tmp/x"),
            Some(FindingKind::Dangerous)
        );
        assert_eq!(kind("gpg.program", "/tmp/x"), Some(FindingKind::Dangerous));
    }

    #[test]
    fn executable_config_patterns_are_denied() {
        for (key, value) in [
            ("core.fsmonitor", "/tmp/x"),
            ("core.hookspath", "/tmp/x"),
            ("core.sshcommand", "sh"),
            ("core.pager", "sh"),
            ("core.editor", "sh"),
            ("core.askpass", "sh"),
            ("core.gitproxy", "sh"),
            ("core.worktree", "/tmp"),
            ("core.alternaterefscommand", "sh"),
            ("sequence.editor", "sh"),
            ("pager.log", "sh"),
            ("alias.x", "!sh"),
            ("gpg.program", "sh"),
            ("gpg.ssh.program", "sh"),
            ("diff.external", "sh"),
            ("diff.x.textconv", "sh"),
            ("diff.x.command", "sh"),
            ("filter.lfs.clean", "sh"),
            ("filter.lfs.smudge", "sh"),
            ("filter.lfs.process", "sh"),
            ("filter.lfs.required", "true"),
            ("merge.x.driver", "sh"),
            ("credential.helper", "sh"),
            ("include.path", "/tmp/config"),
            ("includeif.gitdir:~/.path", "/tmp/config"),
            ("remote.origin.uploadpack", "sh"),
            ("remote.origin.receivepack", "sh"),
            ("remote.origin.proxy", "sh"),
            ("remote.origin.vcs", "sh"),
            ("protocol.ext.allow", "always"),
            ("uploadpack.packobjectsHook", "sh"),
            ("receive.procReceiveRefs", "sh"),
            ("submodule.x.update", "!sh"),
            ("remote.origin.url", "ext::sh -c whoami"),
            ("remote.origin.url", "helper::payload"),
            ("remote.origin.url", "unknown://payload"),
            ("submodule.x.url", "helper::payload"),
        ] {
            assert_eq!(
                kind(&key.to_ascii_lowercase(), value),
                Some(FindingKind::Dangerous),
                "{key}"
            );
        }
    }

    #[test]
    fn signing_key_must_match_host_owned_value() {
        assert_eq!(kind("user.signingkey", "TRUSTED"), None);
        assert_eq!(
            kind("user.signingkey", "OTHER"),
            Some(FindingKind::Dangerous)
        );
    }

    #[test]
    fn built_in_remote_transports_do_not_trigger_the_helper_rule() {
        for value in [
            "https://github.com/example/repository.git",
            "ssh://git@github.com/example/repository.git",
            "git@github.com:example/repository.git",
            "../repository.git",
        ] {
            assert_eq!(kind("remote.origin.url", value), None, "{value}");
        }
    }

    #[test]
    fn common_vscode_branch_metadata_is_allowlisted() {
        assert_eq!(
            kind("branch.master.vscode-merge-base", "origin/master"),
            None
        );
    }

    #[test]
    fn unknown_keys_are_informational_but_block_signing() {
        assert_eq!(kind("tool.example", "value"), Some(FindingKind::Unknown));
    }

    #[test]
    fn clean_repository_is_allowed_for_signing() {
        let repository = TestRepository::create(
            "clean",
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n",
        );
        let audit = audit(&repository.root, None).unwrap().unwrap();
        assert!(!audit.has_attention());
        assert!(!audit.has_unknown());
        assert!(audit.signing_allowed());
    }

    #[test]
    fn executable_local_config_is_detected_without_executing_it() {
        let marker = env::temp_dir().join(format!("sbxr-git-trust-marker-{}", std::process::id()));
        let _ = fs::remove_file(&marker);
        let repository = TestRepository::create(
            "dangerous",
            &format!(
                "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = touch {}\n[gpg]\n\tprogram = touch {}\n[include]\n\tpath = /tmp/untrusted-git-config\n",
                marker.display(),
                marker.display()
            ),
        );
        let audit = audit(&repository.root, None).unwrap().unwrap();
        assert!(audit.has_attention());
        assert!(!audit.signing_allowed());
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.key == "core.fsmonitor")
        );
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.key == "gpg.program")
        );
        assert!(
            audit
                .findings
                .iter()
                .any(|finding| finding.key == "include.path")
        );
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn executable_hooks_and_symlinked_git_metadata_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let repository = TestRepository::create("hook", "[core]\n\trepositoryformatversion = 0\n");
        let hook = repository.root.join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
        let hook_audit = audit(&repository.root, None).unwrap().unwrap();
        assert!(hook_audit.has_attention());
        assert_eq!(hook_audit.hooks, ["pre-commit"]);

        let link_root = repository.root.join("linked-metadata");
        fs::create_dir(&link_root).unwrap();
        symlink(repository.root.join(".git"), link_root.join(".git")).unwrap();
        assert!(audit(&link_root, None).unwrap_err().contains("symlinked"));
    }
}
