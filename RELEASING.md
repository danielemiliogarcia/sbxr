# Releasing sbxr

Tagged releases publish native `sbxr` binaries so users do not need Rust on the
host. Docker Sandboxes remains a separately installed prerequisite. The release
workflow builds natively on GitHub-hosted runners, smoke-tests each binary and
installer, creates checksums and build-provenance attestations, and publishes
the GitHub Release only after every target succeeds.

## Compiler policy

`RUST_RELEASE_VERSION` pins the exact stable compiler used for official
artifacts. It should name the newest stable Rust version reviewed for the
release; it is currently 1.99.1. Pinning the patch version prevents the
compiler input from silently changing when rebuilding a tag instead of allowing
the `stable` channel to move later. This is controlled provenance, not a claim
that archives are bit-for-bit reproducible across runner images.

`Cargo.toml`'s `rust-version = "1.85"` is a different contract: it is the
minimum supported compiler for people building from source. CI checks both.
Updating the release compiler does not justify raising the MSRV. Raise the MSRV
only when sbxr deliberately adopts a newer language or standard-library
feature, and document that compatibility change.

## Release targets

| Host | Rust target | Archive |
|---|---|---|
| Ubuntu 24.04 x86-64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Ubuntu 24.04 ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS 15 Apple silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows 11 x86-64 | `x86_64-pc-windows-msvc` | `.zip` |

The native build confirms compilation and installer behavior. It does not by
itself promote an “Expected” host to “Tested” in `PLATFORM_SUPPORT.md`; that
requires the documented end-to-end sandbox, authentication, editor, and agent
checklist. The Windows artifact statically links its MSVC C runtime so users do
not need a separate Visual C++ redistributable. macOS notarization and Windows
Authenticode signing are not yet configured.

## What is published

Each target archive contains:

```text
sbxr[.exe]
LICENSE
README.md
SECURITY.md
CREDENTIAL_ARCHITECTURE.md
THIRD_PARTY_REVIEW.md
THIRD_PARTY_LICENSES/
RELEASE-MANIFEST.txt
```

The release also contains `install.sh`, `install.ps1`, and `SHA256SUMS`. Every
published file receives a GitHub build-provenance attestation. The installers
require an exact version, download only the matching archive, verify its
repository/workflow/tag attestation and SHA-256, confirm the embedded version,
and refuse to replace a symlink or Windows reparse-point destination.

## Prepare a release

1. Update `Cargo.toml`, `Cargo.lock`, and `VERSION` to the same version.
2. Move the release notes from `Unreleased` to a matching
   `## [VERSION] - YYYY-MM-DD` heading in `CHANGELOG.md`.
3. Check the current stable Rust release. Update `RUST_RELEASE_VERSION` only
   after reviewing and testing that compiler.
4. Review all vendored dependency and embedded-kit changes.
5. Run the local gates:

   ```bash
   rust_version=$(cat RUST_RELEASE_VERSION)
   rustup toolchain install "$rust_version" --profile minimal --component clippy,rustfmt
   ./scripts/verify-release-metadata.sh
   cargo +"$rust_version" fmt --all -- --check
   cargo +"$rust_version" test --all-targets --offline --frozen
   cargo +"$rust_version" clippy --all-targets --all-features --offline --frozen -- -D warnings
   cargo +1.85.0 check --all-targets --offline --frozen
   cargo +"$rust_version" build --release --target x86_64-unknown-linux-gnu --offline --frozen
   ```

6. Exercise the local Linux archive and installer without network access:

   ```bash
   archive=$(./scripts/package-release.sh x86_64-unknown-linux-gnu)
   ./scripts/write-release-checksum.sh "$archive"
   test_dir=$(mktemp -d)
   SBXR_VERSION=$(cat VERSION) \
   SBXR_RELEASE_DIR="$PWD/target/distrib" \
   SBXR_INSTALL_DIR="$test_dir/bin" \
     ./install.sh
   "$test_dir/bin/sbxr" --version
   ```

   `SBXR_RELEASE_DIR` exists only for local/CI smoke tests. It prints a warning
   and skips GitHub provenance because unpublished local files cannot have a
   release attestation. Normal user installation never sets it.

7. Push the branch and let normal CI pass. Manually dispatch the `Release`
   workflow once if the complete four-platform artifact matrix should be tested
   before tagging. Manual dispatch uploads workflow artifacts but never creates
   a GitHub Release.

## Publish

Review and merge the release commit, then create an annotated tag matching
`VERSION`:

```bash
version=$(cat VERSION)
git tag -a "$version" -m "sbxr $version"
git push origin "$version"
```

The tag triggers `.github/workflows/release.yml`. It fails closed when the tag,
Cargo package, `VERSION`, or changelog heading disagree. The publishing job has
the only `contents: write`, `id-token: write`, and `attestations: write`
permissions; checkout credentials are never persisted. External actions are
pinned to immutable commit hashes.

After publication, verify at least one archive independently:

```bash
version=$(cat VERSION)
gh release download "$version" --repo danielemiliogarcia/sbxr \
  --pattern 'sbxr-*' --pattern SHA256SUMS
gh attestation verify sbxr-$version-x86_64-unknown-linux-gnu.tar.gz \
  --repo danielemiliogarcia/sbxr \
  --signer-workflow danielemiliogarcia/sbxr/.github/workflows/release.yml \
  --source-ref "refs/tags/$version" --deny-self-hosted-runners
sha256sum --check --ignore-missing SHA256SUMS
```

Checksums detect corruption and attestations identify the producing
repository, workflow, and tag. Neither proves the source itself safe. Keep the
reviewed-source installation documented as an alternative trust path.
