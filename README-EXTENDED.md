# sbxr extended documentation

This is the complete sbxr reference. For the concise project overview and
quick start, see the [main README](README.md).

`sbxr` is an sbx (docker sandbox) wrapper that creates project-scoped
Rust development microVMs with Codex, Claude, Pi, and native VS Code Remote-SSH.
Its default preset is the complete multi-agent environment.


## Quick start

### Option 1: install a verified binary release

```bash
# Host Rust is not needed. These commands download and verify the installer;
# the installer discovers and installs the latest eligible release itself.
gh release download --repo danielemiliogarcia/sbxr --pattern install.sh --clobber
gh attestation verify install.sh \
  --repo danielemiliogarcia/sbxr \
  --signer-workflow danielemiliogarcia/sbxr/.github/workflows/release.yml \
  --deny-self-hosted-runners
sh install.sh
rm install.sh
```

### Option 2: install from reviewed source

```bash
# Requires Rust and Cargo 1.85 or newer.
git clone https://github.com/danielemiliogarcia/sbxr.git
cd sbxr
cargo install --frozen --path .
```

`cargo install` performs the optimized build and installs `sbxr`; a separate
`cargo build --release` is not required. The committed Cargo configuration
uses the vendored dependency tree without accessing the registry.

### Common setup

After installing by either method:

