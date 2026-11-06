use crate::cli::Preset;
use crate::environment::Context;
use crate::profile::Profile;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Status {
    Pending,
    Verified,
    Skipped,
    NeedsAttention,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Verified => "verified",
            Self::Skipped => "skipped",
            Self::NeedsAttention => "needs-attention",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "verified" => Some(Self::Verified),
            "skipped" => Some(Self::Skipped),
            "needs-attention" => Some(Self::NeedsAttention),
            _ => None,
        }
    }

    fn display(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Verified => "verified",
            Self::Skipped => "skipped by profile",
            Self::NeedsAttention => "needs attention",
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    status: Status,
    checked_at: Option<u64>,
    detail: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Ledger {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl Ledger {
    pub(crate) fn load(
        context: &Context,
        sandbox_name: &str,
        preset: Preset,
        profile: &Profile,
    ) -> Result<Self, String> {
        let path = path(context, sandbox_name);
        reject_symlink(&path)?;
        let mut entries = match fs::read(&path) {
            Ok(bytes) => parse(&bytes, &path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        for (name, enabled) in expected(preset, profile) {
            let entry = entries.entry(name.to_owned()).or_insert(Entry {
                status: Status::Pending,
                checked_at: None,
                detail: "not checked".to_owned(),
            });
            if !enabled {
                entry.status = Status::Skipped;
                entry.checked_at = None;
                entry.detail = "disabled by project profile".to_owned();
            } else if entry.status == Status::Skipped {
                entry.status = Status::Pending;
                entry.detail = "not checked".to_owned();
            }
        }
        Ok(Self { path, entries })
    }

    pub(crate) fn mark(&mut self, name: &str, status: Status, detail: &str) -> Result<(), String> {
        self.entries.insert(
            name.to_owned(),
            Entry {
                status,
                checked_at: Some(now()),
                detail: safe_detail(detail),
            },
        );
        self.save()
    }

    pub(crate) fn begin_verification(&mut self) -> Result<(), String> {
        for entry in self.entries.values_mut() {
            if entry.status != Status::Skipped {
                entry.status = Status::Pending;
                entry.checked_at = None;
                entry.detail = "not checked in this verification run".to_owned();
            }
        }
        self.save()
    }

    pub(crate) fn print(&self) {
        println!("integration health: {}", self.path.display());
        for (name, entry) in &self.entries {
            println!(
                "  {name:<18} {:<18} {}",
                entry.status.display(),
                entry.detail
            );
        }
    }

    pub(crate) fn needs_repair(&self) -> bool {
        self.entries
            .iter()
            .any(|(name, entry)| name != "ssh-agent" && entry.status == Status::NeedsAttention)
    }

    pub(crate) fn needs_verification(&self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.status == Status::Pending)
    }

    fn save(&self) -> Result<(), String> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| format!("health path has no parent: {}", self.path.display()))?;
        secure_directory(directory)?;
        reject_symlink(&self.path)?;
        let entries = self
            .entries
            .iter()
            .map(|(name, entry)| {
                (
                    name.clone(),
                    json!({
                        "status": entry.status.name(),
                        "checkedAt": entry.checked_at,
                        "detail": entry.detail,
                    }),
                )
            })
            .collect::<Map<_, _>>();
        let bytes = serde_json::to_vec_pretty(&json!({
            "version": 1,
            "sbxrVersion": env!("CARGO_PKG_VERSION"),
            "integrations": entries,
        }))
        .map_err(|error| format!("could not serialize integration health: {error}"))?;
        atomic_write(&self.path, &bytes)
    }
}

fn expected(preset: Preset, profile: &Profile) -> Vec<(&'static str, bool)> {
    vec![
        ("runtime", true),
        ("guest-credentials", true),
        ("openai", preset.has_codex()),
        ("anthropic", preset.has_claude()),
        ("github", profile.github_proxy),
        ("pi", preset == Preset::MultiAgent),
        ("rust-toolchain", true),
        ("paseo", true),
        ("paseo-relay", profile.paseo_relay),
        ("browser", profile.browser),
        ("orca", profile.orca),
        ("ssh-agent", true),
    ]
}

fn path(context: &Context, sandbox_name: &str) -> PathBuf {
    context
        .state_root
        .join("integration-health")
        .join(format!("{sandbox_name}.json"))
}

pub(crate) fn remove(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let path = path(context, sandbox_name);
    reject_symlink(&path)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not remove {}: {error}", path.display())),
    }
}

fn parse(bytes: &[u8], path: &Path) -> Result<BTreeMap<String, Entry>, String> {
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid integration health {}: {error}", path.display()))?;
    if document.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(format!(
            "integration health {} has an unsupported version",
            path.display()
        ));
    }
    let stale =
        document.get("sbxrVersion").and_then(Value::as_str) != Some(env!("CARGO_PKG_VERSION"));
    let mut entries = BTreeMap::new();
    let values = document
        .get("integrations")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!(
                "integration health {} has no integrations object",
                path.display()
            )
        })?;
    for (name, value) in values {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .and_then(Status::parse)
            .ok_or_else(|| {
                format!(
                    "integration health {} has an invalid status",
                    path.display()
                )
            })?;
        let checked_at = value.get("checkedAt").and_then(Value::as_u64);
        let detail = value
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("no detail")
            .to_owned();
        entries.insert(
            name.clone(),
            Entry {
                status: if stale && status == Status::Verified {
                    Status::Pending
                } else {
                    status
                },
                checked_at,
                detail: if stale && status == Status::Verified {
                    "sbxr version changed; recheck required".to_owned()
                } else {
                    detail
                },
            },
        );
    }
    Ok(entries)
}

