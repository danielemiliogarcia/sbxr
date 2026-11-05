# Security model

`sbxr` reduces the blast radius of Rust builds and coding agents; it does
not certify dependencies, extensions, or agent output as safe.

The host proxy, guest sentinels, SSH-agent forwarding, OpenPGP signing, and
unsupported-secret boundaries are described in
[CREDENTIAL_ARCHITECTURE.md](CREDENTIAL_ARCHITECTURE.md).

## Mitigations

- Builds, build scripts, procedural macros, tests, binaries, and agents run in
  a project-specific microVM rather than directly on the host.
- Projects receive separate persistent Cargo caches.
- Kit-declared endpoints describe required provisioning/runtime destinations.
  Optional `restricted` and `offline` profiles add sandbox-scoped deny rules
  after bootstrap without modifying host-global policy. Exact sbxr-owned rule
  IDs are tracked in protected host state, not a guest-controlled marker, and
  profile transitions remove only those IDs.
- Review mode mounts a clone rather than a writable host checkout, provides no
  agent OAuth or GitHub SSH, does not mirror host agent/VS Code configuration,
  and explicitly disables Docker's shared host skills mount.
- Authoritative `.sbxenv.yaml` files live in protected host state outside every
  mounted workspace. On Unix their directories/files use `0700`/`0600`, writes
  are atomic, and symlinked authoritative path components/files are rejected.
- Desired/applied contract markers prevent Docker from silently reusing a
  legacy or externally recreated same-name sandbox with stale creation-time
  workspaces, kits, ports, or options. The hash includes the generated
  environment and every regular file in each selected embedded kit, including
  Node package manifests and lockfiles; links and non-regular kit files are
  rejected.
- Protected profiles and explicit `SBXR_NAME` overrides reject unsafe file
  types, permissions, symlinks, and path-like sandbox names before they can
  influence authoritative state paths.
- Toolchains and agents are pinned; available bootstrap artifacts are checksum
  verified. Codex, Claude, and Shell use official Kits v3 pinned by OCI digest.
  Node.js uses architecture-specific 22.23.4 archives with committed SHA-256
  digests. Pi and Paseo use committed npm lockfiles and install with lifecycle
  scripts disabled.
- The launcher's only direct crate dependency is exact-pinned. Its complete
  locked dependency closure is vendored, and path installation can compile it
  with `cargo install --frozen --path .` and no registry access. Executed build
  scripts and package checksums are recorded in `THIRD_PARTY_REVIEW.md`. This
  addresses the bootstrap chicken-and-egg problem: installing the isolation
  launcher does not first fetch a newly resolved dependency graph.
- Docker's credential proxy keeps the real ChatGPT, Claude, and GitHub tokens
  outside the VM. Codex, Claude, Pi, Git/gh, and optional GHCR access receive
  only host-scoped sentinels or proxy-injected requests.
  Each sbxr launch reasserts Pi's managed provider entries, so guest login
  state cannot persistently replace those sentinels with reusable OAuth values.
  Reconciliation also validates the known managed store paths and normalizes
  the Codex, Claude, and Pi sentinel files to guest-private `0600` mode.
- The guest audit accepts only exact managed Codex/Pi schemas, includes
  supported custom credential homes, and fails closed on malformed JSON,
  symlinks, unsafe ownership/types, and overly broad permissions. It reports
  store locations but never credential values.
- GitHub authentication and enabled or explicitly requested SSH signing use
  Docker's forwarded host SSH agent; private SSH keys are not copied into the
  VM. Automatic commit/tag signing follows the host project's effective Git
  defaults only when its signing format is SSH.
- Safe config mirroring excludes credential, history, session, log, project
  trust, and runtime-state files. Claude settings additionally drop
  credential-shaped fields; Pi model configuration drops literal API-key and
  credential-header fields while retaining strict environment-variable
  references.
