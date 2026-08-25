# sbxr

`sbxr` is an sbx (docker sandbox) wrapper that creates project-scoped
Rust development microVMs with Codex, Claude, Pi, and native VS Code Remote-SSH.
Its default preset is the complete multi-agent environment.


## Quick start

```bash
# ONLY ONCE: install sbxr, or rerun this command to upgrade from master.
cargo install --locked --force \
  --git https://github.com/danielemiliogarcia/sbxr.git \
  --branch master

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


## Prerequisites

- A host supported by [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/install/),
  with the required virtualization support (KVM on Linux).
- The Docker Sandboxes `sbx` CLI. Follow Docker's official
  [installation and sign-in guide](https://docs.docker.com/ai/sandboxes/install/).
  Docker Desktop and Docker Engine are not required just to use `sbx`.
- Rust and Cargo 1.85 or newer to build and install `sbxr` from source.
- Native VS Code with the `code` command on `PATH`, plus an OpenSSH client.
- Git is recommended and is required for `sbxr review`.
- A ChatGPT subscription, Claude subscription, or both, depending on the
  selected preset. Authentication uses the providers' OAuth login flows; API
  keys are not required.

`sbxr setup` configures Docker sign-in, agent OAuth, and VS Code Remote-SSH
when needed. It reports missing host software with installation guidance, but
does not install system packages or invoke `sudo`.

To build `sbxr` or contribute a feature, see the
[development guide](DEVELOPMENT.md).


## What it includes

The default `rust-multi-agent` preset combines:

- Docker's official `codex` agent with host-managed ChatGPT subscription OAuth;
- pinned Rust tooling, Cargo security tools, and a colored Git-aware Bash
  prompt;
- pinned Claude Code with sandbox-local Claude subscription OAuth;
- pinned Pi, connected to the Docker-managed Codex OAuth proxy and able to
  detect the sandbox-local Claude login;
- VS Code Remote-SSH, including version-matched host extension installation;
- safe Codex, Claude, and Pi capability/configuration mirroring.

The host launcher and its configuration-copy implementation are Rust. It ships
no host shell helper and embeds all kit specifications in the executable. The
declarative kits still contain normal Linux provisioning commands executed
inside the sandbox.

## Commands

| Command | Purpose |
|---|---|
| `sbxr new .` | Create/reuse and open VS Code |
| `sbxr vscode .` | Explicit alias for `sbxr new .` |
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
| `sbxr status .` | Report whether the project sandbox exists and its state |
| `sbxr stop .` | Stop while preserving state |
| `sbxr rm .` | Remove sandbox-local state; host files remain |
| `sbxr rm --force .` | Remove even while VS Code/SSH is connected |
| `sbxr setup` | Configure missing Docker login, OpenAI OAuth, and Remote-SSH |
| `sbxr doctor` | Run the complete read-only host diagnostic |

`sbxr status` defaults to the current directory and never creates or starts a
sandbox. It prints the deterministic name, resolved project path, and
`running`, `stopped`, or `not found`. Its exit status is `0` when the sandbox
exists and `1` when it does not, making it suitable for scripts. Like sandbox
names, the result is preset- and optional-RocksDB-specific.

No custom environment variables are required. Optional preset, naming,
storage, mirroring, RocksDB, and authentication-import overrides are documented
in [ENVIRONMENT.md](ENVIRONMENT.md).

## Why `sbxr`? From project folder to agent-ready Rust IDE

Docker Sandboxes provides the secure microVM and the flexible `sbx` building
blocks. `sbxr` adds an opinionated Rust-development workflow on top: stand in a
project directory, run `sbxr new`, and get a sandbox that is already prepared
for coding with native VS Code and multiple subscription-backed agents. The
design goal is to give the developer the familiar experience of coding on the
local host while moving tool execution and agent risk behind the sandbox
boundary.

The VS Code window and user interface remain native on the host. Through
Remote-SSH, terminals, Rust Analyzer, debuggers, agents, and workspace
extensions execute beside the project inside the sandbox. The SSH connection
targets a microVM on the same physical machine rather than a server across a
wide-area network, so editor/control traffic has very low latency and expensive
analysis stays close to the source files. This makes the experience feel close
to local development, although VM startup and mounted-filesystem operations can
still add some overhead compared with running everything directly on the host.

That ready-to-code experience includes:

- one project-scoped, deterministic sandbox that is created once and reused;
- a default stack containing Docker's official Codex environment, Rust,
  Claude Code, Pi, and native VS Code Remote-SSH support;
- ChatGPT and Claude subscription OAuth workflows without requiring API keys;
- a pinned Rust toolchain plus `rustfmt`, Clippy, Rust Analyzer, LLDB, and Cargo
  supply-chain tools such as `cargo-audit`, `cargo-deny`, and `cargo-vet`;
- a local-like Rust editing experience in VS Code: remote Rust Analyzer powers
  completion, diagnostics, hover information, find references, go to
  definition, and source navigation across the project and available imported
  crate sources;
- an immediately familiar colored Bash prompt with the current Git branch;
- allowlisted Codex, Claude, and Pi instructions, skills, hooks, prompts,
  themes, plugins, and status-line customizations mirrored from the host;
- compatible host VS Code extensions installed at matching versions in the
  remote extension host, with server-side verification and retries;
- the normal development folder opened without a redundant Workspace Trust
  prompt, while the unfamiliar-code `review` workflow keeps that protection;
- reuse of the same named VS Code remote, allowing VS Code to restore its
  window, terminal tabs, scrollback, and persistent remote workspace state when
  possible;
- automatic Cargo initialization for an empty project directory;
- setup, diagnostics, authentication status, auditing, shell/agent launchers,
  and safe sandbox lifecycle commands in one native Rust executable;
- optional, explicitly enabled host RocksDB reuse for compatible Linux setups.

Nearly the same environment can be assembled manually with `sbx`; `sbxr` does
not replace or bypass Docker Sandboxes. Doing it by hand means maintaining and
stacking the kits, choosing stable project names, configuring SSH, opening the
right remote folder, waiting for the matching VS Code Server, installing and
verifying remote extensions, arranging each agent's authentication and safe
customization, recreating the shell experience, preserving the reusable VS
Code remote identity, and remembering the correct lifecycle commands. `sbxr`
packages that repeatable integration work so every new project does not require
the same manual setup.

## Why a microVM instead of a container?

The reason is the security boundary. A normal container isolates processes
with namespaces and other kernel features, but still shares the host's kernel.
Docker Sandboxes gives each sandbox a lightweight VM with its own Linux kernel,
separated from the host by a hypervisor. Escaping either design is never
impossible, but compromising a container isolation mechanism can expose the
shared host kernel; escaping a microVM must also cross the separate guest-kernel
and hypervisor boundary. That is a stronger fit for autonomous agents that run
untrusted build scripts, install packages, use `sudo`, and execute generated
code. Docker describes the layers in its
[sandbox isolation model](https://docs.docker.com/ai/sandboxes/security/isolation/).

The stronger boundary does not make development feel like manually operating
a traditional VM. `sbx` manages the microVM lifecycle, its private filesystem,
network, and isolated Docker Engine. The developer does not create a VM image,
configure SSH ports, expose the host Docker socket, or maintain a privileged
container solely to give an agent Docker access.

In the default direct mode, Docker Sandboxes mounts the selected project into
the microVM through filesystem passthrough at the same absolute path it has on
the host. Both sides see the same working tree, and writes appear immediately
without a copy or synchronization step. `sbxr new .` supplies that workspace to
`sbx`, so the developer does not have to construct `-v` mappings, translate
paths, or arrange container UID/GID ownership merely to edit the project from
both sides. This managed sharing avoids much of the bind-mount and permission
plumbing commonly encountered in hand-built development containers. See
Docker's [workspace-mounting architecture](https://docs.docker.com/ai/sandboxes/architecture/#workspace-mounting).

This convenience has an intentional boundary: the selected project is
read-write, so an agent can modify or delete anything inside it. The rest of
the host filesystem, host processes, host kernel, and host Docker daemon remain
outside the sandbox boundary. For unfamiliar code, `sbxr review` uses Docker's
clone mode so the host repository is mounted read-only and changes remain in a
private in-VM clone until the developer explicitly retrieves them.

## Installation and bootstrap security

Convenient installation or upgrade from the latest public `master` branch:

```bash
cargo install --locked --force \
  --git https://github.com/danielemiliogarcia/sbxr.git \
  --branch master
