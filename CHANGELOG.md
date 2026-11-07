# Changelog

## [0.3.5] - 2026-11-07

### Changed

- The POSIX and PowerShell binary installers now discover the newest eligible
  release when no version is supplied, while retaining explicit version pins
  for reproducible installs. The release matrix exercises both paths on every
  native target before publication, then a second four-platform matrix
  downloads, attests, and runs the real public installers after publication.
  The README now presents short binary and reviewed source quick starts
  followed by one common setup flow.
- The main README is now a concise landing page containing the quick start,
  included environment, local kit-cache model, and limitations. Detailed
  commands, architecture, integrations, setup behavior, customization, and
  security installation guidance moved to the linked `README-EXTENDED.md`,
  which is also included in binary release archives.

## [0.3.4] - 2026-11-06

### Added

- Added a tag-gated native release matrix for Linux x86-64/ARM64,
  Apple-silicon macOS, and Windows x86-64. Release builds use the exact pinned
  current-stable Rust compiler with the committed vendored dependency tree,
  smoke-test each native binary and its installer, publish SHA-256 checksums,
  and attach GitHub build-provenance attestations. Verified POSIX and
  PowerShell installers plus documented manual archive installation let users
  run sbxr without installing Rust; source builds remain available and CI
  separately enforces the Rust 1.85 MSRV.
- Added first-class Bash, Zsh, and Fish login-shell profiles. The existing
  familiar Bash experience remains the default; protected profiles, direct
  flags, and the wizard can select `familiar`, `minimal`, or `custom`
  mirroring plus Bash, Zsh, or Fish. Explicit alias files and executable
  directories are custom-profile inputs, while prompt choices can be classic,
  plain, or trusted custom files.
- Added `sbxr setup --prebuild` to warm Docker Sandboxes' native,
  content-addressed Kits v3 composition cache from sbxr's pinned default
  composition without mounting a workspace or applying personal host files.
  Ordinary sandbox creation continues to build and reuse the same cache
  automatically, so prebuilding is optional.
- Added a protected, resumable integration-health ledger. `sbxr status` shows
  pending, verified, skipped, and needs-attention stages; `sbxr repair`
  idempotently reconciles integrations and verifies the result.
- Added `sbxr diagnose` to summarize the newest privacy-preserving lifecycle
  report together with project health, and enriched reports with sanitized
  runtime client/server/API metadata and failure categories.
- Added `sbxr auth renew openai|anthropic|github` using each integration's
  supported host-proxy workflow, with credential audits before and after the
  renewal and a verification pass before reuse.
- Added opt-in `sbxr verify --live` model requests for Codex, Claude, and Pi.
  Raw provider output is suppressed and the guest credential audit is repeated
  after the requests.
- Added an optional authenticated Paseo relay profile setting and wizard
  choice. The default remains relay-off; enabling it atomically updates guest
  configuration and adds only the sandbox-scoped relay endpoint.

### Changed

- Pinned the default sandbox toolchain to Rust 1.99.1 while retaining automatic
  discovery, installation, component reconciliation, and verification for
  nested `rust-toolchain.toml` and `rust-toolchain` manifests.
- Kept the official-workload-plus-local-Kits architecture: personal shell and
  agent customization remains a post-creation overlay and is never captured in
  the local composition cache or published as a personalized registry image.
- Docker compatibility is now an explicit client/server boundary: sbxr parses
  the required `sbx version --json` fields and probes every CLI capability it
  relies on before lifecycle work. Committed 0.51 fixtures guard the schemas.
- Orca now uses a Docker-allocated ephemeral host port, discovers the actual
  mapping from structured output, and accepts it only when bound to loopback.
- `sbxr review` explicitly disables Docker's shared host skills mount in
  addition to its existing clone-mode, no-OAuth, no-SSH, no-host-mirror rules.
- The normal `sbxr new -y` path, comfortable defaults, shared workspace, and
  SSH-agent forwarding remain unchanged.

### Upgrade notes

- Existing development sandboxes require recreation for the Rust 1.99.1 and
  host-shell kit revisions. Selecting Zsh or Fish also changes the kit
  composition and therefore requires recreation; changing alias files,
  executable directories, prompt style, or shell mirroring profile is
  reconciled in place by the next ensuring command or `sbxr update`.
