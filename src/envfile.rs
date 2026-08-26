use crate::cli::Preset;
use crate::environment::{Context, Project, RocksDbHost};
use crate::{sbx, sha256};
use serde_json::json;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const GUEST_CONTRACT_MARKER: &str = "/home/agent/.local/state/sbxr/environment-contract-v1";
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Development(Preset),
    Review,
}

pub(crate) struct Definition {
    path: PathBuf,
    state_directory: PathBuf,
    metadata_path: PathBuf,
    sandbox_name: String,
    contract_hash: String,
    project_path: PathBuf,
    kind: Kind,
    rocksdb_prefix: Option<PathBuf>,
    name_overridden: bool,
}

pub(crate) struct Inspection {
    sandbox_name: String,
    project_path: PathBuf,
    environment_path: PathBuf,
    source: &'static str,
    contract_status: String,
    yaml: String,
}

impl Inspection {
    pub(crate) fn print(&self) {
        println!("sandbox: {}", self.sandbox_name);
        println!("project: {}", self.project_path.display());
        println!("environment: {}", self.environment_path.display());
        println!("declaration: {}", self.source);
        println!("contract: {}", self.contract_status);
        println!("\n--- environment ---");
        print!("{}", self.yaml);
        if !self.yaml.ends_with('\n') {
            println!();
        }
    }
}

impl Definition {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn verify_existing(&self, preserve_xdg_state: bool) -> Result<(), String> {
        let guest = sbx::read_file(
            preserve_xdg_state,
            &self.sandbox_name,
            GUEST_CONTRACT_MARKER,
        )?;
        let guest_hash = guest
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(str::trim);
        if guest_hash != Some(self.contract_hash.as_str()) {
            return Err(self.recreate_message());
        }

        match self.applied_host_hash()? {
            Some(hash) if hash == self.contract_hash => Ok(()),
            Some(_) => Err(self.recreate_message()),
            None => {
                // A matching guest marker can only be written after sbxr successfully
                // creates this exact contract. Recover a host-side write interrupted
                // between the guest marker and the final metadata rename.
                self.write_applied_host_record()
            }
        }
    }

    pub(crate) fn mark_applied(&self, preserve_xdg_state: bool) -> Result<(), String> {
        sbx::write_file(
            preserve_xdg_state,
            &self.sandbox_name,
            GUEST_CONTRACT_MARKER,
            OsStr::new(&self.contract_hash),
        )?;
        self.write_applied_host_record()
    }

    pub(crate) fn remove_host_state(self, state_root: &Path) -> Result<(), String> {
        let projects = state_root.join("projects");
        if self.state_directory.parent() != Some(projects.as_path()) {
            return Err(format!(
                "refusing to remove unexpected sbxr state path: {}",
                self.state_directory.display()
            ));
        }
        if !self.state_directory.exists() {
            return Ok(());
        }
        let metadata = fs::symlink_metadata(&self.state_directory).map_err(|error| {
            format!(
                "could not inspect {} before removal: {error}",
                self.state_directory.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "refusing to remove unsafe sbxr state path: {}",
                self.state_directory.display()
            ));
        }
        fs::remove_dir_all(&self.state_directory).map_err(|error| {
            format!(
                "sandbox was removed, but trusted host state {} could not be removed: {error}",
                self.state_directory.display()
            )
        })
    }

    fn applied_host_hash(&self) -> Result<Option<String>, String> {
        Ok(self.read_host_record()?.and_then(|value| {
            value
                .get("appliedHash")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        }))
    }

    fn read_host_record(&self) -> Result<Option<serde_json::Value>, String> {
        read_host_record(&self.metadata_path, &self.sandbox_name)
    }

    fn write_desired_host_record(&self) -> Result<(), String> {
        let applied = self.applied_host_hash()?;
        self.write_host_record(applied.as_deref())
    }

    fn write_applied_host_record(&self) -> Result<(), String> {
        self.write_host_record(Some(&self.contract_hash))
    }

