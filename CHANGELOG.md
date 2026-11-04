# Changelog

## Unreleased

## [0.3.1] - 2026-11-04

### Changed

- Migrated every active local integration to Docker Sandboxes Kits v3 and
  pinned the official Codex, Claude, Claude mixin, and Shell workloads by OCI
  digest.
- Multi-agent sandboxes now compose the official Codex workload with Docker's
  official Claude mixin, so OpenAI and Anthropic OAuth remain behind their
  host proxies. Removed sbxr's duplicated npm-installed Claude distribution.
- Added an actual Anthropic proxy request to `auth-status` and `verify`; a
  sentinel-shaped Pi auth file alone is no longer reported as proof that the
  proxy route works.
- Declared SSH-agent forwarding explicitly through the v3 capability while
  retaining sbxr's current unrestricted compatibility behavior.
- Added a Kits v3 checksum-pinned Node.js 22.23.4 runtime because the official
  hardened workloads no longer guarantee a guest Node installation.
- Stopped capability sync from writing through Docker 0.51's read-only Claude
  skills mount; environment-provided skills remain available there directly.
- Isolated root-run npm bootstrap caching from the agent's writable npm cache,
  preventing ownership failures during Pi extension synchronization.
- Bumped `sbxr` to 0.3.1 and raised the minimum Docker Sandboxes version to
  0.51.0, the first supported release for this Kits v3 composition.

### Fixed

- Install the `xauth` prerequisite required by Orca's `xvfb-run` launcher and
  repair existing Orca-enabled sandboxes without downloading the pinned
  AppImage again.
- Adapted provisioning to hardened workload images that omit `/etc/locale.gen`,
  `hostname`, and the Node.js runtime, and made GitHub SSH host-key setup
  independent of v2 kit file-copy ordering.

## [0.3.0] - 2026-11-03

### Added

- Added protected, validated, nonsecret per-project profiles plus opt-in
  `--wizard`/`--wizzard`. Normal `sbxr new -y` keeps the fast current defaults.
- Added shared/development defaults with optional mountless `isolated` clone
  mode; `development`, `restricted`, and `offline` sandbox-scoped network
  profiles; and checked memory, CPU, root-disk, and Docker-disk overrides.
- Added host-proxied GitHub HTTPS/API authentication by default while retaining
  SSH-agent forwarding. Optional GHCR registry credentials are also proxied by
  Docker and never copied into the guest.
- Added the optional lockfile-pinned Playwright MCP with an isolated Chromium
  profile and the optional checksum-pinned Orca 1.4.247 server. `sbxr orca`
  starts it and prints the private Orca Desktop pairing link.
- Added redacted structured diagnostic reports, per-stage bootstrap markers,
  exact pinned-tool verification, selected-kit/runtime-mount checks, and
  `sbxr doctor PATH` host-versus-sandbox version reporting.
- Rust toolchain reconciliation now discovers nested repositories in a
  multi-project workspace before Rust Analyzer or other editor tools start.

### Security

- Docker receives GitHub/GHCR host commands only from protected launcher state;
  guests receive proxy sentinels. The guest audit rejects reusable GitHub,
  provider, Docker, editor, and agent credentials without printing values.
- Optional browser packages use a committed lockfile and `npm ci
  --ignore-scripts`; Chromium uses an ephemeral profile, the sandbox proxy,
  and the proxy CA without sharing host browser state.
- Profiles and diagnostics use atomic `0600` files below protected `0700` host
  directories and reject secret-looking or unknown profile fields.

### Changed

- Bumped `sbxr` to 0.3.0. The shared workspace, current integrations, broad
  development network, and forwarded SSH agent remain the defaults.

## [0.2.7] - 2026-11-03

### Added

- `sbxr commit-pgp-sign --all-mine [PATH]` now signs the current author's
  unsigned commits ahead of the branch upstream. `--range BASE..HEAD` provides
  an explicit boundary for branches without an upstream or for a narrower
  rewrite.

### Security

- History signing requires a clean, attached, linear branch suffix owned by
  the effective Git `user.email`. It prints the non-fast-forward plan, requires
  confirmation unless `-y` is supplied, disables repository hooks, creates a
  recovery ref, verifies every rewritten OpenPGP signature and the final tree,
  and never pushes. A failed signing operation automatically aborts the rebase
  back to the original branch.

### Changed

- Bumped `sbxr` to 0.2.7. SSH-agent forwarding remains enabled and unchanged.

## [0.2.6] - 2026-11-03

### Added

- Added `sbxr commit-pgp-sign [PATH]`, which performs a host-only,
  hook-disabled signing-only amend using Git's OpenPGP backend. This supports
  host GPG agents and OpenPGP YubiKeys without forwarding the USB device or
  private key into the sandbox.
- Added `sbxr verify [PATH]`, a read-only audit of an existing sandbox's
  runtime identity, shared workspace, supported credential stores, installed
  tool versions, and forwarded SSH-agent availability. Findings identify
  locations without printing credential values.
- Projects with `rust-toolchain.toml` or `rust-toolchain` now automatically
  install/select that requested toolchain before agent and editor bootstrap.