- Validated project profiles are nonsecret, reject unknown and secret-looking
  fields, and use protected atomic host files. Structured diagnostic reports
  exclude arguments, paths, environment, raw errors, output, and secret links.
- Per-integration health uses the same protected host-state discipline and
  records only constrained status/detail fields. A new sbxr version invalidates
  earlier verified stages until they are checked again.
- The optional browser MCP is lockfile-pinned, installs with npm lifecycle
  scripts disabled, uses an ephemeral profile and sandbox proxy CA, and shares
  no host browser state. Orca is checksum-pinned, runs headlessly with
  guest-installed Xvfb/`xauth` and runtime libraries, and binds through host
  loopback only.
- sbxr validates both Docker client and server versions, parses required
  structured schemas, and probes the exact CLI flags it depends on before
  changing sandbox state. Orca host ports are allocated dynamically and a
  non-loopback mapping is rejected.
- Orca stale-lock recovery is scoped to its sandbox-local persistent volume.
  sbxr accepts only Orca's changed-machine-identity refusal, validates the
  lock's owner, mode, type, PID, and Linux start time, and refuses to remove a
  lock matching a live process.

## Limits

- Cargo intentionally executes arbitrary build scripts and procedural macros.
- Malicious code already present in a crate needs no second network request.
- Allowed registries, GitHub, agent, npm, and VS Code hosts remain reachable;
  hostname allowlisting cannot validate every object served by them.
- Direct mode lets sandbox code modify the selected project and `.git`.
- Clone-mode `isolated` removes that writable host workspace mount, but its
  sandbox-local clone is not a replacement for reviewed backups or pushes.
- Sandbox-local caches, agent history/login state, tool installations, and
  remote-editor state are disposable. `sbxr` never silently recreates a
  mismatched sandbox, but explicit `rm`/recreation permanently discards them;
  the host project remains the durable source of truth.
- While an SSH/Remote-SSH connection is active, any sandbox process can ask the
  forwarded host SSH agent to authenticate or sign. Agent confirmation and
  least-privilege Git keys remain important.
- GitHub HTTPS/API and GHCR proxying prevents token extraction, but it does not
  prevent an agent from using the host credential for allowed GitHub actions.
  Keep scopes minimal and review pushes, releases, and package operations.
- Docker does not forward OpenPGP/GPG, YubiKey GPG, or S/MIME signers. When the
  host requires one, `sbxr` warns and disables automatic sandbox signing;
  sandbox commits are unsigned and must be signed or re-signed on the host.
  `sbxr commit-pgp-sign` performs that amend through host Git/GPG with project
  hooks disabled, refuses staged changes and in-progress Git operations, and
  confirms that the new commit contains an OpenPGP signature.
- Multi-commit OpenPGP signing necessarily rewrites history. sbxr restricts it
  to a clean, linear, same-author suffix, creates a recovery ref, disables
  hooks, verifies the unchanged final tree and every new signature, and aborts
  automatically on signing failure. It does not push rewritten history;
  publishing it remains an explicit `git push --force-with-lease` decision.
- Normal development windows disable VS Code Workspace Trust for that session
  because the microVM is the execution boundary; clone-mode `review` windows
  retain Workspace Trust.
- The sandbox can read OAuth sentinel strings, but Docker replaces them only
  for the approved provider hosts. `sbxr` no longer copies host Claude
  credential files into the VM or bridges real tokens between Claude and Pi.
- Mirrored hooks, plugins, skills, prompts, and VS Code extensions execute code
  and expand the trusted computing base.
- `development` networking is intentionally comfortable rather than offline.
  A malicious dependency can use allowed destinations, and code already
  present needs no network. Select `restricted` or `offline` only after the
  needed toolchain/integrations are provisioned; later downloads then require
  a profile change and explicit recreation.
- The recognized `rtk hook claude` executable is copied from the trusted host
  and version-checked. Desktop-audio hooks are omitted quietly because the VM
  has no host audio session. Other simple Claude hooks whose leading executable
  is unavailable are omitted with a warning; `sbxr` never installs arbitrary
  host-hook dependencies automatically.