    fn write_host_record(&self, applied_hash: Option<&str>) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(&json!({
            "version": 1,
            "sandbox": self.sandbox_name,
            "desiredHash": self.contract_hash,
            "appliedHash": applied_hash,
        }))
        .map_err(|error| format!("could not serialize environment contract: {error}"))?;
        atomic_write(&self.metadata_path, &contents)
    }

    fn recreate_message(&self) -> String {
        let project = shell_display(&self.project_path);
        match self.kind {
            Kind::Development(preset) => {
                let mut prefix = String::new();
                if self.name_overridden {
                    prefix.push_str("SBXR_NAME=");
                    prefix.push_str(&shell_quote_str(&self.sandbox_name));
                    prefix.push(' ');
                }
                let mut options = format!("--preset {}", preset.name());
                if let Some(rocksdb) = &self.rocksdb_prefix {
                    options.push_str(" --rocksdb-host=");
                    options.push_str(&shell_display(rocksdb));
                }
                format!(
                    "sandbox '{}' exists but was created from a different or unverifiable environment contract\nNo state was removed. To recreate it explicitly, run:\n  {prefix}sbxr {options} rm {project}\n  {prefix}sbxr {options} new {project}",
                    self.sandbox_name
                )
            }
            Kind::Review => format!(
                "review sandbox '{}' exists but was created from a different or unverifiable environment contract\nNo state was removed. Remove that review sandbox explicitly with `sbx rm {}` and retry `sbxr review {project}`",
                self.sandbox_name, self.sandbox_name
            ),
        }
    }
}

pub(crate) fn development_path(context: &Context, project: &Project) -> PathBuf {
    context
        .state_root
        .join("projects")
        .join(&project.name)
        .join("definitions/.sbxenv.yaml")
}

pub(crate) fn inspect(
    context: &Context,
    project: &Project,
    kind: Kind,
    rocksdb: Option<&RocksDbHost>,
) -> Result<Inspection, String> {
    let sandbox_name = match kind {
        Kind::Development(_) => &project.name,
        Kind::Review => &project.review_name,
    };
    let state_directory = context.state_root.join("projects").join(sandbox_name);
    let definitions = state_directory.join("definitions");
    let metadata_directory = state_directory.join("metadata");
    reject_planned_mounted_state(&definitions, &project.path, rocksdb)?;
    reject_symlink_components(&definitions)?;
    reject_symlink_components(&metadata_directory)?;
    for directory in [
        &context.state_root,
        &context.state_root.join("projects"),
        &state_directory,
        &definitions,
        &metadata_directory,
    ] {
        if directory.exists() {
            inspect_secure_directory(directory)?;
        }
    }

    let environment_path = definitions.join(".sbxenv.yaml");
    let metadata_path = metadata_directory.join("contract.json");
    reject_symlink_file(&environment_path)?;
    let current = render(context, project, kind, rocksdb)?;
    let stored = match fs::read_to_string(&environment_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "could not read {}: {error}",
                environment_path.display()
            ));
        }
    };
    let record = read_host_record(&metadata_path, sandbox_name)?;
    let source = if stored.is_some() {
        "materialized"
    } else {
        "prospective (not written)"
    };
    let contract_status = if let Some(stored) = &stored {
        if stored != &current {
            "recreation required: materialized YAML differs from the current desired declaration"
                .to_owned()
        } else {
            match contract_hash(context, kind, rocksdb, current.as_bytes()) {
                Ok(current_hash) => match record.as_ref() {
                    Some(record)
                        if record
                            .get("desiredHash")
                            .and_then(serde_json::Value::as_str)
                            == Some(current_hash.as_str())
                            && record
                                .get("appliedHash")
                                .and_then(serde_json::Value::as_str)
                                == Some(current_hash.as_str()) =>
                    {
                        "desired and applied host records match".to_owned()
                    }
                    Some(record)
                        if record
                            .get("appliedHash")
                            .is_none_or(serde_json::Value::is_null) =>
                    {
                        "desired declaration exists but has not been recorded as applied".to_owned()
                    }
                    Some(_) => {
                        "recreation required: desired and applied contracts differ".to_owned()
                    }
                    None => "unverifiable: contract metadata is missing".to_owned(),
                },
                Err(error) => format!("unverifiable: {error}"),
            }
        }
    } else {
        "not materialized; no sandbox contract has been applied".to_owned()
    };

    Ok(Inspection {
        sandbox_name: sandbox_name.to_owned(),
        project_path: project.path.clone(),
        environment_path,
        source,
        contract_status,
        yaml: stored.unwrap_or(current),
    })
}

fn read_host_record(
    metadata_path: &Path,
    sandbox_name: &str,
) -> Result<Option<serde_json::Value>, String> {
    reject_symlink_file(metadata_path)?;
    let bytes = match fs::read(metadata_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not read {}: {error}",
                metadata_path.display()
            ));
        }
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "trusted contract metadata {} is invalid: {error}",
            metadata_path.display()
        )
    })?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(1)
        || value.get("sandbox").and_then(serde_json::Value::as_str) != Some(sandbox_name)
    {
        return Err(format!(
            "trusted contract metadata does not describe sandbox {sandbox_name}; remove {} and retry",
            metadata_path.display()
        ));
    }
    Ok(Some(value))
}

