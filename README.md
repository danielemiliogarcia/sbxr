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
- Docker Sandboxes `sbx` 0.39.0 or newer. Follow Docker's official
  [installation and sign-in guide](https://docs.docker.com/ai/sandboxes/install/).
  Docker Desktop and Docker Engine are not required just to use `sbx`.
- Rust and Cargo 1.85 or newer to build and install `sbxr` from source.
- Native VS Code with the `code` command on `PATH`, plus an OpenSSH client.
- On Linux/X11, `wmctrl` is recommended so `sbxr stop` can gracefully close
  only the matching Remote-SSH window (`sudo apt-get install wmctrl` on Ubuntu).
- Git is recommended and is required for `sbxr review`.
- For GitHub SSH push and sandbox commit signing, load a key in the host SSH
  agent (`ssh-add -L` must list it). The private key stays on the host.
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
  prompt, plus `nano`, Bash/Git completion, and a generated `en_US.UTF-8`
  locale for clean VS Code terminals;
- pinned Claude Code with sandbox-local Claude subscription OAuth;
- pinned Pi, connected to the Docker-managed Codex OAuth proxy and able to
  detect the sandbox-local Claude login;
- VS Code Remote-SSH, including version-matched host extension installation;
- GitHub SSH push/pull and host-policy-matched SSH commit signing through
  Docker's forwarded host SSH agent;
- trusted-host Codex, Claude, and Pi extension/plugin and configuration
  mirroring, with credentials and runtime state excluded.

The host launcher and its configuration-copy implementation are Rust. It ships
no host shell helper and embeds all kit specifications in the executable. The
declarative kits still contain normal Linux provisioning commands executed
inside the sandbox.

Sandbox composition is generated as a Docker `.sbxenv.yaml` under protected
host state, never inside the mounted project. Docker's experimental `sbx env`
interface owns declarative creation, reuse, and removal; `sbxr` retains stable
project identity, contract-drift protection, host integration, and native IDE
orchestration.

## Commands

| Command | Purpose |
|---|---|
| `sbxr new .` | Create/reuse and open VS Code |
| `sbxr vscode .` | Explicit alias for `sbxr new .` |
| `sbxr update .` | Force host agent/VS Code mirroring, then open VS Code |
| `sbxr up .` | Create/reuse without attaching |
| `sbxr shell .` | Open a project shell |
| `sbxr codex .` | Run Codex |
| `sbxr claude .` | Run Claude Code |
| `sbxr pi .` | Run Pi in the multi-agent preset |
| `sbxr auth-import .` | Explicitly import a trusted host Claude cache |
| `sbxr auth-status .` | Check Codex, Claude, and Pi provider status |
| `sbxr audit .` | Run cargo-audit, cargo-deny, and cargo-vet if configured |
| `sbxr review .` | No-OAuth, clone-mode review sandbox |
| `sbxr inspect .` | Show the protected environment declaration and contract state |
| `sbxr name .` | Print the deterministic name |
| `sbxr status .` | Report whether the project sandbox exists and its state |
| `sbxr stop .` | Stop while preserving state |
| `sbxr rm .` | Remove sandbox-local state; host files remain |
| `sbxr rm --force .` | Remove even while VS Code/SSH is connected |
| `sbxr setup` | Configure missing Docker login, OpenAI OAuth, and Remote-SSH |
| `sbxr doctor` | Run the complete read-only host diagnostic |

## Git inside or outside the sandbox

In the default direct mode, the host and sandbox see the same working tree and
the same `.git` directory. You may inspect, stage, commit, and push from either
side; do not run competing Git mutations on both sides at the same time.

On first bootstrap, `sbxr` installs `nano` as Git's editor, enables Bash/Git
completion, mirrors the host's global `user.name` and `user.email`, installs
GitHub's published SSH host keys, and permits GitHub SSH traffic. Docker
Sandboxes automatically forwards signing and authentication requests to the
host agent, so private SSH keys are not copied into the microVM. See Docker's
[Git sandbox workflow and signing
model](https://docs.docker.com/ai/sandboxes/workflows/git/).
If the host's `SSH_AUTH_SOCK` is stale, `sbxr` automatically selects a live
standard desktop agent with loaded keys (such as GCR or GnuPG) before starting
Docker Sandboxes; `sbxr doctor` reports when this fallback is active.

`sbxr` mirrors the host project's effective `commit.gpgSign` and `tag.gpgSign`
defaults and `gpg.format`, including global settings and repository overrides.
Host SSH signing remains enabled and uses the forwarded SSH agent. When host
signing is disabled, it remains disabled. When the host requires OpenPGP/GPG
(including a YubiKey) or S/MIME signing, `sbxr` warns during bootstrap and
disables automatic sandbox signing because Docker cannot forward those
signers; ordinary commits made inside that sandbox are unsigned and must be
signed or re-signed on the host before pushing. An explicit `git commit -S`
still requests the mirrored format and fails when its signer is unavailable.
Register an SSH public key with GitHub as a signing key when using SSH signing
and GitHub's **Verified** badge is required.

Only GitHub SSH is allowed by the bundled policy. Other Git hosts need their
own reviewed network/known-host kit. Any process in a connected sandbox can ask
the forwarded SSH agent to authenticate or sign while that connection is
alive; use an agent that confirms operations when this matters.

`sbxr status` defaults to the current directory and never creates or starts a
sandbox. It prints the deterministic name, resolved project path, and
`running`, `stopped`, or `not found`. Its exit status is `0` when the sandbox
exists and `1` when it does not, making it suitable for scripts. Like sandbox
names, the result is preset- and optional-RocksDB-specific.

`sbxr inspect` is also read-only and does not require or start Docker
Sandboxes. It prints the protected environment path, desired/applied host
contract state, and YAML for the selected project, preset, and optional
RocksDB configuration. Before the first sandbox creation it renders a
prospective declaration in memory without writing it. After materialization it
shows the authoritative file stored under protected host state.

When the matching Remote-SSH window is open, `sbxr stop` asks that exact VS
Code window to close normally and waits before stopping the microVM. This gives
VS Code time to serialize terminal tabs, process metadata, command history, and
scrollback; other VS Code windows are left alone. An unsaved-file prompt must
be resolved before stopping proceeds. Linux/X11 uses `wmctrl` to address the
exact window; if safe targeted closing is unavailable, `sbxr` leaves the
sandbox running and asks for a manual close. On the next `sbxr new`, VS Code
uses its process-revive support to recreate terminal shells from the saved
session metadata, with up to 10,000 lines requested for persistent scrollback.
Long-running processes cannot continue while the microVM is powered off.
Terminal-tab, working-directory, and scrollback restoration ultimately depends
on the installed VS Code/Remote-SSH release; see
[Platform support](PLATFORM_SUPPORT.md#vscode-state-across-a-microvm-stop).

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
- `nano` as a working Git editor, Bash/Git completion, mirrored Git author
  identity and signing defaults, GitHub push/pull through the host SSH agent,
  and SSH signatures when requested by that policy or `-S`;
- installed Codex, Claude, and Pi extensions/plugins plus instructions, skills,
  hooks, prompts, themes, and status-line customizations mirrored from the
  trusted host profile;
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

`sbxr setup` is idempotent. It requires Docker Sandboxes 0.39.0 or newer,
detects and configures missing
interactive prerequisites: Docker sign-in, host-managed OpenAI OAuth for
presets that use Codex, Docker-managed `*.sbx` SSH configuration, and the VS
Code Remote-SSH extension. Those steps may
open a browser or download the extension. It verifies the result of each
command and continues far enough to report multiple problems together. It
never installs system packages or invokes `sudo`; missing `sbx`, VS Code,
OpenSSH, Git, or KVM still receive explicit manual installation guidance.

The automated steps are equivalent to running these commands manually when
their corresponding state is missing:

```bash
sbx login
sbx secret set openai --oauth
sbx setup ssh
code --install-extension ms-vscode-remote.remote-ssh
```

| Detected state | `sbxr setup` behavior |
|---|---|
| `sbx` installed but Docker sign-in or readiness is missing | Runs `sbx login`, then verifies with `sbx ls` |
| OpenAI OAuth missing for `rust-multi-agent` or `rust-codex` | Runs the browser-based OAuth command, then verifies it |
| Docker `*.sbx` SSH configuration | Safely reruns `sbx setup ssh` to refresh/verify it |
| VS Code installed but Remote-SSH missing | Installs the Microsoft extension, then verifies it |
| `sbx`, VS Code, OpenSSH, or KVM missing | Prints installation guidance and exits non-zero |
| Git missing | Warns that review mode is unavailable |
| SSH agent has no loaded key | `doctor` warns that sandbox GitHub authentication/signing will be unavailable |
| Everything already configured | Reports success without changing anything |

Use `sbxr --preset rust-claude setup` when configuring a Claude-only
installation; it skips OpenAI OAuth. Claude subscription login itself remains
inside the sandbox. When a host Claude login is detected, the initial `sbxr
new`/`sbxr vscode` bootstrap and explicit `sbxr update` offer to import it with
an explicit credential warning; otherwise use Claude's native `/login` flow
inside the sandbox.

The first `sbxr new .` run writes a protected declarative environment outside
the project, creates a deterministic project sandbox,
initializes an empty directory with Cargo, configures Git/editor integration,
mirrors trusted host agent and VS Code integrations, records that bootstrap
inside the persistent sandbox, and opens VS Code. Later `new` runs skip SSH
setup, Git integration, agent copying, the VS Code Server wait, and extension
inventory/install work; they reuse the sandbox and open its existing remote
project window immediately. Run `sbxr update .` when host Git identity, agent
plugins/settings/hooks, or VS Code extensions change and should be re-mirrored
deliberately. If an older or externally-created same-name sandbox has no
verifiable environment-contract marker, `sbxr` refuses to reconcile it and
prints explicit `sbxr rm`/`sbxr new` commands. It never silently destroys or
adopts that sandbox. The contract covers the rendered environment and the
contents of every selected local kit, so a kit update also requires explicit
recreation instead of silently reusing stale provisioning.

With VS Code's persistent-terminal settings enabled (the default), terminal
tabs and scrollback reconnect or revive when possible. Stopping or removing the
underlying process can prevent a live process from reconnecting; `sbxr rm`
intentionally discards all sandbox-local state. Before taking action, every
command runs a scoped host preflight and reports all of its missing
prerequisites together. For example, `new` and `update` check `sbx`, VS Code,
OpenSSH, the VS Code Remote-SSH extension, and KVM access on Linux, while
`shell` does not require VS Code. The preflight only gives installation
guidance; it never installs host software or invokes `sudo`. `doctor` remains
the complete diagnostic, including Docker sign-in, OAuth, policy, and kit
validation.

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
the sandbox has no Claude login, the initial editor bootstrap and `sbxr update`
offer to import it after an explicit warning. `sbxr auth-import .` performs the
same operation on demand. Same-user project code can read an imported Claude
credential.

Pi recognizes Docker's Codex proxy sentinel, so `pi --provider openai-codex`
uses the same ChatGPT subscription without receiving the real token. When a
Claude login exists inside the sandbox, Pi maps it before launch and reconciles
refreshes afterward. Do not run Claude and Pi concurrently while either is
refreshing this credential. Pi documents third-party Claude OAuth use as
Anthropic “extra usage,” potentially billed per token rather than against the
normal plan allowance.

The launcher mirrors the installed extension/plugin payloads in each agent's
documented configuration tree, plus useful host configuration: instructions,
skills, hooks, prompts, status-line scripts (including a script referenced from
Claude's `statusLine.command`), themes, and plugin metadata/cache. Codex and
Claude plugin caches are copied; Pi source extensions are copied and Pi npm
extension packages are reinstalled at the exact versions in the host lockfile.
It excludes authentication files, histories, sessions, logs, project trust
records, runtime state, and Codex system-managed skills. Host paths in copied
text are rewritten to `/home/agent`, while Docker-managed provider and safety
settings win during merges.

Before merging Claude settings, `sbxr` checks simple host hook commands against
the sandbox. The recognized `rtk hook claude` integration is mirrored from the
host as an exact executable and version-checked. A desktop-audio hook using
`paplay` or `afplay` is omitted quietly because the sandbox has no host audio
session. Other unavailable commands are omitted with a warning instead of
failing on every event. Hooks backed by available commands and mirrored scripts
continue to work normally; `sbxr` does not install arbitrary hook dependencies.

VS Code extensions are read from `code --list-extensions --show-versions`.
During the initial sandbox bootstrap—or an explicit `sbxr update`—the matching
VS Code Server compares them and installs matching versions through its remote
extension CLI. Missing extensions are verified remotely and retried
individually, so one incompatible extension does not hide the others or require
clicking “Install in SSH.” Normal later `sbxr new` runs skip the inventory
entirely for faster startup. Host-only Remote extensions are excluded and any
remaining incompatible extensions are named in a warning. Review mode does not
mirror the host extension set.

The public VS Code CLI reports installed extensions, not every per-profile or
per-workspace enable/disable decision. Newly installed remote extensions are
enabled by default; explicit disablement remains controlled by VS Code's user,
remote, and workspace settings.

Pi prompts, themes, skills, settings, and source extensions are mirrored by
default. npm-based Pi extensions are installed without lifecycle scripts from
the host `package-lock.json`, preserving its exact resolved versions instead of
copying a host `node_modules` tree built for a potentially different Pi or
platform. Set `SBXR_SYNC_PI_EXTENSIONS=0` to omit raw source extensions; package
declarations still follow the mirrored Pi settings. Details are in
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
- `src/envfile.rs`: protected declarative environments and desired/applied
  contract state;
- `src/sbx.rs`: the centralized Docker Sandboxes CLI boundary;
- `src/sandbox.rs`: development/review orchestration, remote commands, and
  Cargo audits on top of declarative lifecycle;
- `src/host.rs`: setup, preflight checks, and diagnostics;
- `src/auth.rs`: credential-import offer, explicit import, and subscription
  status;
- `src/vscode.rs`: Remote-SSH launch and extension mirroring;
- `src/sync.rs`: pure-Rust capability selection, merges, path rewriting, tar
  streaming, and persistent bootstrap markers;
- `src/embedded.rs`: compile-time kit embedding and materialization;
- `src/process.rs`: child-process handling and prerequisite guidance;
- `kits/`: exact declarative environments, including the optional
  `rocksdb-host` mixin, embedded by the binary.
- `.cargo/config.toml` and `vendor/`: offline, locked bootstrap dependency
  source;
- `THIRD_PARTY_REVIEW.md`: version/checksum inventory, build-script findings,
  limitations, and the mandatory dependency-update procedure.
- `ENVIRONMENT.md`: complete optional environment-variable reference.
- `CHANGELOG.md`: user-visible changes pending and included in releases.

See [SECURITY.md](SECURITY.md) for the threat model and
[ENVIRONMENT.md](ENVIRONMENT.md) for configuration overrides.

For operating-system compatibility, host requirements, limitations, and
validation status, see [PLATFORM_SUPPORT.md](PLATFORM_SUPPORT.md).

## Limitations

Docker's `.sbxenv.yaml`/`sbx env` interface is experimental in Docker
Sandboxes 0.39. `sbxr` centralizes that dependency, but a future Docker release
may require a compatibility update.

After a microVM stop, VS Code Remote-SSH may restore terminal tabs but lose their scrollback due to a VS Code limitation/bug; [details and tested versions](PLATFORM_SUPPORT.md#vscode-state-across-a-microvm-stop).

Docker Sandboxes forwards SSH agents, but not OpenPGP/GPG, YubiKey GPG, or S/MIME signers. `sbxr` warns and disables automatic sandbox signing for those host configurations, so commits made in the sandbox are unsigned and must be signed or re-signed on the host.

Docker Sandboxes 0.39 introduced experimental Linux USB passthrough, which could make direct YubiKey GPG signing inside a microVM possible in the future. `sbxr` does not enable it: passing the whole USB device into an agent-controlled VM expands the trust boundary, may temporarily remove it from the host, and requires additional GnuPG, smart-card, and pinentry setup. Track the feature in Docker's [Sandbox release notes](https://docs.docker.com/ai/sandboxes/release-notes/); host-side signing remains the supported workflow.
