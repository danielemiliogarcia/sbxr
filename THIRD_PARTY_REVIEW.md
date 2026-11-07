# Third-party dependency review

Review date: 2026-11-04

This document records the dependency intake review applicable to `sbxr` 0.3.8.
It is evidence of a version-specific review, not a guarantee that third-party
code is vulnerability-free or non-malicious.

## Sandbox Node toolchains

Review update: 2026-11-04

Node.js 22.23.4 is downloaded from the official Node.js release archive and
verified against committed SHA-256 digests for Linux x86-64 and arm64. Pi
1.4.0 and Paseo 0.8.0 each have an independent, committed npm package lock
under their kit. Keeping the locks separate avoids installing agents excluded
by a selected preset. Sandbox provisioning uses
`npm ci --ignore-scripts --no-audit --no-fund`.

Claude is no longer installed from npm by sbxr. The Claude-only preset uses
Docker's official Claude Kits v3 workload and the multi-agent preset uses its
official Claude mixin. Those OCI artifacts, along with the official Codex and
Shell workloads, are pinned by manifest-list digest in `src/envfile.rs`; an
upgrade is therefore an explicit reviewed source change rather than a mutable
tag resolution during sandbox creation.

| Official Docker kit | Pinned version | Manifest-list digest |
|---|---:|---|
| Codex workload | 0.166.0 | `sha256:b41daf2cd2837c53d3e6932a9460761dd737d1b1b4f5978670adf496c5a1b30c` |
| Claude workload | 2.1.317 | `sha256:d2a9f6ec91ecb0d8e2a92fe060abde55ff24f5f3d1644600965060e03d0cbf5d` |
| Claude mixin | 2.1.267 | `sha256:e4d28ab7d38d64e9a205433cde85111af67e55e0f6f84748ff2d20bfa9fe2a22` |
| Shell workload | 1.0.3 | `sha256:6289eaec55acb89d0d5bfc46b2a5870a61cd2252d270621adf2fe7a0e139ab83` |

Pi expects its OpenAI OAuth access value to be a JWT so it can derive an
account ID. Docker deliberately exposes a non-secret proxy sentinel instead.
After the integrity-checked install, sbxr applies a narrow, fail-closed patch
to Pi's Codex provider that maps only the exact Docker sentinel to the fixed
`proxy-managed` account ID. The bootstrap fails if both reviewed patch targets
are not found; no general token parsing or endpoint behavior is changed.

Fresh resolution exposed vulnerable transitive Pi and Paseo releases. The
reviewed manifests constrain Pi's `undici` to 8.12.2 and `minimatch` to 11.0.1,
and constrain Paseo's MCP SDK, AI SDK, Markdown parser, and UUID dependencies;
the lock selects the non-vulnerable Hono server release permitted by Paseo's
range. `npm audit --package-lock-only --omit=dev` reported zero vulnerabilities
for both final locks. Clean temporary `npm ci --ignore-scripts` installs
reported the expected CLI versions. `npm audit signatures` verified registry
signatures for all 122 Pi and 281 Paseo packages; Pi reported 37
attestations and Paseo 42. Registry signatures establish package identity, not
benign behavior, so every future lockfile diff and override remains subject to
review.

The optional browser kit adds a separate three-package lock: Playwright MCP
0.0.83 and exact Playwright/Playwright Core
1.64.0-alpha-1790635538000. The recorded npm SHA-512 integrity values match the
published registry artifacts. Installation uses `npm ci --ignore-scripts`;
system libraries and Chromium are installed by explicit reviewed commands.

The GitHub proxy bootstrap downloads GitHub CLI 2.102.0 and verifies the
published archive with architecture-specific SHA-256 values before installing
only `gh`. The optional Orca bootstrap downloads release 1.4.247's x64 or arm64
AppImage and verifies the release manifest SHA-512 before extraction. Its
headless launcher uses the guest distribution's Xvfb and `xauth`; an existing
AppImage is reused when those prerequisites need repair. Neither artifact is
executed on the host: both are downloaded and verified inside the microVM.
Versions, URLs, checksums, and installation logic are environment contract
inputs.

## Resolution and delivery