pub(crate) fn materialize(
    context: &Context,
    project: &Project,
    kind: Kind,
    rocksdb: Option<&RocksDbHost>,
) -> Result<Definition, String> {
    let sandbox_name = match kind {
        Kind::Development(_) => &project.name,
        Kind::Review => &project.review_name,
    };
    let state_directory = context.state_root.join("projects").join(sandbox_name);
    let definitions = state_directory.join("definitions");
    let metadata = state_directory.join("metadata");
    reject_planned_mounted_state(&definitions, &project.path, rocksdb)?;
    secure_directory(&context.state_root)?;
    secure_directory(&context.state_root.join("projects"))?;
    secure_directory(&state_directory)?;
    secure_directory(&definitions)?;
    secure_directory(&metadata)?;

    let definitions = definitions.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize trusted definitions directory {}: {error}",
            definitions.display()
        )
    })?;
    reject_mounted_state(&definitions, &project.path, rocksdb)?;

    let path = definitions.join(".sbxenv.yaml");
    reject_symlink_file(&path)?;
    let contents = render(context, project, kind, rocksdb)?;
    atomic_write(&path, contents.as_bytes())?;
    let contract_hash = contract_hash(context, kind, rocksdb, contents.as_bytes())?;

    let definition = Definition {
        path,
        state_directory,
        metadata_path: metadata.join("contract.json"),
        sandbox_name: sandbox_name.to_owned(),
        contract_hash,
        project_path: project.path.clone(),
        kind,
        rocksdb_prefix: rocksdb.map(|rocksdb| rocksdb.prefix.clone()),
        name_overridden: std::env::var_os("SBXR_NAME").is_some(),
    };
    definition.write_desired_host_record()?;
    Ok(definition)
}

pub(crate) fn validate_state_root(context: &Context) -> Result<(), String> {
    if !context.state_root.exists() {
        println!(
            "ok   trusted declarative state root will be created at {}",
            context.state_root.display()
        );
        return Ok(());
    }
    inspect_secure_directory(&context.state_root)?;
    let projects = context.state_root.join("projects");
    if projects.exists() {
        inspect_secure_directory(&projects)?;
    }
    let root = context.state_root.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize trusted state root {}: {error}",
            context.state_root.display()
        )
    })?;
    println!("ok   trusted declarative state root {}", root.display());
    Ok(())
}

fn inspect_secure_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "trusted sbxr state path is not a real directory: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "trusted sbxr state directory is accessible to group/other users: {} (expected mode 0700)",
            path.display()
        ));
    }
    Ok(())
}

fn render(
    context: &Context,
    project: &Project,
    kind: Kind,
    rocksdb: Option<&RocksDbHost>,
) -> Result<String, String> {
    let (sandbox_name, agent, clone) = match kind {
        Kind::Development(preset) => (&project.name, preset.primary_agent(), false),
        Kind::Review => (&project.review_name, "shell", true),
    };
    let kits = selected_kits(context, kind, rocksdb.is_some());
    let mut output = String::new();
    output.push_str("schemaVersion: \"1\"\nname: ");
    output.push_str(&yaml_string(sandbox_name)?);
    output.push_str("\nagent: ");
    output.push_str(&yaml_string(agent)?);
    output.push_str("\nworkspace:\n  path: ");
    output.push_str(&yaml_path(&project.path)?);
    output.push_str("\n  clone: ");
    output.push_str(if clone { "true" } else { "false" });
    output.push_str("\nkits:\n");
    for kit in kits {
        output.push_str("  - ");
        output.push_str(&yaml_path(&kit)?);
        output.push('\n');
    }
    if let Some(rocksdb) = rocksdb {
        output.push_str("additionalWorkspaces:\n  - path: ");
        output.push_str(&yaml_path(&rocksdb.prefix)?);
        output.push_str("\n    readOnly: true\nenv:\n  ROCKSDB_LIB_DIR: ");
        output.push_str(&yaml_string(&rocksdb.library_dir)?);
        output.push_str("\n  ROCKSDB_INCLUDE_DIR: ");
        output.push_str(&yaml_string(&rocksdb.include_dir)?);
        output.push_str("\n  LD_LIBRARY_PATH: ");
        output.push_str(&yaml_string(&rocksdb.library_dir)?);
        output.push('\n');
    }
    Ok(output)
}

