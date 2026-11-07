# sbxr and sbx-suite comparison

## Feature and architecture comparison

| Area | sbxr 0.3.4 | sbx-suite | Practical difference |
|---|---|---|---|
| Primary goal | Fast, personalized, project-scoped Rust development | Mountless, controlled AI-agent environment; also specialized for Acrux | sbxr prioritizes daily developer comfort; sbx-suite prioritizes separation |
| Host launcher | Native Rust executable | Python 3.10+ scripts using only the standard library | sbxr needs no host Rust when installed from a release; sbx-suite needs Python |
| Installation | Attested native binaries and installers; source build also available | Clone the repository and run `python3 setup.py` | sbxr feels more like a packaged host application |
| Default workflow | `cd PROJECT && sbxr new -y` | Run a comprehensive setup wizard, then clone projects inside the VM | sbxr is substantially faster for repeat use |
| Wizard | Optional: `sbxr wizard` | Central/default workflow | sbx-suite exposes more choices during initial setup |
| Sandbox scope | One deterministic sandbox per project | One sandbox per setup directory, potentially containing multiple projects | sbxr reduces lateral access between projects |
| Default workspace | Host project mounted read/write | Project exists only on the sandbox's persistent guest disk | sbx-suite gives stronger host-filesystem isolation |
| Alternative workspace modes | `isolated` and hardened `review` clone modes | Mountless is always the normal model | sbxr lets the user choose comfort or isolation |
| Durability | Shared mode leaves source and Git history directly on the host | Work must be pushed or copied out before sandbox deletion | sbxr is safer against accidental sandbox deletion |
| Image architecture | Digest-pinned official workloads plus composable Kits v3 and a local content-addressed cache | Custom full image derived from pinned `shell-docker`; build locally, publish to GHCR, or reuse by digest | sbxr is modular and personalized after creation; sbx-suite is more image-centric |
| Host personalization | Bash aliases, `~/bin`, prompt, Zsh/Fish, agent configuration, plugins and compatible VS Code extensions | Deliberately avoids mounting or mirroring host configuration | sbxr feels like the developer's machine; sbx-suite has a smaller trusted-input surface |
| Rust | Rust 1.99.1 plus automatic nested `rust-toolchain` discovery, installation, components and verification | Shared pinned Rust 1.99.1 toolchain | sbxr is better for repositories containing multiple toolchains |
| Agents and orchestration | Codex, Claude, Pi, Paseo, and optional Orca Server/Desktop integration | Codex, Claude, Pi, and Paseo | Both cover the core agents; Orca integration is specific to sbxr |
| OpenAI credentials | Docker host OAuth with exact guest sentinels | Docker host OAuth with exact guest sentinels | Equivalent credential-value protection |
| Claude credentials | Docker-captured native OAuth, retained on the host | Host-generated subscription token stored by sbx and injected through the proxy | Both keep reusable Claude credentials outside the guest |
| GitHub and GHCR | HTTPS/API and optional GHCR through the host proxy | HTTPS/API and optional GHCR through the host proxy | Broadly equivalent |
| SSH agent | Forwarded by default for private Git, pushes and SSH signing | Globally disabled and actively checked | sbx-suite prevents use of host SSH signing authority; sbxr preserves compatibility |
| OpenPGP/YubiKey | Host-side `sbxr commit-pgp-sign`, including safe multi-commit rewriting | No comparable integrated signing workflow | sbxr fits OpenPGP YubiKey workflows considerably better |
| Credential audit | Fail-closed exact-schema checks, permissions, ownership, symlinks, custom homes and live re-audit | Fail-closed supported-store audit and setup checks | Both are strong; neither is a whole-filesystem secret scanner |
| Unknown-code workflow | Dedicated `sbxr review`: mountless, no OAuth, no SSH, no host configuration or shared skills | Create a separate mountless sandbox and avoid selecting integrations | sbxr provides the cleaner one-command review boundary |
| Shared agent skills | Read-only in normal development; disabled in review mode | Disabled for all suite sandboxes | sbx-suite has the stricter default |
| Network policy | Per-sandbox `development`, `restricted`, and `offline`; tracks and removes only sbxr-owned rule IDs | Keep, balanced or deny-all; changing presets can reset host-global policy | sbxr has safer per-project policy ownership |
| Nested Docker | Root-capable Docker inside the microVM for compatibility | Normal workload also uses privileged Docker inside the microVM | Equivalent host boundary: the microVM, not the inner container |
| Rootless Docker | Not provided | Restricted Acrux agent gets rootless Docker; ordinary workload remains privileged | sbx-suite is stronger specifically for its Acrux split-user workload |
| npm hardening | Managed Node tools use committed lockfiles and `npm ci --ignore-scripts` | Same for managed tooling, and globally defaults project npm to `ignore-scripts=true` | sbx-suite is stricter; sbxr is less likely to break normal Node projects |
| Browser automation | Optional locked Playwright MCP with ephemeral contexts | Optional locked Playwright MCP with ephemeral contexts | Broadly equivalent |
| Orca | `sbxr orca` provides a checksum-pinned, headless, loopback-only server for Orca Desktop | Not present | sbxr-only integration |
| Paseo access | SSH URL; optional authenticated relay | Published host-loopback HTTP port; optional authenticated relay | sbxr avoids a fixed host port; both keep the relay off by default |
| VS Code | Automatically opens the project, matches extensions, verifies server state and preserves the remote identity | Configures Remote-SSH; the user connects and opens `/home/agent/workspace` | sbxr is more automated and host-like |
| Health and repair | Protected health ledger plus `status`, `verify`, `repair`, `diagnose`, and optional live model probes | Resumable wizard checkpoints, Needs Attention states and structured reports | sbxr has the richer daily operational interface |
| Diagnostics | Sanitized reports with runtime metadata and failure categories | Sanitized structured setup report with stages and failure categories | Both are strong |
| Supply-chain model | Exact Rust dependency, complete vendoring, offline builds, pinned actions and attested releases | Python standard-library launcher, pinned image digests, locked Node tree and checksum-pinned downloads | Both are disciplined; sbxr has stronger distributed-binary provenance |
| Specialized workloads | General Rust development | Dedicated Acrux deployment/development with separate operator and restricted agent identities | sbx-suite clearly wins for Acrux |
| License | Apache-2.0 | MIT | Both are open source; MIT imposes fewer conditions, while Apache-2.0 adds an explicit patent grant |