- The only direct dependency is exactly `serde_json = "=1.0.151"`.
- `Cargo.lock` fixes every package version and crates.io package checksum.
- `.cargo/config.toml` replaces crates.io with the checked-in `vendor/`
  directory for path builds.
- `cargo install --frozen --path .` therefore resolves from the lockfile and
  compiles without registry or network access.
- `cargo install --locked` remains supported when a developer intentionally
  chooses a registry-backed source installation.

The vendor tree is the project's answer to the bootstrap chicken-and-egg
problem. `sbxr` exists to move Cargo builds into isolation, so installing
it should not require first resolving and downloading unreviewed build inputs
onto the host. The checked-in sources make that first build inspectable and
offline. They move trust to the reviewed repository revision; they do not
eliminate the need to review the code that Cargo compiles and executes.

The active external build graph is:

```text
serde_json 1.0.151
├── itoa 1.0.18
├── memchr 2.8.3
├── serde_core 1.0.229
└── zmij 1.0.23
```

Cargo vendors optional packages represented in the lockfile as well. Those
packages are retained so the vendored source is complete, but they are not
compiled by the current feature selection.

## Package checksums

The `package` value in every vendored `.cargo-checksum.json` was compared with
the corresponding SHA-256 checksum in `Cargo.lock`.

| Package | SHA-256 |
|---|---|
| `itoa 1.0.18` | `8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682` |
| `memchr 2.8.3` | `cf8baf1c55e62ffcace7a9f06f4bd9cd3f0c4beb022d3b367256b91b87513d98` |
| `proc-macro2 1.0.107` | `985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9` |
| `quote 1.0.47` | `1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001` |
| `serde 1.0.229` | `4148590afebada386688f18773da617792bf2ef03ffc1e4cbd2b1d45b023e0ba` |
| `serde_core 1.0.229` | `67dca2c9c51e58a4791a4b1ed58308b39c64224d349a935ab5039aa360942a48` |
| `serde_derive 1.0.229` | `e7a5d71263a5a7d47b41f6b3f06ba276f10cc18b0931f1799f710578e2309348` |
| `serde_json 1.0.151` | `c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14` |
| `syn 3.0.4` | `e6275cddf4610d1775e6d1fe9469b2e77d0f39fd98fb7450901b821e0c53649f` |
| `unicode-ident 1.0.24` | `e6e4313cd5fcd3dad5cafa179702e2b244f760991f45397d14d4ebf38247da75` |
| `zmij 1.0.23` | `29666d0abbfad1e3dc4dcf6144730dd3a3ab225bbbdac83319345b1b44ccfc1b` |

## Executed-code review

The current feature graph executes no procedural macro. Its build-script
surface was inspected in full:

| Active build script | Observed behavior |
|---|---|
| `serde_core 1.0.229` | Reads Cargo target/version variables, invokes `rustc --version`, emits `cfg` values, and writes `private.rs` only below `OUT_DIR`. |
| `serde_json 1.0.151` | Reads target architecture and pointer width and emits an arithmetic-width `cfg`. |
| `zmij 1.0.23` | Reads Cargo optimization/version variables, invokes `rustc --version`, and emits `cfg` values. |

No active build script contains network access, a shell invocation, package
installation, or writes outside Cargo's output directory. The vendored but
inactive `proc-macro2`, `quote`, and `serde` build scripts and the inactive
`serde_derive` procedural macro were also identified so that a future feature
change cannot make them active unnoticed in dependency-tree review.

All vendored packages declare permissive licenses: MIT, Apache-2.0, Unlicense,
or Unicode-3.0 combinations. The active libraries contain `unsafe` code,
including optimized numeric and byte-search implementations. That source was
not subject to a formal line-by-line security audit in this review.

## Required update procedure

For every dependency update:

1. change the exact direct version intentionally;
2. update and review `Cargo.lock`;
3. regenerate with `cargo vendor --locked --versioned-dirs vendor`;
4. review the vendor diff, active dependency tree, build scripts, procedural
   macros, unsafe-code changes, licenses, and checksums;
5. update this document with the new versions, findings, reviewer, and date;
6. validate with `cargo test --frozen`, strict Clippy, and a fresh
   `cargo install --frozen --path .`.

Do not accept a dependency-only pull request merely because its checksums are
valid. Checksums establish source identity, not source trustworthiness.