```

This follows a mutable branch and downloads the dependency versions selected by
its `Cargo.lock`. For a reproducible registry-backed installation, replace the
branch with a reviewed full commit ID:

```bash
cargo install --locked --force \
  --git https://github.com/danielemiliogarcia/sbxr.git \
  --rev FULL_COMMIT_SHA
```

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

The Git-backed alternatives still enforce `Cargo.lock`, but Cargo may download
the exact locked registry packages because `cargo install --git` does not
discover the repository-local Cargo configuration. To use the vendored offline
path, clone or unpack the reviewed revision and use
`cargo install --frozen --path .`.

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
inside the sandbox. When a host Claude login is detected, `sbxr new` and
`sbxr vscode` offer to import it with an explicit credential warning; otherwise
use Claude's native `/login` flow inside the sandbox.

The first `sbxr new .` run creates a deterministic project sandbox,
initializes an empty directory with Cargo, and opens VS Code. Later runs reuse
that sandbox and ask VS Code to restore the existing remote project window
rather than forcing a fresh session. With VS Code's persistent-terminal
settings enabled (the default), terminal tabs and scrollback reconnect or
revive when possible. Stopping or removing the underlying process can prevent
a live process from reconnecting; `sbxr rm` intentionally discards all
sandbox-local state. Before taking action, every command runs a scoped host
preflight and reports all of its missing prerequisites together. For example,
`new` checks `sbx`, VS Code, OpenSSH, the VS Code Remote-SSH extension, and KVM
access on Linux, while `shell` does not require VS Code. The preflight only gives
installation guidance; it never installs host software or invokes `sudo`.
`doctor` remains the complete diagnostic, including Docker sign-in, OAuth,
policy, and kit validation.

Normal `sbxr new` and `sbxr vscode` windows disable VS Code Workspace Trust for
that launched session. The sandbox is already the execution boundary and the
development agents intentionally have full permissions inside it, so the
additional “Trust this folder” prompt would only block expected tooling.
`sbxr review` does not disable Workspace Trust because it is intended for
opening unfamiliar code with additional safeguards.

## Optional host RocksDB acceleration

This is disabled by default. On a Linux host with a compatible installation at
`/opt/rocksdb-10.4.2`, opt in with:

```bash
sbxr --rocksdb-host new .
```

Use `--rocksdb-host=/another/prefix` for a custom location. The prefix is
validated and mounted read-only, sandbox-native compression libraries are
installed, and the sandbox gets a distinct `rdb` name.
## Presets

| Preset | Primary official agent | Extra agents |
|---|---|---|
| `rust-multi-agent` (default) | Codex | Claude and Pi |
| `rust-codex` | Codex | none |
| `rust-claude` | Claude | none |

Select one with `--preset rust-codex`. Preset-specific sandbox names prevent
accidentally reusing a VM with a different agent composition.

## Subscriptions and customization

Codex uses Docker's official host-managed OAuth path. The real OpenAI token is
not copied into the VM. Claude is an additional CLI in the default preset, so
it cannot use that Codex-specific proxy. When the host Claude cache exists and
the sandbox has no Claude login, `sbxr new` and `sbxr vscode` offer to import
it after an explicit warning. `sbxr auth-import .` performs the same operation
on demand. Same-user project code can read an imported Claude credential.

Pi recognizes Docker's Codex proxy sentinel, so `pi --provider openai-codex`
uses the same ChatGPT subscription without receiving the real token. When a
Claude login exists inside the sandbox, Pi maps it before launch and reconciles
refreshes afterward. Do not run Claude and Pi concurrently while either is
refreshing this credential. Pi documents third-party Claude OAuth use as
Anthropic “extra usage,” potentially billed per token rather than against the
normal plan allowance.

The launcher mirrors an allowlist of useful host configuration: instructions,
skills, hooks, prompts, status-line scripts (including a script referenced from
Claude's `statusLine.command`), themes, and plugin metadata/cache.
It excludes authentication files, histories, sessions, logs, project trust
records, runtime state, and Codex system-managed skills. Host paths in copied
text are rewritten to `/home/agent`, while Docker-managed provider and safety
settings win during merges.

VS Code extensions are read from `code --list-extensions --show-versions`.
After the project window starts the matching VS Code Server, they are compared
and installed through that server's remote extension CLI at matching versions.
Missing extensions are verified remotely and retried
individually, so one incompatible extension does not hide the others or require
clicking “Install in SSH.” Host-only Remote extensions are excluded and any
remaining incompatible extensions are named in a warning. Review mode does not
mirror the host extension set.

The public VS Code CLI reports installed extensions, not every per-profile or
per-workspace enable/disable decision. Newly installed remote extensions are
enabled by default; explicit disablement remains controlled by VS Code's user,
remote, and workspace settings.

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
- `src/auth.rs`: credential-import offer, explicit import, and subscription
  status;
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

For operating-system compatibility, host requirements, limitations, and
validation status, see [PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md).
