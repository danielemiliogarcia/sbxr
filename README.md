# sbxr

`sbxr` is an sbx (docker sandbox) wrapper that creates project-scoped
Rust development microVMs with Codex, Claude, Pi, and native VS Code Remote-SSH.
Its default preset is the complete multi-agent environment.

To build `sbxr` or contribute a feature, see the
[development guide](DEVELOPMENT.md).

## Quick start

From a reviewed `sbxr` checkout:

```bash
# ONLY ONCE: install the launcher from its locked, vendored dependencies.
# Run this again only when installing a newer sbxr version.
cargo install --frozen --path .

# ONLY ONCE: configure login/OAuth/Remote-SSH, then verify the host.
# Setup is safe to rerun later if host configuration changes.
sbxr setup
sbxr doctor

# REPEAT FOR EACH PROJECT that should have its own sandbox.
# Existing project directories work too; this example starts an empty one.
mkdir -p ~/code/my-rust-project
cd ~/code/my-rust-project
sbxr new .
```

`setup` explains how to install any missing system prerequisite and can be
rerun safely. An existing Rust project directory works in place of the empty
directory above; an empty directory is initialized with Cargo automatically.

## What it includes

The default `rust-multi-agent` preset combines:

- Docker's official `codex` agent with host-managed ChatGPT subscription OAuth;
- pinned Rust tooling and Cargo security tools;
- pinned Claude Code with sandbox-local Claude subscription OAuth;
- pinned Pi, connected to the Docker-managed Codex OAuth proxy and able to
  detect the sandbox-local Claude login;
- VS Code Remote-SSH, including version-matched host extension installation;
- safe Codex, Claude, and Pi capability/configuration mirroring.

The host launcher and its configuration-copy implementation are Rust. It ships
no host shell helper and embeds all kit specifications in the executable. The
declarative kits still contain normal Linux provisioning commands executed
inside the sandbox.

## Installation and bootstrap security

Hardened, vendored installation from this checkout:

```bash
cargo install --frozen --path .
```

`--frozen` includes `--locked` and disables network access. The checked-in
`.cargo/config.toml` directs Cargo to the reviewed `vendor/` tree, so no
dependency is downloaded or re-resolved during this installation. The
dependency versions, checksums, build-script findings, and update procedure are
recorded in [`THIRD_PARTY_REVIEW.md`](THIRD_PARTY_REVIEW.md).

This vendoring specifically addresses the bootstrap chicken-and-egg problem:
developers should not need to fetch and compile a newly resolved third-party
dependency graph in order to install the tool that will isolate later Cargo
builds. It does not make vendored code intrinsically safe; the reviewed Git
revision and its vendor diff remain part of the trust decision.

The transparent registry-backed alternative remains available from a reviewed
Git revision after this repository is published:

```bash
cargo install --locked --git https://example.com/your-team/sbxr \
  --rev FULL_COMMIT_SHA
```

This alternative still enforces `Cargo.lock`, but Cargo may download the exact
locked registry packages because `cargo install --git` does not discover the
repository-local Cargo configuration. To use the vendored offline path, clone
or unpack the reviewed revision and use `cargo install --frozen --path .`.

`publish = false` currently prevents a misleading `cargo install sbxr`
claim: no crates.io release exists yet. Release binaries can also be built for
each Docker-Sandboxes-supported host platform. The binary automatically writes
its embedded kits under the platform data directory; developers do not need a
separate kit checkout.

## Host setup behavior

`sbxr setup` is idempotent. It detects and configures only missing
interactive prerequisites: Docker sign-in, host-managed OpenAI OAuth for
presets that use Codex, and the VS Code Remote-SSH extension. Those steps may
open a browser or download the extension. It verifies the result of each
command and continues far enough to report multiple problems together. It
never installs system packages or invokes `sudo`; missing `sbx`, VS Code,
OpenSSH, Git, or KVM still receive explicit manual installation guidance.

The automated steps are equivalent to running these commands manually when
their corresponding state is missing:

```bash
sbx login
sbx secret set openai --oauth
code --install-extension ms-vscode-remote.remote-ssh
```

| Detected state | `sbxr setup` behavior |
|---|---|
| `sbx` installed but Docker sign-in or readiness is missing | Runs `sbx login`, then verifies with `sbx ls` |
| OpenAI OAuth missing for `rust-multi-agent` or `rust-codex` | Runs the browser-based OAuth command, then verifies it |
| VS Code installed but Remote-SSH missing | Installs the Microsoft extension, then verifies it |
| `sbx`, VS Code, OpenSSH, or KVM missing | Prints installation guidance and exits non-zero |
| Git missing | Warns that review mode is unavailable |
| Everything already configured | Reports success without changing anything |

Use `sbxr --preset rust-claude setup` when configuring a Claude-only
installation; it skips OpenAI OAuth. Claude subscription login itself remains
inside the sandbox, using Claude's native `/login` flow.

The first `sbxr new .` run creates a deterministic project sandbox,
initializes an empty directory with Cargo, and opens VS Code. Later runs reuse
that sandbox. Before taking action, every command runs a scoped host preflight
and reports all of its missing prerequisites together. For example, `new`
checks `sbx`, VS Code, OpenSSH, the VS Code Remote-SSH extension, and KVM access
on Linux, while `shell` does not require VS Code. The preflight only gives
installation guidance; it never installs host software or invokes `sudo`.
`doctor` remains the complete diagnostic, including Docker sign-in, OAuth,
policy, and kit validation.

