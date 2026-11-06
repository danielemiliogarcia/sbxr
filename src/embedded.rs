use std::fs;
use std::path::Path;

const KITS: [(&str, &str); 16] = [
    ("rust", include_str!("../kits/rust/spec.yaml")),
    (
        "node-runtime",
        include_str!("../kits/node-runtime/spec.yaml"),
    ),
    ("pi-cli", include_str!("../kits/pi-cli/spec.yaml")),
    ("paseo-cli", include_str!("../kits/paseo-cli/spec.yaml")),
    ("host-shell", include_str!("../kits/host-shell/spec.yaml")),
    ("bash-shell", include_str!("../kits/bash-shell/spec.yaml")),
    ("zsh-shell", include_str!("../kits/zsh-shell/spec.yaml")),
    ("fish-shell", include_str!("../kits/fish-shell/spec.yaml")),
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
        "github-proxy",
        include_str!("../kits/github-proxy/spec.yaml"),
    ),
    ("ghcr", include_str!("../kits/ghcr/spec.yaml")),
    ("browser", include_str!("../kits/browser/spec.yaml")),
    ("orca-server", include_str!("../kits/orca-server/spec.yaml")),
];

const KIT_ASSETS: [(&str, &str, &[u8]); 6] = [
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

pub(crate) const KIT_NAMES: [&str; 16] = [
    "rust",
    "node-runtime",
    "pi-cli",
    "paseo-cli",
    "host-shell",
    "bash-shell",
    "zsh-shell",
    "fish-shell",
    "rocksdb-host",
    "vscode-remote",
    "git-ssh-sign",
    "github-ssh",
    "github-proxy",
    "ghcr",
    "browser",
    "orca-server",
];

pub(crate) const KIT_ASSET_PATHS: [(&str, &str); 6] = [
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_kit_uses_the_v3_mixin_contract() {
        for (name, descriptor) in KITS {
            assert!(
                descriptor.starts_with("# syntax=docker/sandbox-kit:3\n"),
                "{name} has no Kits v3 frontend"
            );
            assert!(
                descriptor.contains("schemaVersion: \"3\"\n"),
                "{name} has no v3 schema"
            );
            assert!(
                descriptor.contains("kind: mixin\n"),
                "{name} is not a v3 mixin"
            );
            assert!(
                !descriptor.lines().any(|line| line.starts_with("name:")),
                "{name} retained the removed v2 top-level name"
            );
        }
    }

    #[test]
    fn rust_analyzer_ignores_nested_project_toolchain_overrides() {
        let rust = KITS
            .iter()
            .find_map(|(name, descriptor)| (*name == "rust").then_some(*descriptor))
            .unwrap();
        assert!(
            rust.contains("exec /home/agent/.cargo/bin/rustup run 1.99.1 rust-analyzer \"$@\"")
        );
        assert!(!rust.contains("rust-analyzer rustc rustdoc rustfmt rustup; do"));
    }
}
