#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 ARCHIVE" >&2
  exit 2
fi

archive=$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")
if [[ ! -f "$archive" ]]; then
  echo "archive is missing: $archive" >&2
  exit 1
fi

directory=$(dirname "$archive")
name=$(basename "$archive")

if command -v sha256sum >/dev/null 2>&1; then
  digest=$(sha256sum "$archive" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  digest=$(shasum -a 256 "$archive" | awk '{print $1}')
elif command -v openssl >/dev/null 2>&1; then
  digest=$(openssl dgst -sha256 "$archive" | awk '{print $NF}')
else
  echo "sha256sum, shasum, or openssl is required" >&2
  exit 1
fi

printf '%s  %s\n' "$digest" "$name" > "$directory/SHA256SUMS"
