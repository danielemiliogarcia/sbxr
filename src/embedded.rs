use std::fs;
use std::path::Path;

const KITS: [(&str, &str); 6] = [
    ("rust", include_str!("../kits/rust/spec.yaml")),
    ("codex-cli", include_str!("../kits/codex-cli/spec.yaml")),
    ("claude-cli", include_str!("../kits/claude-cli/spec.yaml")),
    ("pi-cli", include_str!("../kits/pi-cli/spec.yaml")),
    (
        "rocksdb-host",
        include_str!("../kits/rocksdb-host/spec.yaml"),
    ),
    (
        "vscode-remote",
        include_str!("../kits/vscode-remote/spec.yaml"),
    ),
];

pub(crate) const KIT_NAMES: [&str; 6] = [
    "rust",
    "codex-cli",
    "claude-cli",
    "pi-cli",
    "rocksdb-host",
    "vscode-remote",
];

pub fn materialize(kit_root: &Path) -> Result<(), String> {
    for (name, contents) in KITS {
        let directory = kit_root.join(name);
        let spec = directory.join("spec.yaml");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let current = fs::read_to_string(&spec).ok();
        if current.as_deref() != Some(contents) {
            fs::write(&spec, contents)
                .map_err(|error| format!("could not write {}: {error}", spec.display()))?;
        }
    }
    Ok(())
}
