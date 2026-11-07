#!/bin/sh
set -eu

repository=danielemiliogarcia/sbxr
workflow="$repository/.github/workflows/release.yml"
version=${SBXR_VERSION:-${1:-}}
install_dir=${SBXR_INSTALL_DIR:-"$HOME/.local/bin"}
release_dir=${SBXR_RELEASE_DIR:-}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

if [ -z "$version" ]; then
  if [ -n "$release_dir" ]; then
    fail "set SBXR_VERSION when using SBXR_RELEASE_DIR"
  fi
  command -v gh >/dev/null 2>&1 || fail "GitHub CLI (gh) is required to discover, download, and verify the latest release"
  if ! version=$(gh api "repos/$repository/releases?per_page=100" \
    --jq '[.[] | select(.draft == false and .prerelease == false and any(.assets[]; .name == "install.sh"))][0].tag_name'); then
    fail "could not discover the latest sbxr release"
  fi
  [ -n "$version" ] || fail "no sbxr release containing install.sh was found"
fi
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$'; then
  fail "invalid sbxr version: $version"
fi
case "$install_dir" in
  /*) ;;
  *) fail "SBXR_INSTALL_DIR must be an absolute path" ;;
esac

case "$(uname -s 2>/dev/null || true):$(uname -m 2>/dev/null || true)" in
  Linux:x86_64|Linux:amd64)
    target=x86_64-unknown-linux-gnu
    ;;
  Linux:aarch64|Linux:arm64)
    target=aarch64-unknown-linux-gnu
    ;;
  Darwin:arm64|Darwin:aarch64)
    target=aarch64-apple-darwin
    ;;
  *)
    fail "this installer supports Linux x86-64/ARM64 and Apple-silicon macOS"
    ;;
esac

name="sbxr-$version-$target"
archive="$name.tar.gz"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/sbxr-install.XXXXXX")
temporary_destination=
cleanup() {
  rm -rf -- "$temporary"
  if [ -n "$temporary_destination" ]; then
    rm -f -- "$temporary_destination"
  fi
}
trap cleanup EXIT HUP INT TERM

if [ -n "$release_dir" ]; then
  case "$release_dir" in
    /*) ;;
    *) fail "SBXR_RELEASE_DIR must be an absolute path" ;;
  esac
  printf '%s\n' "warning: local release mode; GitHub provenance verification is skipped" >&2
  cp "$release_dir/$archive" "$temporary/$archive"
  cp "$release_dir/SHA256SUMS" "$temporary/SHA256SUMS"
else
  command -v gh >/dev/null 2>&1 || fail "GitHub CLI (gh) is required to download and verify release provenance"
  gh attestation verify --help >/dev/null 2>&1 || fail "update GitHub CLI (gh): this installer requires 'gh attestation verify' support"
  gh release download "$version" \
    --repo "$repository" \
    --pattern "$archive" \
    --pattern SHA256SUMS \
    --dir "$temporary"
  gh attestation verify "$temporary/$archive" \
    --repo "$repository" \
    --signer-workflow "$workflow" \
    --source-ref "refs/tags/$version" \
    --deny-self-hosted-runners >/dev/null
fi

expected=$(awk -v archive="$archive" '$2 == archive { print $1 }' "$temporary/SHA256SUMS")
case "$expected" in
  ''|*[!0-9A-Fa-f]*) fail "SHA256SUMS has no valid entry for $archive" ;;
esac
if [ "$(printf '%s' "$expected" | wc -c | tr -d ' ')" -ne 64 ]; then
  fail "SHA256SUMS has an invalid digest for $archive"
fi

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$temporary/$archive" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$temporary/$archive" | awk '{print $1}')
elif command -v openssl >/dev/null 2>&1; then
  actual=$(openssl dgst -sha256 "$temporary/$archive" | awk '{print $NF}')
else
  fail "sha256sum, shasum, or openssl is required to verify the archive"
fi
if [ "$(printf '%s' "$actual" | tr 'A-F' 'a-f')" != "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" ]; then
  fail "SHA-256 verification failed for $archive"
fi

tar -xzf "$temporary/$archive" -C "$temporary"
candidate="$temporary/$name/sbxr"
[ -f "$candidate" ] || fail "release archive does not contain $name/sbxr"
chmod 0755 "$candidate"
reported=$("$candidate" --version)
[ "$reported" = "sbxr $version" ] || fail "release binary reports '$reported', expected 'sbxr $version'"

mkdir -p "$install_dir"
destination="$install_dir/sbxr"
[ ! -L "$destination" ] || fail "refusing to replace symlinked destination: $destination"
temporary_destination="$install_dir/.sbxr.install.$$"
cp "$candidate" "$temporary_destination"
chmod 0755 "$temporary_destination"
mv -f "$temporary_destination" "$destination"
temporary_destination=

printf 'installed %s to %s\n' "$reported" "$destination"
case ":${PATH:-}:" in
  *":$install_dir:"*) ;;
  *) printf 'note: add %s to PATH\n' "$install_dir" ;;
esac