### Security

- Claude, Pi, and Paseo are now installed with per-kit committed npm
  lockfiles and `npm ci --ignore-scripts`. Narrow transitive overrides remove
  the advisories present in a fresh registry resolution; all locked packages
  passed npm registry-signature verification.
- Pi's managed Codex and Anthropic entries are reasserted as Docker proxy
  sentinels on launch, and the guest credential audit recognizes Pi's
  `access`/`refresh` OAuth field names.
- Environment contract hashes now cover every regular file in each selected
  kit, including package manifests and lockfiles, and reject symbolic or
  non-regular kit content.

### Changed

- Bumped `sbxr` to 0.2.6. SSH-agent forwarding remains enabled and unchanged.

## [0.2.5] - 2026-11-02

### Fixed

- Cargo now fetches Git dependencies through the system Git CLI, allowing
  private repositories to authenticate through Docker's forwarded SSH agent.
  Sandbox bootstrap mirrors only same-scope `github.com` HTTPS-to-SSH rewrites
  explicitly configured on the host; arbitrary rewrite destinations remain
  excluded.

## [0.2.4] - 2026-11-02

### Fixed

- Made the default Codex+Claude workload a sandbox agent kit extending Docker's
  Codex agent. Docker only accepts OAuth declarations on the agent kit, so this
  allows the Anthropic host-proxy credential to compose successfully instead of
  failing creation when it was declared by the former Claude mixin.

## [0.2.3] - 2026-11-02

### Added

- Added equivalent `-y`, `--yes-to-all`, and `--force` flags for sandbox
  lifecycle commands. They forward `--force` to Docker's declarative
  environment command so provisioning can skip the plan-approval prompt;
  `sbxr up -y` provisions without opening VS Code.

## [0.2.2] - 2026-11-02

### Added

- Added `sbxr --version` and `sbxr -V`; both print the installed version
  without running host or KVM preflight checks.

### Security

- Claude subscription OAuth now uses Docker's host-side credential proxy in
  the multi-agent kit. Claude and Pi receive only provider-shaped sentinels;
  the legacy host credential import and Pi token-copy bridge are disabled.

### Changed

- Bumped the pinned sandbox Pi from 0.84.4 to 1.4.0 and updated its bundled
  Codex-proxy compatibility patch for Pi's new distribution layout.

### Fixed

- Restored compatibility with Docker Sandboxes 0.48.0 and newer: `sbxr` now
  accepts the top-level array emitted by `sbx ls --json` and uses the renamed
  `sbx env run --detach` and `sbx diagnose --format json` flags. Docker
  Sandboxes 0.39.0 through 0.47.x remain supported through their legacy output
  shape and flag names.

## [0.2.1] - 2026-09-12

### Added

- Added `sbxr paseo [PATH]`. Every development sandbox now installs the pinned
  Paseo CLI and a loopback-only daemon with its external relay disabled; the
  command starts that daemon when it is not running and prints the
  `ssh://USER@SANDBOX.sbx` URL a host Paseo client or the Paseo app connects to.
- The alias definitions in the host `~/.bashrc`, the whole of
  `~/.bash_aliases`, and the host `~/bin` utility directory are mirrored into
  the sandbox during bootstrap and wired into its interactive shells. Mirrored
  utilities are appended to `PATH` so sandbox-managed toolchains keep priority.
  `SBXR_SYNC_HOST_SHELL=0` disables it; `~/bin/.git` is never copied.

### Changed

- npm-based Pi extensions are now resolved inside the sandbox with
  `npm install` from the semver ranges the host `package.json` declares instead
  of `npm ci` against the mirrored host lockfile. Those exact host builds are
  selected against the host's own Pi release, so an extension could call an API
  the sandbox's kit-pinned Pi no longer exports and fail to load on every start.
- Bumped the pinned sandbox Pi to 0.84.4.
- A mirrored symbolic link is preserved as a link only while it resolves inside
  the same mirrored tree. One that points elsewhere is copied by content under
  the link's own name, since the sandbox has no copy of the path it names;
  previously it became a dangling link. Links the host cannot resolve are
  skipped.

## [0.2.0] - 2026-08-26

### Added

- Added `sbxr inspect [PATH]`, a read-only view of the project-specific sandbox
  name, protected environment path, desired/applied contract state, and
  materialized or prospective `.sbxenv.yaml` declaration.
- Successful `sbxr new`, `sbxr vscode`, `sbxr update`, and `sbxr up` commands
  now finish by printing the protected environment path and its corresponding
  `sbxr inspect` command.

### Changed

- Sandbox creation, reuse, and removal now use Docker's declarative `sbx env`
  interface. Docker can reconcile supported runtime fields on an existing
  environment, but changes to kits, workspaces, agents, mounts, and other
  creation-time properties require explicit sandbox recreation. `sbxr` detects
  those changes and never removes the existing sandbox automatically.
- Declarative environment definitions improve inspectability, reproducibility,
  and drift safety. `sbxr` stores the desired definition outside the writable
  project, hashes it together with every selected kit specification, and
  refuses to silently adopt legacy, externally recreated, or outdated
  same-name sandboxes.
