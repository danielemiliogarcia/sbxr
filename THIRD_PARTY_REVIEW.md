# Third-party dependency review

Review date: 2026-08-24

This document records the dependency intake review for `sbxr` 0.1.0. It
is evidence of a version-specific review, not a guarantee that third-party
code is vulnerability-free or non-malicious.

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
