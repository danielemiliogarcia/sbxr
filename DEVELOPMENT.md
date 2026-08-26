# Developing sbxr

This guide is for developers who want to build `sbxr`, change its behavior, or
add a feature. For installation and first use, start with the
[README](README.md). For security assumptions, read [SECURITY.md](SECURITY.md)
before changing authentication, configuration mirroring, dependency handling,
network permissions, or sandbox mounts.

## Prerequisites

- a reviewed `sbxr` checkout;
- Rust 1.85 or newer, including Cargo;
- the checked-in `.cargo/config.toml`, `Cargo.lock`, and `vendor/` directory;
- Docker Sandboxes (`sbx`) only for host diagnostics and live sandbox tests.

The normal build and unit-test workflow is offline. Docker, `sbx`, KVM, agent
subscriptions, and VS Code are not required merely to compile the binary.

## Build, run, and install

Open a shell in the repository:

```bash
cd /path/to/sbxr
```

Build a development binary:

```bash
cargo build --frozen
./target/debug/sbxr help
```

`cargo build` compiles but does not install the command. Build an optimized
binary without installing it with:

```bash
cargo build --release --frozen
./target/release/sbxr help
```

For quick development runs, Cargo can build and execute in one command. The
second `--` separates Cargo's options from `sbxr` arguments:

```bash
cargo run --frozen -- name .
cargo run --frozen -- help
```

Install the optimized binary into Cargo's binary directory, normally
`~/.cargo/bin`:

```bash
cargo install --frozen --path . --force
sbxr help
```

`cargo install` performs its own optimized build, so running `cargo build`
first is unnecessary. `--force` replaces an existing `sbxr` installation.

`--frozen` combines locked dependency resolution with disabled network access.
The repository's Cargo configuration resolves those dependencies from the
reviewed `vendor/` tree. Do not remove `--frozen` merely to make a build pass;
investigate inconsistent lockfile or vendor state instead.

## Architecture

The binary is deliberately split by responsibility:

| Module | Responsibility |
|---|---|
| `main.rs` | Minimal process entrypoint and module wiring |
| `app.rs` | Top-level command dispatch and workflow coordination |
| `cli.rs` | Argument parsing, presets, actions, and help text |
| `environment.rs` | Project identity, deterministic names, data paths, and RocksDB validation |
| `git.rs` | Sandbox Git tooling, identity/signing-policy mirroring, GitHub access, and migration |
| `sandbox.rs` | Sandbox status/lifecycle, review mode, remote commands, and Cargo audits |
| `host.rs` | Setup, action-specific preflight, and `doctor` diagnostics |
| `auth.rs` | Credential-import offer, explicit import, and subscription status |
| `vscode.rs` | Remote-SSH launch and VS Code extension mirroring |
| `sync.rs` | Agent configuration selection/transfer and persistent bootstrap state |
| `embedded.rs` | Compile-time inclusion and materialization of kit specifications |
| `process.rs` | External-command execution and installation guidance |
| `sha256.rs` | Dependency-free deterministic project-name hashing |

Keep `main.rs` minimal and `app.rs` focused on orchestration. Put new behavior
in the module that owns the concept. Create another cohesive module when a
feature introduces a genuinely new responsibility; do not split code solely
to reduce line counts.

The specifications under `kits/` are embedded into the executable. They may
contain Linux provisioning commands that run inside a sandbox. The host
launcher itself remains pure Rust and must not depend on companion shell
scripts.

## Feature workflow

1. Create a focused branch and establish a clean baseline:

   ```bash
   git switch -c feat/short-description
   cargo test --frozen
   cargo clippy --frozen --all-targets -- -D warnings
   ```

2. Identify the owning module from the architecture table. Before adding a new
   abstraction, search for the existing path with `rg`.

3. Implement the smallest complete behavior. Preserve the current boundaries:

   - CLI syntax and preset selection belong in `cli.rs`.
   - Command sequencing belongs in `app.rs`, while the actual operation belongs
     in its domain module.
   - External commands should use the helpers in `process.rs` so errors and
     `SBXR_PRESERVE_XDG_STATE_HOME` behavior stay consistent.
   - Keep internal APIs `pub(crate)` unless they truly need wider visibility.
   - Gate platform-specific code with `cfg` and avoid assuming UTF-8 filesystem
     paths where the existing code supports native paths.
   - Return actionable errors for invalid input and external failures. Do not
     panic on user-controlled data.

4. Add or update tests next to the module they cover. Parsing, naming,
   filtering, merging, and serialization behavior should normally have unit
   tests that do not require Docker Sandboxes.

5. Update user documentation when commands, environment variables, presets,
   authentication, security properties, or setup behavior change. New
   environment variables must be documented in
   [ENVIRONMENT.md](ENVIRONMENT.md).

## Kit changes

For a change to an existing kit, edit its `kits/<name>/spec.yaml`. If adding or
removing a kit, also update both lists in `src/embedded.rs`: the embedded
content table and the kit-name validation list.

Validate affected kits with the installed Docker Sandboxes CLI:

```bash
sbx kit validate kits/rust
sbx kit validate kits/claude-cli
sbx kit validate kits/git-ssh-sign
sbx kit validate kits/github-ssh
```

Run validation for every changed kit. A syntax-valid kit can still fail during
installation, so provisioning changes also require a live creation test.
Remember that kits are applied when a sandbox is created; reusing an old
sandbox does not retest changed installation commands.

Kit security changes deserve explicit review. Pay particular attention to:

- newly allowed network destinations;
- downloaded artifact versions and SHA-256 checksums;
- credentials and files made visible inside the sandbox;
- writable or host-mounted paths;
- root and startup commands;
- commands that execute package-manager or project-controlled code.

## Dependency changes

Dependencies are intentionally minimized because `sbxr` is the bootstrap tool
used to create the safer build environment. Do not add a crate for a small
operation that is straightforward to implement and test locally.

Any accepted dependency change must update `Cargo.toml`, `Cargo.lock`, the
vendored source, and [THIRD_PARTY_REVIEW.md](THIRD_PARTY_REVIEW.md). Follow the
mandatory review procedure in that document, including source review,
checksums, build scripts, procedural macros, features, licenses, and the vendor
diff. Never hand-edit `Cargo.lock` checksums.

## Required local checks

Run these from the `sbxr` repository root before considering a change ready:

```bash
cargo fmt --all -- --check
cargo test --frozen
cargo clippy --frozen --all-targets -- -D warnings
git diff --check
```

Also build the optimized binary when changing conditional compilation, release
profiles, embedding, or installation behavior:

```bash
cargo build --release --frozen
```

## Live verification

Run live checks when changing sandbox lifecycle, kits, host preflight,
authentication, configuration mirroring, VS Code, process invocation, or
platform integration.

Start with the read-only host diagnostic:

```bash
cargo run --frozen -- doctor
```

Use `up` for a non-GUI lifecycle check against a disposable or dedicated test
project:

```bash
cargo run --frozen -- up /path/to/test-rust-project
```

Confirm the second run reuses the same deterministic sandbox:

```bash
cargo run --frozen -- name /path/to/test-rust-project
cargo run --frozen -- up /path/to/test-rust-project
```

Check relevant tools and compile without writing `target/` into the mounted
host project:

```bash
sbx exec SANDBOX_NAME -- bash -lc '
  set -euo pipefail
  cd -- "$1"
  rustc --version
  cargo --version
  codex --version
  claude --version
  pi --version
  CARGO_TARGET_DIR=/tmp/sbxr-live-target cargo check --locked
' bash /path/to/test-rust-project
```

Only test the presets affected by the change. For example:

```bash
cargo run --frozen -- --preset rust-codex up /path/to/test-rust-project
cargo run --frozen -- --preset rust-claude up /path/to/test-rust-project
```

Stop test sandboxes afterward while preserving their state:

```bash
cargo run --frozen -- stop /path/to/test-rust-project
```

Use `rm` only when intentionally discarding the sandbox-local state. It does
not delete the mounted host project, but sandbox packages, agent history, and
other VM-local state are removed.

GUI changes additionally require manual verification of `sbxr new`: VS Code
must open through Remote-SSH, the workspace path must be correct, Rust Analyzer
must work, and compatible host extensions should appear remotely without an
“Install in SSH” action. Compare `code --list-extensions --show-versions` with
the SSH window's installed extensions. Do not use `code --remote NAME
--list-extensions` as remote verification: VS Code's desktop extension CLI can
report the local inventory for that form. The development window must not show
a Workspace Trust prompt, while `sbxr review` must retain Workspace Trust.
For bootstrap changes, exercise three distinct runs: the first `sbxr new` must
ensure the UTF-8 locale, configure Git integration, mirror agent integrations,
wait for the VS Code
Server, install/verify remote extensions, and write
`/home/agent/.local/state/sbxr/bootstrap-v1.json`; the
second `sbxr new` must open VS Code without any of that bootstrap output; and
`sbxr update` must force both agent and VS Code work again and restore both
completion flags. Repeat the fast-path check once with a stopped sandbox so the
marker lookup also proves that it starts the sandbox correctly.
For Git integration changes, verify `ssh-add -L` inside `sbxr shell`, an
authenticated `ssh -T git@github.com`, lazy-loaded Bash completion, editor
launch, and a signed disposable commit. Never use the real working tree merely
to test commit signing. Also test with `SSH_AUTH_SOCK` pointing at a stale
socket while a standard desktop agent socket is live; `sbxr doctor` must name
the selected fallback without printing public-key material.
Close the window with multiple terminal tabs open, run `sbxr new` again, and
verify VS Code restores the remote window and its persistent terminal state.
Then run `sbxr stop` while the remote window is open: only that window should
close, the sandbox should stop after the window disappears, and the next
`sbxr new` should exercise VS Code's process-revive path. Verify terminal tabs,
working directories, history, and scrollback separately, and inspect the host
workspace database plus the remote `ptyhost.log` when any part is missing; do
not infer scrollback persistence merely from the tab reappearing. Record the
exact VS Code and Remote-SSH versions because remote scrollback restoration is
currently best-effort as described in `PLATFORM_SUPPORT.md`. Cancel an
unsaved-file prompt once and confirm the stop is aborted without powering off
the sandbox. On Linux/X11, exercise this with `wmctrl` installed and with
another unrelated VS Code window open.

## Before handing off a change

- Review `git diff` and confirm unrelated developer changes were preserved.
- Confirm the required local checks pass without network access.
- Record which live scenarios and presets were exercised.
- Report warnings separately from failures, especially missing optional Claude
  login or a deliberately broader global network policy.
- Install the final candidate if the locally installed command must reflect the
  checkout:

  ```bash
  cargo install --frozen --path . --force
  sbxr doctor
  ```

Do not commit generated `target/` directories, authentication material,
sandbox state, or host-specific paths.