```bash
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
- Docker Sandboxes `sbx` 0.51.0 or newer (required for Kits v3). Follow Docker's official
  [installation and sign-in guide](https://docs.docker.com/ai/sandboxes/install/).
  Docker Desktop and Docker Engine are not required just to use `sbx`.
- A current GitHub CLI (`gh`) with `gh attestation verify` support for the
  verified binary installer, plus `gh` login for the default GitHub
  credential-proxy integration.
- Rust and Cargo 1.85 or newer only when building `sbxr` from source. They are
  not needed to run a prebuilt release.
- Native VS Code with the `code` command on `PATH`, plus an OpenSSH client.
- On Linux/X11, `wmctrl` is recommended so `sbxr stop` and `sbxr rm` can
  gracefully close only the matching Remote-SSH window (`sudo apt-get install
  wmctrl` on Ubuntu).
- Git is recommended and is required for `sbxr review`.
- For GitHub SSH push and sandbox commit signing, load a key in the host SSH
  agent (`ssh-add -L` must list it). The private key stays on the host.
- A ChatGPT subscription, Claude subscription, or both, depending on the
  selected preset. Authentication uses the providers' OAuth login flows; API
  keys are not required.

`sbxr setup` configures Docker sign-in, agent OAuth, and VS Code Remote-SSH
when needed. It reports missing host software with installation guidance, but
does not install system packages or invoke `sudo`.

Before lifecycle operations, sbxr parses Docker's client, server, state, and
API versions and probes the exact CLI capabilities it relies on. It rejects an
old client, an old or unavailable server, or a partially compatible command
surface with an actionable upgrade error instead of guessing across releases.

To build `sbxr` or contribute a feature, see the
[development guide](DEVELOPMENT.md).


## What it includes

The default `rust-multi-agent` preset combines:

- Docker's digest-pinned official Kits v3 Codex workload and Claude mixin,
  with host-managed ChatGPT and Claude subscription OAuth;
- pinned Rust 1.99.1 tooling, Cargo security tools, and a colored Git-aware
  Bash prompt, plus `nano`, Bash/Git completion, and a generated `en_US.UTF-8`
  locale for clean VS Code terminals;
- the native Claude Code binary shipped by Docker's official Claude mixin;
- pinned Pi, connected to the Docker-managed Codex and Claude OAuth proxies;
- the pinned Paseo CLI and a loopback-only Paseo daemon that host Paseo clients
  reach over the sandbox SSH transport;
- checksum-pinned Node.js 22.23.4 plus committed npm lockfiles for Pi and
  Paseo (and the optional browser); installation uses `npm ci
  --ignore-scripts` so runtime and transitive package resolution cannot drift
  during the first sandbox bootstrap;
- GitHub HTTPS/API access through Docker's host credential proxy, with only a
  non-reusable sentinel in the guest; forwarded GitHub SSH remains available;
- protected per-project profiles, redacted diagnostic reports, staged
  bootstrap checkpoints, and a resumable per-integration health ledger;
- VS Code Remote-SSH, including version-matched host extension installation;
- GitHub SSH push/pull and host-policy-matched SSH commit signing through
  Docker's forwarded host SSH agent;
- trusted-host Codex, Claude, and Pi extension/plugin and configuration
  mirroring, with credentials and runtime state excluded;
- the host's Bash aliases and `~/bin` utilities mirrored into the sandbox and
  placed on `PATH`, with opt-in Zsh/Fish, custom alias files, executable
  directories, and prompt styles.

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
| `sbxr new -y .` | Create/reuse without Docker's plan prompt, then open VS Code |
| `sbxr vscode .` | Explicit alias for `sbxr new .` |
| `sbxr update .` | Force host agent/VS Code mirroring, then open VS Code |
| `sbxr up .` | Create/reuse without attaching |
| `sbxr up -y .` | Create/reuse unattended without attaching or opening VS Code |
| `sbxr shell .` | Open a project shell |
| `sbxr codex .` | Run Codex |
| `sbxr claude .` | Run Claude Code |
| `sbxr pi .` | Run Pi in the multi-agent preset |
| `sbxr paseo .` | Start the sandbox Paseo daemon and print its SSH URL |
| `sbxr orca .` | Enable/start optional Orca Server and print its Desktop pairing URL |
| `sbxr isolated .` | Open a long-lived clone-mode sandbox with no writable host workspace mount |
| `sbxr wizard .` | Interactively configure optional integrations/resources, save, and open |
| `sbxr profile .` | Print the protected nonsecret profile |
| `sbxr auth-import .` | Explain why legacy credential imports are disabled |
| `sbxr auth-status .` | Check Codex, Claude, and Pi provider status |
| `sbxr auth renew openai\|anthropic\|github .` | Renew one host-proxied credential, then audit and verify it |
| `sbxr audit .` | Run cargo-audit, cargo-deny, and cargo-vet if configured |
| `sbxr verify .` | Audit runtime identity, guest credentials, tools, and SSH forwarding |
| `sbxr verify --live .` | Also make one minimal request through each selected model provider |
| `sbxr repair .` | Reconcile missing/outdated integrations and verify the result |
| `sbxr diagnose .` | Summarize the newest redacted report across all projects and selected project health |
| `sbxr commit-pgp-sign .` | OpenPGP-sign the current commit on the host |
| `sbxr commit-pgp-sign --all-mine .` | Sign your unsigned commits ahead of the upstream |
| `sbxr commit-pgp-sign --range BASE..HEAD .` | Sign your unsigned commits in an explicit range |
| `sbxr review .` | No-OAuth, clone-mode review sandbox |
| `sbxr inspect .` | Show the protected environment declaration and contract state |
| `sbxr name .` | Print the deterministic name |
| `sbxr status .` | Report sandbox state and the persisted integration-health summary |
| `sbxr stop .` | Stop while preserving state |
| `sbxr rm .` | Remove sandbox-local state; host files remain |
| `sbxr rm --force .` | Pass Docker's force flag while removing sandbox-local state |
| `sbxr setup` | Configure missing Docker login, provider OAuth, and Remote-SSH |
| `sbxr setup --prebuild` | Also warm the local content-addressed default kit-composition cache |
| `sbxr doctor [PATH]` | Diagnose the host; with a project compare host/guest versions |
| `sbxr --version` | Print the installed sbxr version |

### CLI options

| Option | Values/default | Effect |
|---|---|---|
| `-y`, `--yes-to-all`, `--force` | Off | Approve lifecycle plans and pass Docker `--force`; hardware-signing prompts remain interactive |
| `--wizard`, `--wizzard` | Off | Ask for all optional project settings; the misspelling remains a compatibility alias |
| `--config PATH` | None | Load, validate, and save a protected nonsecret JSON project profile |
| `--shared`, `--isolated` | Shared | Use the writable host workspace or a mountless sandbox-local clone; changing it requires recreation |
| `--network PROFILE` | `development` | Select `development`, `restricted`, or `offline`; reconciled in place |
| `--memory SIZE`, `--cpus N` | Docker defaults | Set microVM resources; changing them requires recreation |
| `--root-disk SIZE`, `--docker-disk SIZE` | Docker defaults | Set guest root/nested-Docker disk sizes; changing them requires recreation |
| `--browser`, `--no-browser` | Off | Enable or disable the isolated Playwright MCP; changing it requires recreation |
| `--orca`, `--no-orca` | Off | Enable or disable Orca Server; changing it requires recreation |
| `--ghcr`, `--no-ghcr` | Off | Enable or disable host-proxied private GHCR access; changing it requires recreation |
| `--paseo-relay`, `--no-paseo-relay` | Off | Enable or disable Paseo's authenticated relay; reconciled in place and incompatible with `offline` |
| `--shell-profile PROFILE` | `familiar` | Select `familiar`, `minimal`, or `custom`; reconciled in place |
| `--login-shell SHELL` | `bash` | Select `bash`, `zsh`, or `fish`; changing it requires recreation |
| `--prompt STYLE` | `classic` | Select `classic`, `plain`, or `custom`; reconciled in place |
| `--prompt-file PATH` | None | Select a trusted custom prompt file below the host home and imply a custom prompt |
| `--alias-file PATH` | None; repeatable | Select a trusted alias file and imply the custom shell profile |
| `--bin-dir PATH` | None; repeatable | Select a trusted executable directory and imply the custom shell profile |
| `--preset PRESET` | `rust-multi-agent` | Select `rust-multi-agent`, `rust-codex`, or `rust-claude`; names and contracts are preset-specific |
| `--rocksdb-host[=PREFIX]` | Off | Mount a trusted host RocksDB installation read-only; changing it requires recreation |

Command-specific options are shown in the command table: `setup --prebuild`,
`verify --live`, `commit-pgp-sign --all-mine`, `commit-pgp-sign --range`, and
`rm --force`. Options may appear before or after the command.

`-y`, `--yes-to-all`, and `--force` are equivalent. On commands that may create
or start a sandbox, they pass `--force` to `sbx env run` and skip Docker's
environment-plan approval prompt. On `sbxr rm`, they pass `--force` to
`sbx env rm`. On multi-commit `commit-pgp-sign`, they approve the printed
history-rewrite plan but do not suppress GPG/YubiKey interaction. They do not
copy credentials or perform a missing OAuth login.

Normal launch keeps the opinionated defaults: shared host workspace, the
multi-agent Rust environment, development networking, host-proxied GitHub,
forwarded SSH, and the existing editor/shell integrations. Browser, Orca,
GHCR, the Paseo relay, isolated mode, and tighter network profiles are opt-in.
Run `sbxr wizard .` only when you want to choose them interactively, or use
direct flags:

```bash
sbxr --browser --memory 8g --cpus 4 new -y .
sbxr --network restricted up -y .
sbxr --ghcr up -y .
sbxr --paseo-relay up -y .
sbxr --login-shell zsh --alias-file ~/.config/zsh/aliases.zsh new -y .
```

Selections are validated and saved as nonsecret JSON under the protected sbxr
state directory. Workspace, integration, and VM-resource selections become
part of the creation-time environment contract; changing one requires the
explicit `sbxr rm`/`new` sequence printed by sbxr. Network policy and the Paseo
relay are runtime-reconciled and do not require VM recreation. `development`
retains ordinary kit and project egress. `restricted` blocks package/toolchain
download hosts after bootstrap, and `offline` blocks all sandbox egress after
bootstrap. sbxr records only the rule IDs it created in protected host state,
removes only those IDs when the profile changes, and never resets user-created
or host-global policy.

## Git inside or outside the sandbox

In the default direct mode, the host and sandbox see the same working tree and
the same `.git` directory. You may inspect, stage, commit, and push from either
side; do not run competing Git mutations on both sides at the same time.

On first bootstrap, `sbxr` installs `nano` as Git's editor, enables Bash/Git
completion, mirrors the host's global `user.name` and `user.email`, installs
GitHub's published SSH host keys, and permits GitHub SSH traffic. Docker
Sandboxes automatically forwards SSH authentication and SSH-format signing
requests to the host agent, so private SSH keys are not copied into the
microVM. See Docker's
[Git sandbox workflow and signing
model](https://docs.docker.com/ai/sandboxes/workflows/git/).
If the host's `SSH_AUTH_SOCK` is stale, `sbxr` automatically selects a live
standard desktop agent with loaded keys (such as GCR or GnuPG) before starting
Docker Sandboxes; `sbxr doctor` reports when this fallback is active.

Cargo Git dependencies use the system Git CLI so they can authenticate through
that forwarded agent. `sbxr` also mirrors explicit host HTTPS-to-SSH rewrites
only when both sides are the same path on `github.com`; arbitrary URL rewrites
and rewrites to other hosts are rejected. This lets a host rule such as
`https://github.com/FairgateLabs/` → `git@github.com:FairgateLabs/` work inside
the sandbox without copying a GitHub token or private key.

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