fn safe_detail(detail: &str) -> String {
    detail
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || " -_./".contains(*character))
        .take(96)
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn secure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "health path is not a real directory: {}",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)
            .map_err(|error| format!("could not create {}: {error}", path.display()))?,
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect {}: {error}", path.display()))?;
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing symlinked health file: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "health path is not a regular file: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("health path has no parent")?;
    let temporary = parent.join(format!(
        ".health-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("could not create {}: {error}", temporary.display()))?;
    let result = (|| {
        file.write_all(bytes)
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

    fn temporary_context(label: &str) -> (PathBuf, Context) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sbxr-health-{label}-{}-{unique}",
            std::process::id()
        ));
        let context = Context {
            kit_root: root.join("kits"),
            state_root: root.join("state"),
            preserve_xdg_state: false,
        };
        (root, context)
    }

    #[test]
    fn rejects_unknown_statuses() {
        let path = Path::new("health.json");
        let bytes = br#"{"version":1,"integrations":{"runtime":{"status":"maybe"}}}"#;
        assert!(parse(bytes, path).is_err());
    }

    #[test]
    fn controlled_details_do_not_retain_shell_metacharacters() {
        assert_eq!(
            safe_detail("authentication: bad; token=$SECRET"),
            "authentication bad tokenSECRET"
        );
    }

    #[test]
    fn pending_and_attention_have_distinct_actions() {
        let (root, context) = temporary_context("actions");
        let mut ledger =
            Ledger::load(&context, "sandbox", Preset::MultiAgent, &Profile::default()).unwrap();

        assert!(ledger.needs_verification());
        assert!(!ledger.needs_repair());
        ledger
            .mark(
                "ssh-agent",
                Status::NeedsAttention,
                "forwarded agent unavailable or empty",
            )
            .unwrap();
        assert!(!ledger.needs_repair());
        ledger
            .mark("runtime", Status::NeedsAttention, "verification failed")
            .unwrap();
        assert!(ledger.needs_repair());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verification_start_resets_enabled_results_but_preserves_skips() {
        let (root, context) = temporary_context("reset");
        let mut ledger =
            Ledger::load(&context, "sandbox", Preset::MultiAgent, &Profile::default()).unwrap();
        ledger
            .mark("runtime", Status::Verified, "sandbox contract matches")
            .unwrap();
        ledger
            .mark("github", Status::Verified, "proxy route works")
            .unwrap();
        ledger.begin_verification().unwrap();

        assert_eq!(ledger.entries["runtime"].status, Status::Pending);
        assert_eq!(ledger.entries["github"].status, Status::Pending);
        assert_eq!(ledger.entries["browser"].status, Status::Skipped);
        assert_eq!(ledger.entries["orca"].status, Status::Skipped);
        assert_eq!(ledger.entries["runtime"].checked_at, None);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_merges_expected_entries_and_unskips_enabled_integrations() {
        let (root, context) = temporary_context("merge");
        let default_profile = Profile::default();
        let mut ledger =
            Ledger::load(&context, "sandbox", Preset::MultiAgent, &default_profile).unwrap();
        assert_eq!(ledger.entries["browser"].status, Status::Skipped);
        ledger.mark("runtime", Status::Verified, "ok").unwrap();

        let mut enabled_profile = default_profile.clone();
        enabled_profile.browser = true;
        let enabled =
            Ledger::load(&context, "sandbox", Preset::MultiAgent, &enabled_profile).unwrap();
        assert_eq!(enabled.entries["runtime"].status, Status::Verified);
        assert_eq!(enabled.entries["browser"].status, Status::Pending);
        assert!(enabled.entries.contains_key("guest-credentials"));

        let disabled =
            Ledger::load(&context, "sandbox", Preset::MultiAgent, &default_profile).unwrap();
        assert_eq!(disabled.entries["browser"].status, Status::Skipped);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_change_invalidates_only_verified_results() {
        let path = Path::new("health.json");
        let bytes = br#"{
          "version": 1,
          "sbxrVersion": "older",
          "integrations": {
            "runtime": {"status":"verified","checkedAt":1,"detail":"ok"},
            "github": {"status":"needs-attention","checkedAt":2,"detail":"failed"}
          }
        }"#;
        let entries = parse(bytes, path).unwrap();

        assert_eq!(entries["runtime"].status, Status::Pending);
        assert_eq!(entries["github"].status, Status::NeedsAttention);
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_a_symlinked_ledger() {
        use std::os::unix::fs::symlink;

        let (root, context) = temporary_context("symlink");
        let ledger_path = path(&context, "sandbox");
        std::fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
        let target = root.join("target.json");
        std::fs::write(&target, b"{}").unwrap();
        symlink(&target, &ledger_path).unwrap();

        assert!(
            Ledger::load(&context, "sandbox", Preset::MultiAgent, &Profile::default()).is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
