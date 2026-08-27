use std::fs;
use std::path::Path;

const KITS: [(&str, &str); 7] = [
    ("rust", include_str!("../kits/rust/spec.yaml")),
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
    (
        "git-ssh-sign",
        include_str!("../kits/git-ssh-sign/spec.yaml"),
    ),
    ("github-ssh", include_str!("../kits/github-ssh/spec.yaml")),
];

pub(crate) const KIT_NAMES: [&str; 7] = [
    "rust",
    "claude-cli",
    "pi-cli",
    "rocksdb-host",
    "vscode-remote",
    "git-ssh-sign",
    "github-ssh",
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