For an OpenPGP/GPG YubiKey, create the commit in the sandbox and sign it from
the host-mounted checkout:

```bash
sbxr commit-pgp-sign .
```

The command refuses staged changes or an in-progress merge/rebase operation,
disables repository Git hooks for the signing-only amend, forces Git's OpenPGP
backend, and verifies that the resulting `HEAD` contains an OpenPGP signature.
Git invokes the host GPG agent, so the YubiKey, PIN entry, touch confirmation,
and private key remain on the host. Unstaged and untracked files are preserved.

To sign a linear suffix of unpublished commits authored with the repository's
effective `user.email`, use:

```bash
sbxr commit-pgp-sign --all-mine .
# Or, when there is no upstream or a narrower boundary is wanted:
sbxr commit-pgp-sign --range origin/main..HEAD .
```

`--all-mine` examines only commits ahead of the current branch's upstream and
starts at the earliest unsigned matching commit. Adding a signature changes a
commit ID, so every descendant in that suffix is recreated and re-signed,
including descendants that were already signed. History mode therefore
requires a completely clean worktree, an attached branch, no merge/root commit
in the rewritten suffix, and no differently authored descendant. Before the
rewrite it prints every affected commit and asks for confirmation; `-y` skips
that confirmation but never bypasses YubiKey PIN/touch prompts.

sbxr creates a `refs/sbxr/commit-pgp-sign/...` recovery ref, disables project
hooks, verifies the final tree and every resulting OpenPGP signature, and
automatically aborts to the original branch if signing fails. It never pushes.
If the old commits were already published, inspect the result carefully and
run `git push --force-with-lease` yourself; this rewrite cannot be a
fast-forward.

GitHub HTTPS/API requests can use the Docker host proxy without exposing the
host token, while GitHub SSH remains allowed for existing workflows. Other Git
hosts need their own reviewed network/known-host kit. Any process in a connected sandbox can ask
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