- Pi npm extension packages are resolved inside the sandbox from the semver
  ranges the host `package.json` declares, with lifecycle scripts disabled. The
  host lockfile is not reused, because its exact builds are selected against the
  host's own Pi release and an extension pinned that way can fail to load
  against the sandbox's kit-pinned Pi. Resolution therefore follows the host's
  declared ranges rather than a host-verified version set. Their runtime code,
  raw Pi extensions, and mirrored Codex/Claude plugins still execute in the
  sandbox and must be treated as trusted code.
- Host Bash aliases and every executable in the host `~/bin` are mirrored into
  the sandbox and placed on `PATH`. They are host-authored code that runs there;
  mirror them only from a trusted host profile, or set `SBXR_SYNC_HOST_SHELL=0`.
- The sandbox Paseo daemon binds to the sandbox loopback interface with its
  external relay disabled, so reaching it requires an SSH session into that
  sandbox. Anyone who can open that session can drive the agents it controls.
- Enabling the optional Paseo relay expands reachability to an authenticated
  third-party relay and permits its endpoint for that sandbox. It remains off
  by default, removes the scoped endpoint rule when disabled, cannot be
  combined with `offline`, and does not make agent actions less privileged
  inside the sandbox.
- `--rocksdb-host` trusts the selected host-built shared library and its ABI.
  The prefix is read-only, but a compromised or incompatible library still
  executes inside the sandbox process and can affect the mounted project.
- `cargo audit` detects known RustSec advisories, not arbitrary malware.
- `cargo deny` enforces dependency policy; it is not behavioral detection.
- `cargo vet` helps only when the team maintains meaningful audits and trust
  criteria.
- `--locked` prevents resolution drift but still builds a malicious version
  already selected in `Cargo.lock`.
- Vendoring prevents source drift and network fetching, but reviewed vendored
  code still executes during compilation. Checksums establish identity, not
  benign behavior.
- `sbxr setup` intentionally invokes Docker browser OAuth and `sbx setup ssh`,
  and may download Microsoft's Remote-SSH extension into the host VS Code profile. It never
  invokes `sudo` or installs operating-system packages; run it only from a
  reviewed launcher binary.
- Pi's documented third-party Claude OAuth route may consume separately billed
  Anthropic extra usage.

## Recommended practice

1. Use global `deny-all` plus kit-declared endpoints where practical.
2. Commit application `Cargo.lock` and review every dependency change.
3. Install this launcher from a reviewed checkout with
   `cargo install --frozen --path .`; retain `--locked` for intentional
   registry-backed source installations.
4. Run `sbxr audit` before accepting dependency changes.
5. Maintain cargo-vet policy for critical repositories and consider registry
   allowlists, dependency cooling periods, and signed internal artifacts.
6. Use `sbxr review` before building an unknown repository.
7. Keep private SSH-key files, Cargo publish tokens, cloud credentials, and
   production secrets out of project sandboxes; prefer Docker's host-side
   credential and SSH-agent forwarding mechanisms.
8. Authenticate Claude through its native `/login` flow so Docker can retain
   the real OAuth tokens behind the host proxy; never paste or copy tokens into
   the sandbox.
9. Inspect blocked egress with `sbx policy log "$(sbxr name)"`.
10. Run `sbxr verify` to check managed credential stores and runtime identity
    without printing credential values. Use `sbxr verify --live` deliberately
    when a real minimal provider request is needed, and `sbxr repair` for a
    health stage marked pending or needing attention.
11. Remove a suspected sandbox with `sbxr rm` rather than reusing its
    caches. Sandbox-local deletion is irreversible; host project files remain.

No policy makes unknown code risk-free. Combine isolation with least-privilege
accounts, disposable credentials, dependency review, reproducible builds,
artifact signing, and monitoring.
