# sbxr

`sbxr` is an sbx (docker sandbox) wrapper that creates project-scoped
Rust development microVMs with Codex, Claude, Pi, and native VS Code Remote-SSH.
Its default preset is the complete multi-agent environment.

[Read the complete sbxr reference](README-EXTENDED.md).

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
Lifecycle commands start Docker's on-demand sandbox daemon when needed and
wait for it to become ready, so a sleeping daemon does not require a separate
`sbx daemon start` step.

## What it includes

The default `rust-multi-agent` preset combines:

- Docker's digest-pinned official Kits v3 Codex workload and Claude mixin,
  with host-managed ChatGPT and Claude subscription OAuth;
- pinned Rust 1.99.1 tooling, Cargo security tools, and a host-style colored,
  Git-aware Bash prompt, plus `nano`, Bash/Git completion, an explicit VS Code
  login-shell profile, and a generated `en_US.UTF-8` locale for clean remote
  terminals;
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

## Extended documentation

- [Prerequisites](README-EXTENDED.md#prerequisites)
- [Commands and CLI options](README-EXTENDED.md#commands)
- [Git inside or outside the sandbox](README-EXTENDED.md#git-inside-or-outside-the-sandbox)
- [Driving sandbox agents with Paseo](README-EXTENDED.md#driving-sandbox-agents-with-paseo)
- [Optional browser and Orca integrations](README-EXTENDED.md#optional-browser-and-orca-integrations)
- [Why sbxr?](README-EXTENDED.md#why-sbxr-from-project-folder-to-agent-ready-rust-ide)
- [Why a microVM instead of a container?](README-EXTENDED.md#why-a-microvm-instead-of-a-container)
- [Installation and bootstrap security](README-EXTENDED.md#installation-and-bootstrap-security)
- [Host setup behavior](README-EXTENDED.md#host-setup-behavior)
- [Optional host RocksDB acceleration](README-EXTENDED.md#optional-host-rocksdb-acceleration)
- [Presets](README-EXTENDED.md#presets)
- [Subscriptions and customization](README-EXTENDED.md#subscriptions-and-customization)
- [Source map](README-EXTENDED.md#source-map)

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

## Limitations

Docker's `.sbxenv.yaml`/`sbx env` and Kits v3 interfaces remain evolving in
Docker Sandboxes 0.51. sbxr centralizes those dependencies, validates both
runtime sides and required capabilities, and keeps structured compatibility
fixtures, but a future Docker release can still require a compatibility update.

After a microVM stop, VS Code Remote-SSH may restore terminal tabs but lose their scrollback due to a VS Code limitation/bug; [details and tested versions](PLATFORM_SUPPORT.md#vscode-state-across-a-microvm-stop).

Docker Sandboxes forwards SSH agents, but not OpenPGP/GPG, YubiKey GPG, or S/MIME signers. `sbxr` warns and disables automatic sandbox signing for those host configurations, so commits made in the sandbox are unsigned and must be signed or re-signed on the host.

The default shared workspace includes `.git`. `sbxr status`, `sbxr verify`, and
the hardened host-only signing command inspect repository-local executable
settings and hooks, but plain host `git` is not wrapped. See the
[shared Git trust model](README-EXTENDED.md#git-inside-or-outside-the-sandbox).

Docker Sandboxes 0.39 introduced experimental Linux USB passthrough, which could make direct YubiKey GPG signing inside a microVM possible in the future. `sbxr` does not enable it: passing the whole USB device into an agent-controlled VM expands the trust boundary, may temporarily remove it from the host, and requires additional GnuPG, smart-card, and pinentry setup. Track the feature in Docker's [Sandbox release notes](https://docs.docker.com/ai/sandboxes/release-notes/); host-side signing remains the supported workflow.