fn selected_kits(context: &Context, kind: Kind, rocksdb: bool) -> Vec<PathBuf> {
    let mut kits = vec![context.kit_root.join("rust")];
    if let Kind::Development(preset) = kind {
        kits.push(context.kit_root.join("git-ssh-sign"));
        kits.push(context.kit_root.join("github-ssh"));
        if preset == Preset::MultiAgent {
            kits.push(context.kit_root.join("claude-cli"));
            kits.push(context.kit_root.join("pi-cli"));
        }
    }
    if rocksdb {
        kits.push(context.kit_root.join("rocksdb-host"));
    }
    kits.push(context.kit_root.join("vscode-remote"));

    kits
}

fn contract_hash(
    context: &Context,
    kind: Kind,
    rocksdb: Option<&RocksDbHost>,
    definition: &[u8],
) -> Result<String, String> {
    let mut input = b"sbxr-environment-contract-v1\0".to_vec();
    append_contract_part(&mut input, definition);
    for kit in selected_kits(context, kind, rocksdb.is_some()) {
        let spec = kit.join("spec.yaml");
        let contents = fs::read(&spec)
            .map_err(|error| format!("could not hash selected kit {}: {error}", spec.display()))?;
        append_contract_part(&mut input, kit.as_os_str().as_encoded_bytes());
        append_contract_part(&mut input, &contents);
    }
    Ok(sha256::hex(&input))
}

fn append_contract_part(input: &mut Vec<u8>, part: &[u8]) {
    input.extend_from_slice(&(part.len() as u64).to_be_bytes());
    input.extend_from_slice(part);
}

fn yaml_path(path: &Path) -> Result<String, String> {
    let value = path.to_str().ok_or_else(|| {
        format!(
            "path is not valid UTF-8 and cannot be used by sbx env: {}",
            path.display()
        )
    })?;
    yaml_string(value)
}

fn yaml_string(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("could not encode YAML scalar: {error}"))
}

fn reject_mounted_state(
    definitions: &Path,
    project: &Path,
    rocksdb: Option<&RocksDbHost>,
) -> Result<(), String> {
    let project = project.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize workspace {}: {error}",
            project.display()
        )
    })?;
    if definitions.starts_with(&project) {
        return Err(format!(
            "trusted environment definitions must be outside the mounted workspace: {}",
            definitions.display()
        ));
    }
    if let Some(rocksdb) = rocksdb {
        let prefix = rocksdb.prefix.canonicalize().map_err(|error| {
            format!(
                "could not canonicalize RocksDB workspace {}: {error}",
                rocksdb.prefix.display()
            )
        })?;
        if definitions.starts_with(prefix) {
            return Err(format!(
                "trusted environment definitions must be outside additional workspaces: {}",
                definitions.display()
            ));
        }
    }
    Ok(())
}

fn reject_planned_mounted_state(
    definitions: &Path,
    project: &Path,
    rocksdb: Option<&RocksDbHost>,
) -> Result<(), String> {
    let candidate = if definitions.is_absolute() {
        definitions.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("could not resolve the current directory: {error}"))?
            .join(definitions)
    };
    let candidate = lexical_normalize(&candidate);
    let project = project.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize workspace {}: {error}",
            project.display()
        )
    })?;
    if candidate.starts_with(&project) {
        return Err(format!(
            "trusted environment definitions must be outside the mounted workspace: {}",
            candidate.display()
        ));
    }
    if let Some(rocksdb) = rocksdb {
        let prefix = rocksdb.prefix.canonicalize().map_err(|error| {
            format!(
                "could not canonicalize RocksDB workspace {}: {error}",
                rocksdb.prefix.display()
            )
        })?;
        if candidate.starts_with(prefix) {
            return Err(format!(
                "trusted environment definitions must be outside additional workspaces: {}",
                candidate.display()
            ));
        }
    }
    Ok(())
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            component => output.push(component.as_os_str()),
        }
    }
    output
}

fn secure_directory(path: &Path) -> Result<(), String> {
    reject_symlink_components(path)?;
    if path.exists() {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "trusted sbxr state path is not a real directory: {}",
                path.display()
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    }
    reject_symlink_components(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect {}: {error}", path.display()))?;
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("could not resolve the current directory: {error}"))?
            .join(path)
    };
    for component in absolute.ancestors() {
        match fs::symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing symlinked trusted sbxr state path component: {}",
                    component.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect trusted state path component {}: {error}",
                    component.display()
                ));
            }
        }
    }
    Ok(())
}

