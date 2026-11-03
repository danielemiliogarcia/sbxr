use std::fs;
use std::path::Path;

const KITS: [(&str, &str); 14] = [
    ("rust", include_str!("../kits/rust/spec.yaml")),
    ("claude-cli", include_str!("../kits/claude-cli/spec.yaml")),
    ("pi-cli", include_str!("../kits/pi-cli/spec.yaml")),
    ("paseo-cli", include_str!("../kits/paseo-cli/spec.yaml")),
    ("host-shell", include_str!("../kits/host-shell/spec.yaml")),
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
    (
        "github-proxy-codex",
        include_str!("../kits/github-proxy-codex/spec.yaml"),
    ),
    (
        "github-proxy-claude",
        include_str!("../kits/github-proxy-claude/spec.yaml"),
    ),
    ("ghcr", include_str!("../kits/ghcr/spec.yaml")),
    ("browser", include_str!("../kits/browser/spec.yaml")),
    ("orca-server", include_str!("../kits/orca-server/spec.yaml")),
];

const KIT_ASSETS: [(&str, &str, &[u8]); 8] = [
    (
        "claude-cli",
        "files/opt/sbxr-tools/claude/package.json",
        include_bytes!("../kits/claude-cli/files/opt/sbxr-tools/claude/package.json"),
    ),
    (
        "claude-cli",
        "files/opt/sbxr-tools/claude/package-lock.json",
        include_bytes!("../kits/claude-cli/files/opt/sbxr-tools/claude/package-lock.json"),
    ),
    (
        "pi-cli",
        "files/opt/sbxr-tools/pi/package.json",
        include_bytes!("../kits/pi-cli/files/opt/sbxr-tools/pi/package.json"),
    ),
    (
        "pi-cli",
        "files/opt/sbxr-tools/pi/package-lock.json",
        include_bytes!("../kits/pi-cli/files/opt/sbxr-tools/pi/package-lock.json"),
    ),
    (
        "paseo-cli",
        "files/opt/sbxr-tools/paseo/package.json",
        include_bytes!("../kits/paseo-cli/files/opt/sbxr-tools/paseo/package.json"),
    ),
    (
        "paseo-cli",
        "files/opt/sbxr-tools/paseo/package-lock.json",
        include_bytes!("../kits/paseo-cli/files/opt/sbxr-tools/paseo/package-lock.json"),
    ),
    (
        "browser",
        "files/opt/sbxr-tools/browser/package.json",
        include_bytes!("../kits/browser/files/opt/sbxr-tools/browser/package.json"),
    ),
    (
        "browser",
        "files/opt/sbxr-tools/browser/package-lock.json",
        include_bytes!("../kits/browser/files/opt/sbxr-tools/browser/package-lock.json"),
    ),
];

pub(crate) const KIT_NAMES: [&str; 14] = [
    "rust",
    "claude-cli",
    "pi-cli",
    "paseo-cli",
    "host-shell",
    "rocksdb-host",
    "vscode-remote",
    "git-ssh-sign",
    "github-ssh",
    "github-proxy-codex",
    "github-proxy-claude",
    "ghcr",
    "browser",
    "orca-server",
];

pub(crate) const KIT_ASSET_PATHS: [(&str, &str); 8] = [
    ("claude-cli", "files/opt/sbxr-tools/claude/package.json"),
    (
        "claude-cli",
        "files/opt/sbxr-tools/claude/package-lock.json",
    ),
    ("pi-cli", "files/opt/sbxr-tools/pi/package.json"),
    ("pi-cli", "files/opt/sbxr-tools/pi/package-lock.json"),
    ("paseo-cli", "files/opt/sbxr-tools/paseo/package.json"),
    ("paseo-cli", "files/opt/sbxr-tools/paseo/package-lock.json"),
    ("browser", "files/opt/sbxr-tools/browser/package.json"),
    ("browser", "files/opt/sbxr-tools/browser/package-lock.json"),
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
    for (kit, relative, contents) in KIT_ASSETS {
        let path = kit_root.join(kit).join(relative);
        let directory = path
            .parent()
            .ok_or_else(|| format!("embedded kit asset has no parent: {}", path.display()))?;
        fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        if fs::read(&path).ok().as_deref() != Some(contents) {
            fs::write(&path, contents)
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        }
    }
    Ok(())
}