- Existing Orca-enabled sandboxes must be recreated because their environment
  contract now requests a Docker-allocated loopback port instead of a fixed
  host port. Existing review sandboxes must also be recreated because the
  contract now explicitly disables shared host skills. Docker Sandboxes 0.51
  reallocates the ephemeral port across a normal stop/start too, so rerun
  `sbxr orca` after a restart or recreation and update the Orca Desktop pairing
  entry.
- Existing development sandboxes also require a one-time recreation because
  the profile fingerprint now correctly excludes runtime network policy. After
  that migration, changing `development`, `restricted`, or `offline` no longer
  recreates the VM.

### Security

- Custom shell files must be explicitly selected below the trusted host home.
  Their paths are validated against traversal and overlap with known
  credential stores, including SSH/GPG/cloud/agent homes, `.npmrc`,
  `.pypirc`, Cargo registry credentials, `.vault-token`, password managers,
  browser profiles across supported host platforms, login/environment files,
  and Fish universal variables. The check is repeated on each selection's
  resolved target, which must remain inside the canonical host home, and is
  case-insensitive, so symlinks cannot bypass it; escaping nested symlinks are
  skipped rather than followed.
  Profiles remain nonsecret, and neither shell personalization nor credentials
  are inputs to the local composition cache. Mirrored shell files are executable
  trusted-host inputs.
- Strengthened the guest credential audit to require exact Codex and Pi
  sentinel schemas, inspect supported custom credential homes, fail closed on
  malformed JSON, and reject credential stores with unsafe types, owners,
  permissions, or symlinks. Findings contain locations only, never values.
- Health records and their details use protected atomic host state and a
  constrained redacted schema. Live provider verification is never implicit.
- Added a consolidated credential-architecture guide covering Docker's host
  proxy, guest sentinels, GitHub/GHCR host commands, SSH-agent forwarding,
  host-only OpenPGP signing, workspace modes, unsupported secrets, renewal,
  auditing, and verification commands.
- Network-profile rule ownership now lives in protected host metadata as exact
  Docker rule IDs. Guest code cannot forge an ownership marker, and sbxr never
  removes a manual or host-global policy rule while changing profiles.
- The credential audit now rejects malformed Docker auth entries, guest Docker
  credential helpers, and qualified plural fields such as `authTokens` and
  `clientSecrets`. Protected profile reads reject unsafe files and sandbox-name
  overrides cannot traverse outside authoritative state.

### Fixed

- Restored the declared Rust 1.85 source-build compatibility by replacing five
  nonessential newer let-chain expressions. CI now compiles all targets at the
  declared MSRV so future compatibility drift fails before release.
- Mirrored host shell files now live under one launcher-owned guest root,
  `~/.local/share/sbxr/host-shell`, instead of at their host-relative paths in
  `/home/agent`. Selecting a path such as `.cargo/bin` or `.local/bin` can no
  longer overwrite or delete sandbox-managed toolchains or agent binaries, and
  switching to `minimal` removes every previously mirrored alias and
  executable directory. Each refresh is serialized per sandbox, uses a unique
  staging directory, and restores the prior mirror if installation fails.
  Host paths inside mirrored files and
  extracted aliases (`/host/home/bin/x`, `~/bin/x`, `$HOME/bin/x`) are
  rewritten to the mirror, including quoted `"$HOME"/bin/x`. Host source
  fingerprints (now including permission bits and the generated overlay) make
  ordinary host edits, `chmod +x`, and launcher overlay changes refresh on the
  next ensuring command.
- Familiar shell profiles no longer fail when `~/.bash_aliases`, `~/bin`, or
  their Zsh/Fish equivalents are symlinks managed by stow, chezmoi, or a
  dotfiles repository; in-home selections are mirrored by content. A
  conventional path resolving outside the home (for example into the Nix
  store or a separate data volume) is skipped with a warning instead of
  blocking every launch, while an explicit external selection remains a hard
  error. A dangling link inside a mirrored directory is skipped instead of
  blocking every launch.
- Concurrent shell reconciliation now treats a freshly created empty lock as
  initialization in progress. It retries during a bounded grace period and
  later reclaims an abandoned empty lock, while malformed nonempty locks remain
  fail-closed.
- In-tree symlinks in mirrored shell directories are emitted as relative links,
  so deep targets no longer exceed ustar's 100-byte link field and links stay
  valid when the host home is itself reached through a symlink.
