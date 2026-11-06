#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd -P)
version=$(tr -d '[:space:]' < "$root/VERSION")
release_rust=$(tr -d '[:space:]' < "$root/RUST_RELEASE_VERSION")

if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$'; then
  echo "VERSION is not a supported semantic version: $version" >&2
  exit 1
fi
if ! printf '%s\n' "$release_rust" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "RUST_RELEASE_VERSION is not an exact stable Rust version: $release_rust" >&2
  exit 1
fi

cargo_version=$(awk '
  /^\[package\]$/ { package = 1; next }
  /^\[/ { package = 0 }
  package && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
' "$root/Cargo.toml")
if [[ "$cargo_version" != "$version" ]]; then
  echo "Cargo.toml version $cargo_version does not match VERSION $version" >&2
  exit 1
fi

if ! grep -Fq "## [$version]" "$root/CHANGELOG.md"; then
  echo "CHANGELOG.md has no release heading for $version" >&2
  exit 1
fi

if [[ ${GITHUB_REF_TYPE:-} == tag ]]; then
  expected="$version"
  if [[ ${GITHUB_REF_NAME:-} != "$expected" ]]; then
    echo "release tag ${GITHUB_REF_NAME:-<missing>} does not match $expected" >&2
    exit 1
  fi
fi

printf 'release metadata: sbxr %s, Rust %s\n' "$version" "$release_rust"