fn reject_symlink_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing symlinked trusted sbxr state file: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "trusted sbxr state path is not a regular file: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    reject_symlink_file(path)?;
    if fs::read(path).ok().as_deref() == Some(contents) {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("state file has no parent: {}", path.display()))?;
    secure_directory(parent)?;
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".sbxr-tmp-{}-{sequence}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("could not create {}: {error}", temporary.display()))?;
    let result = (|| {
        file.write_all(contents)
            .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("could not sync {}: {error}", temporary.display()))?;
        drop(file);
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)
                .map_err(|error| format!("could not replace {}: {error}", path.display()))?;
        }
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "could not atomically replace {} with {}: {error}",
                path.display(),
                temporary.display()
            )
        })?;
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not protect {}: {error}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn shell_display(path: &Path) -> String {
    shell_quote_str(&path.to_string_lossy())
}

fn shell_quote_str(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_are_safe_yaml_scalars() {
        assert_eq!(
            yaml_string("hello: world\nnext").unwrap(),
            "\"hello: world\\nnext\""
        );
        assert_eq!(yaml_string("quote \\\"").unwrap(), "\"quote \\\\\\\"\"");
    }

    #[test]
    fn renders_release_controlled_presets_without_secrets() {
        let context = Context {
            kit_root: PathBuf::from("/trusted/kits"),
            state_root: PathBuf::from("/trusted/state"),
            preserve_xdg_state: false,
        };
        let project = Project {
            path: PathBuf::from("/work/project with spaces"),
            name: "rust-multi-project".to_owned(),
            review_name: "rust-review-project".to_owned(),
        };
        let development = render(
            &context,
            &project,
            Kind::Development(Preset::MultiAgent),
            None,
        )
        .unwrap();
        assert!(development.contains("/trusted/kits/claude-cli"));
        assert!(development.contains("/trusted/kits/pi-cli"));
        assert!(!development.contains("codex-cli"));
        assert!(!development.contains("secret"));
        assert!(development.contains("\"/work/project with spaces\""));

        let review = render(&context, &project, Kind::Review, None).unwrap();
        assert!(review.contains("agent: \"shell\""));
        assert!(review.contains("clone: true"));
        assert!(!review.contains("github-ssh"));
        assert!(!review.contains("claude-cli"));
    }

    #[test]
    fn recreate_guidance_preserves_identity_options() {
        let definition = Definition {
            path: PathBuf::from("/state/definitions/.sbxenv.yaml"),
            state_directory: PathBuf::from("/state"),
            metadata_path: PathBuf::from("/state/metadata/contract.json"),
            sandbox_name: "custom-name".to_owned(),
            contract_hash: "hash".to_owned(),
            project_path: PathBuf::from("/work/project name"),
            kind: Kind::Development(Preset::Codex),
            rocksdb_prefix: Some(PathBuf::from("/opt/rocks db")),
            name_overridden: true,
        };
        let message = definition.recreate_message();
        assert!(message.contains("SBXR_NAME='custom-name'"));
        assert!(message.contains("--preset rust-codex"));
        assert!(message.contains("--rocksdb-host='/opt/rocks db'"));
        assert!(message.contains("rm '/work/project name'"));
        assert!(message.contains("new '/work/project name'"));
    }

    #[test]
    fn contract_changes_when_a_selected_kit_changes() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-contract-kit-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        for kit in ["rust", "git-ssh-sign", "github-ssh", "vscode-remote"] {
            let directory = root.join("kits").join(kit);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("spec.yaml"), format!("name: {kit}\n")).unwrap();
        }
        let context = Context {
            kit_root: root.join("kits"),
            state_root: root.join("state"),
            preserve_xdg_state: false,
        };
        let before = contract_hash(
            &context,
            Kind::Development(Preset::Codex),
            None,
            b"definition",
        )
        .unwrap();
        fs::write(
            context.kit_root.join("rust/spec.yaml"),
            "name: rust\nchanged: true\n",
        )
        .unwrap();
        let after = contract_hash(
            &context,
            Kind::Development(Preset::Codex),
            None,
            b"definition",
        )
        .unwrap();
        assert_ne!(before, after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desired_contract_is_not_recorded_as_applied() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-contract-record-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let definition = Definition {
            path: root.join("definitions/.sbxenv.yaml"),
            state_directory: root.clone(),
            metadata_path: root.join("metadata/contract.json"),
            sandbox_name: "sandbox".to_owned(),
            contract_hash: "desired-contract".to_owned(),
            project_path: PathBuf::from("/work/project"),
            kind: Kind::Development(Preset::Codex),
            rocksdb_prefix: None,
            name_overridden: false,
        };
        definition.write_desired_host_record().unwrap();
        let desired = definition.read_host_record().unwrap().unwrap();
        assert_eq!(desired["desiredHash"], "desired-contract");
        assert!(desired["appliedHash"].is_null());

        definition.write_applied_host_record().unwrap();
        let applied = definition.read_host_record().unwrap().unwrap();
        assert_eq!(applied["desiredHash"], "desired-contract");
        assert_eq!(applied["appliedHash"], "desired-contract");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_definition_directory_inside_workspace() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        let definitions = workspace.join("state/definitions");
        fs::create_dir_all(&definitions).unwrap();
        let definitions = definitions.canonicalize().unwrap();
        assert!(reject_mounted_state(&definitions, &workspace, None).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_uncreated_state_inside_workspace_without_writing_it() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-prewrite-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let state_root = workspace.join("untrusted-state");
        let context = Context {
            kit_root: root.join("kits"),
            state_root: state_root.clone(),
            preserve_xdg_state: false,
        };
        let project = Project {
            path: workspace,
            name: "sandbox".to_owned(),
            review_name: "review".to_owned(),
        };
        assert!(materialize(&context, &project, Kind::Development(Preset::Codex), None).is_err());
        assert!(!state_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn protects_materialized_state_directories_and_files() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-mode-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        for kit in ["rust", "git-ssh-sign", "github-ssh", "vscode-remote"] {
            let directory = root.join("kits").join(kit);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("spec.yaml"), format!("name: {kit}\n")).unwrap();
        }
        let context = Context {
            kit_root: root.join("kits"),
            state_root: root.join("state"),
            preserve_xdg_state: false,
        };
        let project = Project {
            path: workspace,
            name: "sandbox".to_owned(),
            review_name: "review".to_owned(),
        };
        let definition =
            materialize(&context, &project, Kind::Development(Preset::Codex), None).unwrap();
        let directory_mode = fs::metadata(definition.path().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let definition_mode = fs::metadata(definition.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let metadata_mode = fs::metadata(&definition.metadata_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(definition_mode, 0o600);
        assert_eq!(metadata_mode, 0o600);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspection_previews_without_writing_state() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-inspect-preview-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let context = Context {
            kit_root: root.join("kits"),
            state_root: root.join("state"),
            preserve_xdg_state: false,
        };
        let project = Project {
            path: workspace,
            name: "sandbox".to_owned(),
            review_name: "review".to_owned(),
        };
        let inspection =
            inspect(&context, &project, Kind::Development(Preset::Codex), None).unwrap();
        assert_eq!(inspection.source, "prospective (not written)");
        assert!(inspection.contract_status.starts_with("not materialized"));
        assert!(inspection.yaml.contains("agent: \"codex\""));
        assert!(!context.state_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspection_reports_matching_materialized_contract() {
        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-inspect-contract-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        for kit in ["rust", "git-ssh-sign", "github-ssh", "vscode-remote"] {
            let directory = root.join("kits").join(kit);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("spec.yaml"), format!("name: {kit}\n")).unwrap();
        }
        let context = Context {
            kit_root: root.join("kits"),
            state_root: root.join("state"),
            preserve_xdg_state: false,
        };
        let project = Project {
            path: workspace,
            name: "sandbox".to_owned(),
            review_name: "review".to_owned(),
        };
        let definition =
            materialize(&context, &project, Kind::Development(Preset::Codex), None).unwrap();
        definition.write_applied_host_record().unwrap();
        let inspection =
            inspect(&context, &project, Kind::Development(Preset::Codex), None).unwrap();
        assert_eq!(inspection.source, "materialized");
        assert_eq!(
            inspection.contract_status,
            "desired and applied host records match"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_authoritative_file() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-link-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target");
        fs::write(&target, "data").unwrap();
        let link = root.join("contract.json");
        symlink(&target, &link).unwrap();
        assert!(atomic_write(&link, b"replacement").is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "data");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_authoritative_directory_component() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "sbxr-envfile-dir-link-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        let link = root.join("linked-state");
        symlink(&target, &link).unwrap();
        assert!(secure_directory(&link.join("projects")).is_err());
        assert!(!target.join("projects").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
