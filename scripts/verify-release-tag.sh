#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd -P)
tag=${1:-${GITHUB_REF_NAME:-}}
expected_commit=${2:-${GITHUB_SHA:-}}
trusted_ref=${3:-refs/remotes/origin/master}

expected_primary_fingerprint=7159ADD2565AAF0837AB84C38AA61E99F1D01839
expected_signing_fingerprint=202B2E36BD365D4EB93C4C6A71B0843F284916BA
key_file="$root/.github/release-signing-key.asc"

fail() {
  printf 'release tag verification failed: %s\n' "$*" >&2
  exit 1
}

if [[ ! $tag =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  fail "tag ${tag:-<missing>} is not a supported semantic version"
fi
if [[ ! $expected_commit =~ ^[0-9a-fA-F]{40}$ ]]; then
  fail "expected commit ${expected_commit:-<missing>} is not a full Git object ID"
fi
if [[ ! -f $key_file || -L $key_file ]]; then
  fail "trusted public key is missing or is a symbolic link: $key_file"
fi

tag_ref="refs/tags/$tag"
if [[ $(git -C "$root" cat-file -t "$tag_ref" 2>/dev/null || true) != tag ]]; then
  fail "$tag_ref must be an annotated tag"
fi

tag_commit=$(git -C "$root" rev-parse "$tag_ref^{commit}")
if [[ $tag_commit != "$expected_commit" ]]; then
  fail "$tag_ref resolves to $tag_commit, expected $expected_commit"
fi
if ! git -C "$root" show-ref --verify --quiet "$trusted_ref"; then
  fail "trusted branch ref $trusted_ref is unavailable; fetch complete history first"
fi
if ! git -C "$root" merge-base --is-ancestor "$tag_commit" "$trusted_ref"; then
  fail "$tag_ref does not point to a commit reachable from $trusted_ref"
fi

verify_home=$(mktemp -d)
cleanup() {
  rm -rf -- "$verify_home"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
chmod 0700 "$verify_home"

GNUPGHOME=$verify_home gpg --batch --quiet --import "$key_file"
fingerprints=$(GNUPGHOME=$verify_home gpg --batch --with-colons --fingerprint --fingerprint)
if ! grep -Fqx "fpr:::::::::$expected_primary_fingerprint:" <<<"$fingerprints"; then
  fail "trusted key file does not contain primary fingerprint $expected_primary_fingerprint"
fi
if ! grep -Fqx "fpr:::::::::$expected_signing_fingerprint:" <<<"$fingerprints"; then
  fail "trusted key file does not contain signing fingerprint $expected_signing_fingerprint"
fi

if ! verify_output=$(GNUPGHOME=$verify_home git -C "$root" verify-tag --raw "$tag_ref" 2>&1); then
  printf '%s\n' "$verify_output" >&2
  fail "$tag_ref does not have a valid OpenPGP signature"
fi
valid_signature=$(awk '
  $1 == "[GNUPG:]" && $2 == "VALIDSIG" { print $3, $NF }
' <<<"$verify_output")
if [[ $valid_signature != "$expected_signing_fingerprint $expected_primary_fingerprint" ]]; then
  fail "$tag_ref was not signed by the allowed OpenPGP signing key"
fi

printf 'release tag: %s, commit %s, OpenPGP signer %s\n' \
  "$tag" "$tag_commit" "$expected_signing_fingerprint"