## Security comparison

There is no universal security winner. The defaults defend different risks.

| Security dimension | Stronger default | Why |
|---|---|---|
| Prevent sandbox code from modifying host source | sbx-suite | No host workspace mount |
| Prevent access to host SSH/signing authority | sbx-suite | SSH-agent forwarding is disabled |
| Protect reusable OAuth/token values | Approximately equal | Both use host proxying and non-secret sentinels |
| Inspect completely unfamiliar code | sbxr `review` | Mountless and also removes OAuth, SSH, host configuration and shared skills |
| Isolate projects from each other | sbxr | One sandbox per project by design |
| Avoid trusting personal configuration | sbx-suite | It does not mirror shell or agent customization |
| Preserve work if a VM is deleted | sbxr shared mode | The authoritative repository remains on the host |
| Restrict policy changes to one sandbox | sbxr | Exact launcher-owned rule IDs and no global reset |
| npm lifecycle-script resistance | sbx-suite | Globally disables automatic npm scripts by default |
| Release artifact provenance | sbxr | Native artifacts and installers have checksums and GitHub attestations |

## Supply-chain clarification

Current sbx-suite should not be characterized as Python with dynamically
installed Python dependencies. Its launcher uses the Python standard library
without pip packages, and its managed Node toolchain is locked. The remaining
distinction is that it depends on the host Python runtime and builds a larger
custom image, whereas sbxr distributes an attested native launcher and composes
official workloads with Kits.

## Recommended fit

sbxr is the better fit when the developer wants:

- source visible and durable on the host;
- private Git over SSH;
- OpenPGP/YubiKey signing;
- familiar shell aliases, utilities and VS Code configuration;
- the fast `sbxr new -y` workflow;
- automatic nested Rust toolchains;
- Orca Desktop integration; and
- a dedicated hardened review mode when convenience is inappropriate.

sbx-suite is the better fit when the primary requirement is that agents must
not see or modify any host project path or use host SSH authority, or when the
specialized Acrux workload is required.

## References

- [sbxr README](README.md)
- [sbxr credential architecture](CREDENTIAL_ARCHITECTURE.md)
- [sbxr security model](SECURITY.md)
- [sbx-suite README](/home/emi/tmp/sbx-suite/README.md)
- [sbx-suite credential isolation](/home/emi/tmp/sbx-suite/docs/credential-isolation.md)
- [sbx-suite usage and isolation](/home/emi/tmp/sbx-suite/docs/usage.md)
