use crate::cli::Preset;
use crate::{embedded, process, sha256};
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

pub(crate) struct Context {
    pub(crate) kit_root: PathBuf,
    pub(crate) preserve_xdg_state: bool,
}

impl Context {
    pub(crate) fn discover() -> Result<Self, String> {
        let explicit_kit_root = env::var_os("SBXR_KIT_ROOT").map(PathBuf::from);
        let data_root = env::var_os("SBXR_ROOT")
            .map(PathBuf::from)
            .unwrap_or(default_data_root()?);
        let kit_root = explicit_kit_root
            .clone()
            .unwrap_or_else(|| data_root.join("kits"));
        if explicit_kit_root.is_none() {
            embedded::materialize(&kit_root)?;
        }
        Ok(Self {
            kit_root,
            preserve_xdg_state: env::var("SBXR_PRESERVE_XDG_STATE_HOME").as_deref() == Ok("1"),
        })
    }
}

fn default_data_root() -> Result<PathBuf, String> {
    if let Some(root) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(root).join("sbxr"));
    }
    if cfg!(target_os = "windows") {
        if let Some(root) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(root).join("sbxr"));
        }
    }
    let home = process::home_dir()?;
    if cfg!(target_os = "macos") {
        Ok(home.join("Library/Application Support").join("sbxr"))
    } else {
        Ok(home.join(".local/share/sbxr"))
    }
}

pub(crate) struct RocksDbHost {
    pub(crate) prefix: PathBuf,
    pub(crate) library_dir: String,
    pub(crate) include_dir: String,
    pub(crate) mount_argument: std::ffi::OsString,
}

impl RocksDbHost {
    pub(crate) fn resolve(requested: Option<PathBuf>) -> Result<Option<Self>, String> {
        let Some(requested) = requested else {
            return Ok(None);
        };
        if !cfg!(target_os = "linux") {
            return Err("--rocksdb-host currently requires a Linux host".to_owned());
        }
        let prefix = requested.canonicalize().map_err(|error| {
            format!(
                "RocksDB host prefix {} is unavailable: {error}",
                requested.display()
            )
        })?;
        if !prefix.is_dir() || prefix == Path::new("/") {
            return Err(format!(
                "RocksDB host prefix must be a non-root directory: {}",
                prefix.display()
            ));
        }
        let header = prefix.join("include/rocksdb/c.h");
        if !header.is_file() {
            return Err(format!("missing RocksDB C header: {}", header.display()));
        }
        let library_link = prefix.join("lib/librocksdb.so");
        let library = library_link.canonicalize().map_err(|error| {
            format!(
                "missing RocksDB shared library {}: {error}",
                library_link.display()
            )
        })?;
        if !library.starts_with(&prefix) || !library.is_file() {
            return Err(format!(
                "RocksDB library symlink escapes the mounted prefix: {}",
                library.display()
            ));
        }
        let prefix_text = prefix
            .to_str()
            .ok_or("RocksDB host prefix must be valid UTF-8")?;
        Ok(Some(Self {
            library_dir: format!("{prefix_text}/lib"),
            include_dir: format!("{prefix_text}/include"),
            mount_argument: format!("{prefix_text}:ro").into(),
            prefix,
        }))
    }
}

pub(crate) struct Project {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) review_name: String,
}

impl Project {
    pub(crate) fn resolve(
        requested: &Path,
        preset: Preset,
        rocksdb: Option<&RocksDbHost>,
    ) -> Result<Self, String> {
        let path = requested.canonicalize().map_err(|error| {
            format!(
                "project path {} is unavailable: {error}",
                requested.display()
            )
        })?;
        if !path.is_dir() {
            return Err(format!(
                "project path is not a directory: {}",
                path.display()
            ));
        }
        let base = sandbox_component(path.file_name().unwrap_or_else(|| OsStr::new("project")));
        #[cfg(unix)]
        let mut identity = path.as_os_str().as_bytes().to_vec();
        #[cfg(not(unix))]
        let mut identity = path.to_string_lossy().as_bytes().to_vec();
        if let Some(rocksdb) = rocksdb {
            identity.push(0);
            #[cfg(unix)]
            identity.extend_from_slice(rocksdb.prefix.as_os_str().as_bytes());
            #[cfg(not(unix))]
            identity.extend_from_slice(rocksdb.prefix.to_string_lossy().as_bytes());
        }
        let hash = sha256::hex_prefix(&identity, 4);
        let feature = if rocksdb.is_some() { "-rdb" } else { "" };
        let generated = format!("rust-{}{feature}-{base}-{hash}", preset.slug());
        let name = env::var("SBXR_NAME").unwrap_or(generated);
        Ok(Self {
            review_name: format!("rust-review{feature}-{base}-{hash}"),
            path,
            name,
        })
    }
}

fn sandbox_component(value: &OsStr) -> String {
    sanitize_component(&value.to_string_lossy(), '-', true, 24, "project")
}

pub(crate) fn crate_name(path: &Path) -> String {
    let mut name = sanitize_component(
        &path
            .file_name()
            .unwrap_or_else(|| OsStr::new("rust_app"))
            .to_string_lossy(),
        '_',
        false,
        usize::MAX,
        "rust_app",
    );
    if name.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        name.insert_str(0, "app_");
    }
    name
}

fn sanitize_component(
    input: &str,
    replacement: char,
    allow_dot: bool,
    max_len: usize,
    fallback: &str,
) -> String {
    let mut output = String::new();
    let mut replaced = false;
    for character in input.chars().flat_map(char::to_lowercase) {
        let allowed = character.is_ascii_alphanumeric()
            || character == '-'
            || character == '_'
            || (allow_dot && character == '.');
        if allowed {
            output.push(character);
            replaced = false;
        } else if !replaced {
            output.push(replacement);
            replaced = true;
        }
    }
    output = output
        .trim_matches(['-', '_', '.'])
        .chars()
        .take(max_len)
        .collect();
    output = output.trim_end_matches(['-', '_', '.']).to_owned();
    if output.is_empty() {
        fallback.to_owned()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sandbox_component(OsStr::new("My Rust App!")), "my-rust-app");
        assert_eq!(sandbox_component(OsStr::new("...")), "project");
    }
}