## Optional host RocksDB acceleration

This is disabled by default. On a Linux host with a compatible installation at
`/opt/rocksdb-10.4.2`, opt in with:

```bash
sbxr --rocksdb-host new .
```

Use `--rocksdb-host=/another/prefix` for a custom location. The prefix is
validated and mounted read-only, sandbox-native compression libraries are
installed, and the sandbox gets a distinct `rdb` name. Repeat the option on
later `shell`, `audit`, `stop`, or `rm` commands. See the full comparison and
all three option quickstarts in
[`../explain.md`](../explain.md#rocksdb-reuse-options-and-quickstarts).

## Presets

| Preset | Primary official agent | Extra agents |
|---|---|---|
| `rust-multi-agent` (default) | Codex | Claude and Pi |
| `rust-codex` | Codex | none |
| `rust-claude` | Claude | none |

Select one with `--preset rust-codex`. Preset-specific sandbox names prevent
accidentally reusing a VM with a different agent composition.

## Commands

| Command | Purpose |
|---|---|
| `sbxr new .` | Create/reuse and open VS Code |
| `sbxr up .` | Create/reuse without attaching |
| `sbxr shell .` | Open a project shell |
| `sbxr codex .` | Run Codex |
| `sbxr claude .` | Run Claude Code |
| `sbxr pi .` | Run Pi in the multi-agent preset |
| `sbxr auth-import .` | Explicitly import a trusted host Claude cache |
| `sbxr auth-status .` | Check Codex, Claude, and Pi provider status |
| `sbxr audit .` | Run cargo-audit, cargo-deny, and cargo-vet if configured |
| `sbxr review .` | No-OAuth, clone-mode review sandbox |
| `sbxr name .` | Print the deterministic name |
| `sbxr stop .` | Stop while preserving state |
| `sbxr rm .` | Remove sandbox-local state; host files remain |
| `sbxr setup` | Configure missing Docker login, OpenAI OAuth, and Remote-SSH |
| `sbxr doctor` | Run the complete read-only host diagnostic |

No custom environment variables are required. Optional preset, naming,
storage, mirroring, RocksDB, and authentication-import overrides are documented
in [ENVIRONMENT.md](ENVIRONMENT.md).

## Subscriptions and customization

Codex uses Docker's official host-managed OAuth path. The real OpenAI token is
not copied into the VM. `auth-import` copies only Claude's cache, after an
explicit warning, because same-user project code can read it.

Pi recognizes Docker's Codex proxy sentinel, so `pi --provider openai-codex`
uses the same ChatGPT subscription without receiving the real token. When a
Claude login exists inside the sandbox, Pi maps it before launch and reconciles
refreshes afterward. Do not run Claude and Pi concurrently while either is
refreshing this credential. Pi documents third-party Claude OAuth use as
Anthropic “extra usage,” potentially billed per token rather than against the
normal plan allowance.

The launcher mirrors an allowlist of useful host configuration: instructions,
skills, hooks, prompts, status-line scripts, themes, and plugin metadata/cache.
It excludes authentication files, histories, sessions, logs, project trust
records, runtime state, and Codex system-managed skills. Host paths in copied
text are rewritten to `/home/agent`, while Docker-managed provider and safety
settings win during merges.

VS Code extensions are read from `code --list-extensions --show-versions` and
installed remotely at matching versions. Host-only Remote extensions are
excluded and incompatible extensions produce a warning. Review mode does not
mirror the host extension set.

Pi prompts, themes, skills, and safe settings are mirrored normally. Executable
Pi extensions/packages are mirrored only when host and sandbox Pi versions
match. Mirroring controls and their compatibility override are documented in
[ENVIRONMENT.md](ENVIRONMENT.md#mirroring-controls).

Mirrored hooks, plugins, skills, and extensions are executable software. Only
mirror them from a trusted host profile.

## Source map

- `DEVELOPMENT.md`: build, architecture, feature workflow, and verification
  guide;
- `src/main.rs`: minimal process entrypoint and module wiring;
- `src/app.rs`: top-level command dispatch;
- `src/cli.rs`: argument parsing, presets, actions, and help text;
- `src/environment.rs`: runtime context, project identity, sandbox names, and
  optional host RocksDB validation;
- `src/sandbox.rs`: development/review sandbox lifecycle, remote commands, and
  Cargo audits;
- `src/host.rs`: setup, preflight checks, and diagnostics;
- `src/auth.rs`: explicit credential import and subscription status;
- `src/vscode.rs`: Remote-SSH launch and extension mirroring;
- `src/sync.rs`: pure-Rust allowlist, safe merges, path rewriting, and tar
  streaming;
- `src/embedded.rs`: compile-time kit embedding and materialization;
- `src/process.rs`: child-process handling and prerequisite guidance;
- `kits/`: exact declarative environments, including the optional
  `rocksdb-host` mixin, embedded by the binary.
- `.cargo/config.toml` and `vendor/`: offline, locked bootstrap dependency
  source;
- `THIRD_PARTY_REVIEW.md`: version/checksum inventory, build-script findings,
  limitations, and the mandatory dependency-update procedure.
- `ENVIRONMENT.md`: complete optional environment-variable reference.

See [SECURITY.md](SECURITY.md) for the threat model and
[ENVIRONMENT.md](ENVIRONMENT.md) for configuration overrides.
