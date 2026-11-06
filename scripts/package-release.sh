#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 TARGET" >&2
  exit 2
fi

target=$1
root=$(cd "$(dirname "$0")/.." && pwd -P)
version=$(tr -d '[:space:]' < "$root/VERSION")
release_rust=$(tr -d '[:space:]' < "$root/RUST_RELEASE_VERSION")

case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|aarch64-apple-darwin)
    binary=sbxr
    extension=tar.gz
    ;;
  x86_64-pc-windows-msvc)
    binary=sbxr.exe
    extension=zip
    ;;
  *)
    echo "unsupported release target: $target" >&2
    exit 2
    ;;
esac

source_binary="$root/target/$target/release/$binary"
if [[ ! -f "$source_binary" ]]; then
  echo "release binary is missing: $source_binary" >&2
  exit 1
fi

case "$version" in
  *[!0-9A-Za-z.+-]*|'')
    echo "VERSION contains unsafe archive-name characters" >&2
    exit 1
    ;;
esac

name="sbxr-$version-$target"
archive="$name.$extension"
output="$root/target/distrib"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/sbxr-package.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT

mkdir -p "$temporary/$name/THIRD_PARTY_LICENSES" "$output"
cp "$source_binary" "$temporary/$name/$binary"
cp \
  "$root/LICENSE" \
  "$root/README.md" \
  "$root/SECURITY.md" \
  "$root/CREDENTIAL_ARCHITECTURE.md" \
  "$root/THIRD_PARTY_REVIEW.md" \
  "$temporary/$name/"
license_count=0
for package_dir in "$root"/vendor/*; do
  [[ -d "$package_dir" ]] || continue
  package=$(basename "$package_dir")
  for license in \
    "$package_dir"/LICENSE* "$package_dir"/license* \
    "$package_dir"/COPYING* "$package_dir"/copying* \
    "$package_dir"/UNLICENSE* "$package_dir"/unlicense*
  do
    [[ -f "$license" ]] || continue
    file=$(basename "$license")
    cp "$license" "$temporary/$name/THIRD_PARTY_LICENSES/$package-$file"
    ((license_count += 1))
  done
done
if ((license_count == 0)); then
  echo "no vendored dependency license files were found" >&2
  exit 1
fi

source_commit=${GITHUB_SHA:-}
if [[ -z "$source_commit" ]] && command -v git >/dev/null 2>&1; then
  source_commit=$(git -C "$root" rev-parse HEAD 2>/dev/null || true)
  if [[ -n "$source_commit" ]] && [[ -n $(git -C "$root" status --porcelain 2>/dev/null) ]]; then
    source_commit="$source_commit-dirty"
  fi
fi
cat > "$temporary/$name/RELEASE-MANIFEST.txt" <<EOF
product=sbxr
version=$version
target=$target
rust=$release_rust
rustflags=${SBXR_RELEASE_RUSTFLAGS:-}
source_commit=$source_commit
EOF

rm -f -- "$output/$archive"
if [[ "$extension" == zip ]]; then
  if ! command -v 7z >/dev/null 2>&1; then
    echo "7z is required to package the Windows release" >&2
    exit 1
  fi
  (
    cd "$temporary"
    7z a -bd -tzip "$output/$archive" "$name" >/dev/null
  )
else
  tar -C "$temporary" -czf "$output/$archive" "$name"
fi

printf '%s\n' "$output/$archive"