`sbxr verify` operates on an existing sandbox. It validates Docker's reported
agent, selected kits, empty runtime-mount list, and workspace; audits known
Codex, Claude, Pi, GitHub, Docker, and VS Code credential stores for reusable
values; performs a real, non-generation Anthropic API request through the
sentinel proxy; verifies pinned tool versions; and checks the forwarded SSH
agent without printing public keys. The audit accepts only the exact managed
credential schemas, checks supported custom credential homes, rejects unsafe
owners, permissions, symlinks, and malformed stores, and never scans project
files or deletes findings. Normal reconciliation validates the managed store
paths and enforces `0600` on the Codex, Claude, and Pi sentinel files before
verification. `sbxr verify --live` additionally makes minimal
Codex, Claude, and Pi model requests, suppresses provider output, and repeats
the credential audit afterward. It is opt-in because it consumes a small
amount of subscription usage.

Every verification stage is recorded in a protected host-side integration
health ledger as `pending`, `verified`, `skipped`, or `needs attention`.
`sbxr status` shows it without changing the sandbox. `sbxr repair` safely
reruns idempotent reconciliation for missing or outdated integrations and then
verifies the result; it does not make live model requests.
Removing a sandbox also removes its health ledger so a later recreation cannot
inherit stale verification results; the nonsecret project profile is retained.

Lifecycle commands write a redacted JSON report under
`$SBXR_STATE_ROOT/diagnostics`. Reports contain stage/status categories only:
no arguments, project paths, environment, raw errors, command output, pairing
links, or credential values. `sbxr diagnose .` labels and summarizes the newest
report across all projects, then displays the selected project's health ledger.

