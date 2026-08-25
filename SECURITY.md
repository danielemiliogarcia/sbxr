# Security model

`sbxr` reduces the blast radius of Rust builds and coding agents; it does
not certify dependencies, extensions, or agent output as safe.

## Mitigations

- Builds, build scripts, procedural macros, tests, binaries, and agents run in
  a project-specific microVM rather than directly on the host.
- Projects receive separate persistent Cargo caches.
- Global `deny-all` plus kit-declared endpoints blocks and logs undeclared
  outbound hosts.
- Review mode mounts a clone rather than a writable host checkout, provides no
  agent OAuth, and does not mirror the host VS Code extension set.
- Toolchains and agents are pinned; available bootstrap artifacts are checksum
  verified.
- The launcher's only direct crate dependency is exact-pinned. Its complete
  locked dependency closure is vendored, and path installation can compile it
  with `cargo install --frozen --path .` and no registry access. Executed build
  scripts and package checksums are recorded in `THIRD_PARTY_REVIEW.md`. This
  addresses the bootstrap chicken-and-egg problem: installing the isolation
  launcher does not first fetch a newly resolved dependency graph.
- Official Codex keeps the real ChatGPT OAuth token outside the VM.
- Safe config mirroring excludes credential, history, session, log, project
  trust, and runtime-state files.

## Limits

- Cargo intentionally executes arbitrary build scripts and procedural macros.
- Malicious code already present in a crate needs no second network request.
- Allowed registries, GitHub, agent, npm, and VS Code hosts remain reachable;
  hostname allowlisting cannot validate every object served by them.
- Direct mode lets sandbox code modify the selected project and `.git`.
- Normal development windows disable VS Code Workspace Trust for that session
  because the microVM is the execution boundary; clone-mode `review` windows
  retain Workspace Trust.
- Claude credentials imported or created in the VM are readable by code
  running as `agent`. Pi's Claude bridge uses that same credential.
- Mirrored hooks, plugins, skills, prompts, and VS Code extensions execute code
  and expand the trusted computing base.
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
- `sbxr setup` intentionally invokes Docker browser OAuth and may download
  Microsoft's Remote-SSH extension into the host VS Code profile. It never
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
7. Keep SSH keys, Cargo publish tokens, cloud credentials, and production
   secrets out of project sandboxes.
8. Import Claude OAuth only into trusted sandboxes, and avoid concurrent
   Claude/Pi credential refresh.
9. Inspect blocked egress with `sbx policy log "$(sbxr name)"`.
10. Remove a suspected sandbox with `sbxr rm` rather than reusing its
    caches. Sandbox-local deletion is irreversible; host project files remain.

No policy makes unknown code risk-free. Combine isolation with least-privilege
accounts, disposable credentials, dependency review, reproducible builds,
artifact signing, and monitoring.