- Home-relative shell paths are normalized (`bin/`, `./bin/.` become `bin`).
- The Zsh kit seeds `/etc/shells` before installing, matching the Fish kit, so
  `add-shell` cannot fail on the minimal agent workload.
- Verification now starts the selected Bash, Zsh, or Fish login shell and
  confirms that every tool required by the chosen preset resolves through its
  real interactive `PATH`. A dedicated Bash kit now explicitly selects
  `/bin/bash` instead of inheriting `/bin/sh` from the Codex base image, while
  the generic host-overlay kit no longer overrides selected Zsh or Fish login
  shells. Zsh enables completion, and Fish familiar-mode extraction ignores
  nested functions instead of treating their aliases as top-level statements.
- Live Claude verification now accepts its JSON result as one record in a
  line-oriented output stream, matching the Codex and Pi checks. Benign
  launcher notices or harmless variation in model wording can no longer make
  a successful nonempty proxied response fail verification; error and empty
  results remain rejected.
- `rust-analyzer` now always launches the pinned 1.99.1 server instead of
  allowing rustup's shim to select an older nested project toolchain that may
  not include the component. Cargo and rustc still follow each project's
  `rust-toolchain` manifest, and verification checks the analyzer from every
  discovered toolchain directory.
- `sbxr setup --prebuild` removes abandoned launcher-owned warm-up sandboxes
  before starting another build. Ownership is checked with `ps` (Linux and
  macOS) and requires the recorded PID to still be an sbxr process. The documented warm-up scope is the default
  Bash composition; opt-in shell layers are built on their first use. The
  documentation also makes explicit that this reuses kit assembly, while each
  real sandbox still initializes fresh persistent volumes and runs idempotent
  install hooks; sbxr does not use incomplete volume-omitting snapshots.
- Reconciliation now validates the known proxy-managed credential paths and
  enforces guest-private `0600` mode on Codex, Claude, and Pi sentinel files.
  This fixes the broader mode initially assigned to Codex and Claude stores by
  the upstream workload while keeping the audit fail-closed.
- Orca repair now detects and installs the NSPR runtime required by its
  Electron AppImage on hardened Kits v3 images, while reusing the existing
  checksum-verified extraction.
- Orca startup now safely recovers the stale daemon lock retained when its
  sandbox-local data volume survives a microVM recreation. Recovery is limited
  to Orca's explicit changed-machine-identity case and refuses a lock matching
  a live local process.
- Claude settings mirroring now removes credential-shaped fields. Pi
  `models.json` mirroring removes literal API keys and credential-like headers
  while retaining explicit environment-variable references. The guest audit no
  longer mistakes numeric `maxTokens` model metadata for a token.
- Rust verification now checks the pinned sbxr toolchain explicitly through
  `rustup run`, so a legitimate project-root override is not reported as base
  toolchain drift. It also validates every discovered project toolchain, and
  reconciliation repairs a removed toolchain even when its cached manifest
  marker still matches.
- Remote VS Code extensions are installed in bounded batches, preventing a
  large host inventory from accumulating installer memory in one process and
  exhausting the sandbox VM.
- `sbxr rm` now closes the matching Remote-SSH window before removing the VM,
  preventing a recreated sandbox with the same name from inheriting a stale
  canceled connection. Window-close polling is bounded and avoids a VS Code
  status race after the last desktop window exits.
- Network profiles are now genuinely runtime-reconfigurable without sandbox
  recreation. Transitions add the new deny policy before removing prior
  sbxr-owned rules, persist interrupted transitions safely, and support
  Docker's one-rule-per-resource expansion.
- Runtime verification now rejects unexpected extra kits, accounts for the
  optional RocksDB kit, and verifies the selected network profile. Orca accepts
  only TCP loopback port mappings and pairing payloads whose endpoint authority
  is the expected guest loopback listener.

### Testing

- Added failure-path tests for runtime schema drift, missing command
  capabilities, executable credential-audit fixtures, review isolation,
  protected health-ledger transitions, relay-profile validation, non-loopback
  Orca mappings, and the guarded Orca stale-lock script contract. Added
  multi-toolchain verification and bounded extension-batch regression
  coverage, plus a bounded-process regression for the VS Code status probe.
- Added behavioral regressions for Docker auth parsing and credential helpers,
  plural credential fields, exact kit inventories, sandbox-name traversal,
  runtime network contracts, split network rules, TCP-only published ports,
  and Orca endpoint validation.

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