When the matching Remote-SSH window is open, `sbxr stop` and `sbxr rm` ask that
exact VS Code window to close normally and wait before stopping or removing the
microVM. This gives
VS Code time to serialize terminal tabs, process metadata, command history, and
scrollback; other VS Code windows are left alone. An unsaved-file prompt must
be resolved before stopping proceeds. Linux/X11 uses `wmctrl` to address the
exact window; if safe targeted closing is unavailable, `sbxr` leaves the
sandbox in place and asks for a manual close. This also prevents a later
recreation with the same sandbox name from reusing a stale Remote-SSH window.
On the next `sbxr new`, VS Code
uses its process-revive support to recreate terminal shells from the saved
session metadata, with up to 10,000 lines requested for persistent scrollback.
Long-running processes cannot continue while the microVM is powered off.
Terminal-tab, working-directory, and scrollback restoration ultimately depends
on the installed VS Code/Remote-SSH release; see
[Platform support](PLATFORM_SUPPORT.md#vscode-state-across-a-microvm-stop).

No custom environment variables are required. Optional preset, naming,
storage, mirroring, RocksDB, and authentication-import overrides are documented
in [ENVIRONMENT.md](ENVIRONMENT.md).

## Driving sandbox agents with Paseo

Every development sandbox installs the pinned Paseo CLI and a daemon bound to
the sandbox's own loopback interface. The daemon's external relay is disabled,
so it is reachable only through the sandbox SSH transport that Docker Sandboxes
already provides:

```bash
sbxr paseo .
# ssh://agent@rust-multi-my-project-1a2b3c4d.sbx
```

`sbxr paseo` creates or reuses the project sandbox, starts the daemon when it is
not already running, and prints that URL. Pass it to a host Paseo client, or
paste it into the Paseo app as a remote daemon:

```bash
paseo --host ssh://agent@rust-multi-my-project-1a2b3c4d.sbx ls
```

The client opens `ssh -W 127.0.0.1:6767` to the sandbox, so the daemon port is
never published on the host or the network, and the connection reuses the
managed `*.sbx` SSH configuration written by `sbx setup ssh`. Paseo's own SSH
transport does not install or start a remote daemon; `sbxr paseo` is what starts
it. Agents Paseo launches there are the sandbox's own Claude, Codex, and Pi
installations, with the sandbox's network policy and file access.

For authenticated access when direct SSH is unavailable, opt in with
`--paseo-relay` or the wizard. sbxr updates Paseo's sandbox-local configuration
atomically, restarts an already-running daemon only when that setting changed,
and allows only `relay.paseo.sh:443` for that sandbox. Disabling it removes that
scoped rule. The fast default remains relay-off and the SSH URL remains usable
when the relay is enabled. Relay mode is incompatible with the `offline`
network profile.

## Optional browser and Orca integrations

`--browser` installs the committed `@playwright/mcp` 0.0.83 lock and its exact
Playwright/Chromium dependency inside a Codex-capable sandbox. npm lifecycle
scripts remain disabled; Chromium is installed explicitly, uses a fresh
in-memory profile for every MCP process, trusts only the sandbox proxy CA, and
does not share host browser cookies or a control port. The integration is
registered as `sbxr-browser` after trusted Codex configuration mirroring.

`sbxr orca .` enables the optional Orca kit in the protected project profile,
installs the checksum-pinned Orca 1.4.247 AppImage by extracting it without
FUSE, installs its guest-only headless prerequisites including Xvfb and
`xauth` plus Electron's NSPR runtime, starts `orca serve` as the unprivileged
sandbox user, and prints its pairing URL. Re-running the command repairs missing prerequisites without
downloading an already verified AppImage again. The server is published only
to a Docker-allocated ephemeral host loopback port. sbxr discovers the actual
mapping, fails closed if it is not loopback-only, and rewrites the endpoint
embedded in the pairing payload to that port. Paste the URL into Orca Desktop
under **Settings → Remote Orca Servers → Add Server**. Treat the pairing URL as
a secret and do not publish the loopback port through a public tunnel.

Orca's data volume survives a sandbox recreation. If its lock belongs to the
previous microVM, sbxr first asks Orca to perform its guarded stale-lock
recovery. For the expected changed-machine-identity refusal, sbxr clears only
the sandbox-local `orcad.lock` after validating its type, owner, permissions,
PID, and Linux process start time. A lock matching a live process, or any other
recovery refusal, remains untouched and `sbxr orca` fails closed.

Both integrations are disabled by default and become creation-time contract
inputs when enabled.

### Upgrade note for existing sandboxes

The Unreleased environment contract replaces Orca's fixed host port with a
Docker-allocated loopback port and explicitly disables shared host skills in
review sandboxes. An Orca-enabled or review sandbox created by an older sbxr
must therefore be removed and recreated when sbxr reports contract drift. Run
`sbxr rm PATH`, then `sbxr new -y PATH` (or rerun `sbxr review PATH`) as
appropriate. Shared-workspace source files remain on the host.

This release also moves network choice out of the creation-time fingerprint,
which causes a one-time contract migration for existing development sandboxes.
After recreating once, later switches among `development`, `restricted`, and
`offline` are applied in place.

With Docker Sandboxes 0.51, the Orca loopback host port changes across a normal
`sbxr stop`/start as well as recreation. Rerun `sbxr orca PATH` after a restart
and replace the old Orca Desktop remote-server entry with the newly printed
pairing URL.

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
- automatic installation/selection of requested `rust-toolchain.toml` or
  `rust-toolchain` toolchains and components, including nested repositories in
  a multi-project workspace; reconciliation rechecks that every selected
  compiler remains runnable before trusting its cached marker;
- a local-like Rust editing experience in VS Code: remote Rust Analyzer powers
  completion, diagnostics, hover information, find references, go to
  definition, and source navigation across the project and available imported
  crate sources;
- an immediately familiar colored Bash prompt with the current Git branch;
- `nano` as a working Git editor, Bash/Git completion, mirrored Git author
  identity and signing defaults, GitHub push/pull through the host SSH agent,
  and SSH signatures when requested by that policy or `-S`;
- installed Codex, Claude, and Pi extensions/plugins plus instructions, hooks,
  prompts, themes, and status-line customizations mirrored from the trusted
  host profile; Docker's environment-provided shared agent skills stay on
  their read-only guest mounts;
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

The Docker Engine inside the microVM remains Docker Sandboxes' normal
root-capable guest engine. `sbxr` treats the microVM—not an additional
rootless-engine layer—as the host security boundary, so containerized builds
and end-to-end tests retain broad Docker compatibility out of the box. Guest
root remains powerful inside the sandbox and can still damage sandbox-local
state or the explicitly shared workspace.

## Local kit-composition cache, not a personalized registry image

`sbxr` deliberately does not publish or snapshot a personalized machine
image. Docker's digest-pinned official agent workloads provide the reviewed
base, while the Kits v3 specifications committed in this repository are the
deterministic generic recipe for Rust, Node, CLI, editor, and shell
prerequisites. Docker Sandboxes assembles source-form kit compositions into
content-addressed images in its private local runtime and reuses a matching
composition across sandbox creation. A normal `sbxr new` warms this cache
automatically.

`sbxr setup --prebuild` is an optional way to move kit assembly ahead of the
first project creation. It creates a workspace-free disposable sandbox from
the pinned default composition, removes that sandbox afterward, and leaves
Docker's composition image cached. It neither requires Docker Engine/Desktop
on the host nor exports or pushes an image.
Personal aliases, prompt files, executable directories, agent settings, VS
Code extensions, project files, Git identity, and credentials are applied only
after a real sandbox exists and are never cache inputs. If prebuilding is not
supported or desired, the ordinary official-workload-plus-kits path remains
the complete fallback.

This cache avoids repeating kit **assembly**, not every provisioning step.
Each new microVM still receives fresh persistent volumes and runs the kit's
idempotent install hooks to initialize them. In particular, Docker's saved
templates omit mounted filesystems, so snapshotting the warm-up sandbox would
drop sbxr's volume-backed Rust toolchains and Pi state. sbxr therefore never
uses a saved template as a shortcut. Docker owns cache reclamation; the broad,
destructive `sbx reset` command clears the image cache along with all sandbox
state and authentication.

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

### Verified prebuilt release (recommended)

Release archives contain the single native `sbxr` executable and its license,
security, credential-architecture, dependency-review, and release-manifest
documents, including the vendored dependencies' license texts. Rust is not
required on the host. Docker's separately distributed `sbx` CLI remains a
prerequisite.
Verified prebuilt installation is available for current releases. When no
version is passed, the installer selects the newest release that contains the
appropriate platform installer instead of selecting an older source-only
release. Pass an exact version as the first argument, or through
`SBXR_VERSION`, to pin it.

On Linux x86-64/ARM64 and Apple-silicon macOS, download and verify the installer
before executing it:

```bash
gh release download --repo danielemiliogarcia/sbxr --pattern install.sh --clobber
gh attestation verify install.sh \
  --repo danielemiliogarcia/sbxr \
  --signer-workflow danielemiliogarcia/sbxr/.github/workflows/release.yml \
  --deny-self-hosted-runners
sh install.sh
rm install.sh
```

For an exact release, invoke the verified installer as `sh install.sh VERSION`.
It then requires the archive attestation to come from that exact tag.

The default destination is `~/.local/bin/sbxr`. Set `SBXR_INSTALL_DIR` to an
alternative. The installer detects the host target, downloads only its matching
archive and `SHA256SUMS`, verifies the archive's GitHub build attestation and
SHA-256, checks `sbxr --version`, and atomically replaces a non-symlink
destination.

On Windows x86-64, use PowerShell:

```powershell
gh release download --repo danielemiliogarcia/sbxr --pattern install.ps1 --clobber
gh attestation verify .\install.ps1 `
  --repo danielemiliogarcia/sbxr `
  --signer-workflow danielemiliogarcia/sbxr/.github/workflows/release.yml `
  --deny-self-hosted-runners
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
Remove-Item .\install.ps1
```

Pass `-Version VERSION` to the verified PowerShell installer to select an exact
release.

The Windows default is `%LOCALAPPDATA%\sbxr\bin\sbxr.exe`;
`SBXR_INSTALL_DIR` selects another directory.

### Manual binary installation

The installer is optional. Download the archive matching the host directly
from the release page together with `SHA256SUMS`:

| Host | Release archive |
|---|---|
| Linux x86-64 | `sbxr-VERSION-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `sbxr-VERSION-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple silicon | `sbxr-VERSION-aarch64-apple-darwin.tar.gz` |
| Windows x86-64 | `sbxr-VERSION-x86_64-pc-windows-msvc.zip` |

Verify the selected archive before extracting it:

```bash
gh attestation verify sbxr-VERSION-TARGET.ARCHIVE \
  --repo danielemiliogarcia/sbxr \
  --signer-workflow danielemiliogarcia/sbxr/.github/workflows/release.yml \
  --source-ref refs/tags/VERSION --deny-self-hosted-runners
sha256sum --check --ignore-missing SHA256SUMS
```

Then extract the target-specific directory and copy `sbxr` to a directory on
`PATH`, such as `~/.local/bin`. On macOS use `shasum -a 256`; on Windows use
`Get-FileHash -Algorithm SHA256` and compare it with `SHA256SUMS`.

Checksums detect corruption. The GitHub attestation additionally binds an
artifact to this repository, its tag, and the pinned release workflow. Neither
is a substitute for trusting that reviewed source and workflow.

Before any official build starts, release CI verifies that the version tag is
an annotated OpenPGP tag signed by the allowlisted YubiKey signing key, points
to the workflow commit, and is reachable from `master`. GitHub then makes the
published release immutable, so its tag and assets cannot subsequently be
moved, replaced, or deleted. The committed public key and exact accepted
fingerprints are auditable in `.github/release-signing-key.asc` and
`scripts/verify-release-tag.sh`.

### Build from source

Convenient source installation or upgrade from the latest public `master`
branch:

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

`publish = false` prevents a misleading `cargo install sbxr` claim: no
crates.io package exists. Tagged release binaries are built with the exact
stable compiler recorded in `RUST_RELEASE_VERSION`, currently Rust 1.99.1,
while `rust-version = 1.85` remains the tested minimum for source builders.
Every release build uses `--offline --frozen` against the committed vendor
tree, runs natively on its target GitHub-hosted runner, smoke-tests the binary
and installer, and is published only after all targets succeed. The binary
automatically writes its embedded kits under the platform data directory;
users do not need Rust or a separate kit checkout.

## Host setup behavior

`sbxr setup` is idempotent. It requires Docker Sandboxes 0.51.0 or newer,
detects and configures missing
interactive prerequisites: Docker sign-in, host-managed OpenAI OAuth for
presets that use Codex, the presence of Docker-managed Anthropic OAuth for
presets that use Claude, Docker-managed `*.sbx` SSH configuration, and the VS
Code Remote-SSH extension. Those steps may
open a browser or download the extension. It verifies the result of each
command and continues far enough to report multiple problems together. It
never installs system packages or invokes `sudo`; missing `sbx`, VS Code,
OpenSSH, Git, or KVM still receive explicit manual installation guidance.
Passing `--prebuild` runs the same checks first and then warms only the generic
local Kits v3 composition cache. It shifts composition work ahead of the first
project, but fresh volume initialization and install-hook verification still
occur when each project sandbox is created.

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
| Anthropic OAuth missing for `rust-multi-agent` or `rust-claude` | Explains the one-time proxy-captured `/login` flow; no host Claude credential is copied |
| Docker `*.sbx` SSH configuration | Safely reruns `sbx setup ssh` to refresh/verify it |
| VS Code installed but Remote-SSH missing | Installs the Microsoft extension, then verifies it |
| `sbx`, VS Code, OpenSSH, or KVM missing | Prints installation guidance and exits non-zero |
| Git missing | Warns that review mode is unavailable |
| SSH agent has no loaded key | `doctor` warns that sandbox GitHub authentication/signing will be unavailable |
| Everything already configured | Reports success without changing anything |

Use `sbxr --preset rust-claude setup` when configuring a Claude-only
installation; it skips OpenAI OAuth. Start Claude with `sbxr claude .` and use
its native `/login` flow. Docker intercepts that OAuth exchange and keeps the
real access and refresh tokens on the host; only proxy sentinel values are
written inside the sandbox. `sbxr` never imports a host Claude credential file.

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
adopts that sandbox. The contract covers the rendered environment, every
digest-pinned official workload/mixin reference, and the contents of every
selected local kit, so a kit update also requires explicit recreation instead
of silently reusing stale provisioning.

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
opening unfamiliar code with additional safeguards. It also tells Docker not
to mount shared host agent skills, in addition to using a mountless clone,
omitting agent OAuth and GitHub SSH, and skipping host configuration and
extension mirroring.

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

Codex and Claude both use Docker's host-managed OAuth proxy. The generated
environment explicitly approves only the OAuth token endpoint and provider API
host required by the Claude kit. The real OpenAI and Anthropic tokens are not
copied into the VM; Codex, Claude, and Pi see provider-shaped sentinel values
that Docker replaces only on requests to the approved hosts. The legacy
`sbxr auth-import` command is intentionally disabled so it cannot bypass this
boundary.

For the complete service-by-service data flow, SSH-agent distinction,
unsupported-secret limits, and verification procedure, see
[Credential architecture](CREDENTIAL_ARCHITECTURE.md).

Pi recognizes both proxy sentinels. `pi --provider openai-codex` uses the same
ChatGPT subscription and `pi --provider anthropic` uses the same host-managed
Claude login without receiving either real token. Docker owns refresh and
routing, so Claude and Pi no longer synchronize credential files with each
other. Pi documents third-party Claude OAuth use as Anthropic “extra usage,”
potentially billed per token rather than against the normal plan allowance.

The launcher mirrors the installed extension/plugin payloads in each agent's
documented configuration tree, plus useful host configuration: instructions,
skills, hooks, prompts, status-line scripts (including a script referenced from
Claude's `statusLine.command`), themes, and plugin metadata/cache. Codex and
Claude plugin caches are copied; Pi source extensions are copied and Pi npm
extension packages are resolved inside the sandbox from the semver ranges in
the trusted host `package.json`, with lifecycle scripts disabled. The host
lockfile is deliberately not reused because it targets the host's Pi release.
The mirror excludes authentication files, histories, sessions, logs, project
trust records, runtime state, and Codex system-managed skills. Host paths in
copied text are rewritten to `/home/agent`, while Docker-managed provider and
safety settings win during merges. Credential-shaped fields are removed from
mirrored Claude settings, and literal API keys or credential headers are
removed from Pi model configuration; strict environment-variable references
remain available without copying their values.

Remote VS Code extensions are installed in bounded batches rather than one
unbounded CLI invocation. This releases installer state between batches and
avoids large host extension inventories exhausting the sandbox or host while
preserving exact version mirroring and the existing individual retry pass.

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
the semver ranges declared in the host `package.json`, resolved inside the
sandbox against its own kit-pinned Pi release, instead of copying a host
`node_modules` tree built for a potentially different Pi or platform. The host
`package-lock.json` is deliberately not reused: those exact builds are selected
against the host's Pi version, and an extension pinned that way can call an API
the sandbox's Pi no longer exports, which makes every `pi` start fail to load
it. Set `SBXR_SYNC_PI_EXTENSIONS=0` to omit raw source extensions; package
declarations still follow the mirrored Pi settings. Details are in
[ENVIRONMENT.md](ENVIRONMENT.md#mirroring-controls).

A mirrored symbolic link is kept as a link only while it resolves inside the
same mirrored tree. One pointing anywhere else—a Pi extension developed in a
separate host checkout, for instance—is copied by content under the link's own
name, because the sandbox has no copy of the path it names. A link the host
itself cannot resolve is skipped.

The default `familiar` shell profile preserves the existing behavior: it
extracts alias definitions from `~/.bashrc`, mirrors `~/.bash_aliases` and
`~/bin` into the sbxr-owned guest folder `~/.local/share/sbxr/host-shell`, and
appends the mirrored `bin` to `PATH` so sandbox-managed toolchains keep
priority. The rest of `.bashrc` is not adopted. Choose Bash, Zsh, or Fish and
configure the overlay through the wizard or direct options:

```bash
# Clean sbxr shell with no default host alias/bin mirror.
sbxr --shell-profile minimal --prompt plain new -y .

# Mirror only explicitly selected Zsh inputs.
sbxr --shell-profile custom --login-shell zsh \
  --alias-file ~/.config/zsh/aliases.zsh \
  --bin-dir ~/scripts \
  --prompt-file ~/.config/zsh/prompt.zsh new -y .
```

`--alias-file` and `--bin-dir` are repeatable and automatically select the
`custom` profile; `familiar` uses only conventions for the selected shell and
`minimal` uses no alias or executable paths. Explicitly selected paths must
remain below the host home and retain their relative suffix below the sbxr-owned guest mirror
`~/.local/share/sbxr/host-shell`. sbxr refuses paths that overlap known
credential stores (including `.ssh`, `.gnupg`, `.config/gh`, `.docker`, agent
credential/config homes, and protected sbxr state), and shell trees never
follow symlinks that escape the selected tree. Top-level selections must be
regular files/directories whose resolved targets remain below the host home;
an automatically discovered familiar-profile alias file or `bin` directory
whose target leaves the home is skipped with a warning instead of blocking
the sandbox. Explicit external selections remain errors. Escaping nested links
are skipped. This is a defense-in-depth path policy, not
a content secret scanner: selected files must still be trusted and nonsecret.
`--prompt` accepts
`classic`, `plain`, or `custom`; `--prompt-file` selects `custom`
automatically. Zsh and Fish are installed only when selected. Changing the
login-shell program changes the kit contract and requires recreation; changing
the overlay is reconciled in place. sbxr fingerprints the selected host files
and directories on every ensuring command, replaces changed content, and
removes paths that are no longer selected; `sbxr update` still forces a full
refresh.
Set `SBXR_SYNC_HOST_SHELL=0` to skip the overlay entirely.

Mirrored hooks, plugins, skills, and extensions are executable software. Only
mirror them from a trusted host profile.

## Source map

- `README.md`: concise overview, installation paths, and common setup;
- `README-EXTENDED.md`: complete feature and operational reference;
- `DEVELOPMENT.md`: build, architecture, feature workflow, and verification
  guide;
- `src/main.rs`: minimal process entrypoint and module wiring;
- `src/app.rs`: top-level command dispatch;
- `src/cli.rs`: argument parsing, presets, actions, and help text;
- `src/profile.rs`: protected profiles, resource validation, and wizard;
- `src/diagnostics.rs`: redacted structured reports;
- `src/health.rs`: protected resumable integration-health records;
- `src/environment.rs`: runtime context, project identity, sandbox names, and
  optional host RocksDB validation;
- `src/envfile.rs`: protected declarative environments and desired/applied
  contract state;
- `src/sbx.rs`: the centralized Docker Sandboxes CLI boundary, strict runtime
  schema parsing, capability probes, and loopback-port discovery;
- `src/sandbox.rs`: development/review orchestration, remote commands, and
  Cargo audits on top of declarative lifecycle;
- `src/host.rs`: setup, preflight checks, and diagnostics;
- `src/auth.rs`: disabled legacy credential import, subscription status,
  supported renewal flows, and proxy-route verification;
- `src/sign.rs`: host-only OpenPGP commit signing;
- `src/verify.rs`: redacted runtime and guest-credential verification;
- `src/node_tools.rs`: embedded lockfile upload, exact Node-tool installation,
  and installation-state hashing;
- `src/github_proxy.rs`, `src/browser.rs`, and `src/orca.rs`: optional/default
  integration bootstrap without host credential copies;
- `kits/`: embedded declarative sandbox mixins, including `host-shell`,
  `bash-shell`, `zsh-shell`, and `fish-shell` for profile-selected shell integration and
  `paseo-cli` for the pinned Paseo CLI and its loopback-only daemon;
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

Docker's `.sbxenv.yaml`/`sbx env` and Kits v3 interfaces remain evolving in
Docker Sandboxes 0.51. sbxr centralizes those dependencies, validates both
runtime sides and required capabilities, and keeps structured compatibility
fixtures, but a future Docker release can still require a compatibility update.

After a microVM stop, VS Code Remote-SSH may restore terminal tabs but lose their scrollback due to a VS Code limitation/bug; [details and tested versions](PLATFORM_SUPPORT.md#vscode-state-across-a-microvm-stop).

Docker Sandboxes forwards SSH agents, but not OpenPGP/GPG, YubiKey GPG, or S/MIME signers. `sbxr` warns and disables automatic sandbox signing for those host configurations, so commits made in the sandbox are unsigned and must be signed or re-signed on the host.

Docker Sandboxes 0.39 introduced experimental Linux USB passthrough, which could make direct YubiKey GPG signing inside a microVM possible in the future. `sbxr` does not enable it: passing the whole USB device into an agent-controlled VM expands the trust boundary, may temporarily remove it from the host, and requires additional GnuPG, smart-card, and pinentry setup. Track the feature in Docker's [Sandbox release notes](https://docs.docker.com/ai/sandboxes/release-notes/); host-side signing remains the supported workflow.
