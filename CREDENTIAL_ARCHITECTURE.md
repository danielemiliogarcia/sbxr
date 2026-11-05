# Credential architecture

`sbxr` keeps supported reusable credentials on the host while allowing selected
sandbox tools to authenticate through Docker Sandboxes. This document explains
that boundary, what it protects, and what remains possible for code running in
the sandbox.

For the broader threat model, including writable workspaces, network access,
agent extensions, and dependency execution, read [SECURITY.md](SECURITY.md).

## Core model

The normal provider flow is:

```text
real credential in Docker's host store
                  |
                  v
Docker Sandboxes credential proxy
                  |
                  v
non-secret sentinel visible inside the sandbox
                  |
                  v
request to an explicitly approved provider domain
                  |
                  v
proxy injects host authentication while forwarding the request
```

The generated `.sbxenv.yaml` declares services, approved domains, and optional
host credential commands. It does not contain OAuth tokens, private keys, or
literal GitHub/GHCR tokens. Docker executes declared credential commands on the
host and exposes only its proxy representation to the guest.

This protects the reusable credential value. It does not prevent sandbox code
from using the connected account through approved operations. A GitHub proxy
credential that can push to a repository can still be used by an agent to push;
an OAuth subscription can still be used to make model requests. Use
least-privilege accounts and review consequential operations.

## Integration matrix

| Integration | Reusable credential location | Guest-visible authentication | Important limits |
|---|---|---|---|
| Codex / ChatGPT | Docker host OpenAI OAuth store | Docker-managed OpenAI sentinel and official Codex configuration | Guest code can consume the subscription and available tools |
| Claude Code | Docker host Anthropic OAuth store | Docker-managed Anthropic access/refresh sentinels | The first native `/login` is completed through Claude so Docker can capture the OAuth exchange; do not copy a host Claude credential file |
| Pi with Codex | Same host OpenAI OAuth as Codex | Managed `openai-codex` sentinel entry | A narrow compatibility patch recognizes only Docker's exact sentinel; sbxr reasserts the managed entry on every launch |
| Pi with Claude | Same host Anthropic OAuth as Claude | Managed Anthropic access/refresh sentinel entry | May use Anthropic's separately billed third-party OAuth route; do not use Pi `/login` for this integration |
| GitHub HTTPS/API and `gh` | Host `gh` login, obtained by Docker through `/usr/bin/gh auth token` | Docker GitHub proxy sentinel | The agent can perform every GitHub action allowed by the host token |
| Private GHCR pulls | Host `gh` login, enabled only with the GHCR profile option | Docker registry proxy authentication | Does not make other registries safe and does not limit package access beyond the host credential's scopes |
| GitHub SSH | Private key and agent remain on the host | Forwarded SSH-agent socket | Guest processes can request authentication or SSH signatures while connected; the key material is not copied |
| SSH commit/tag signing | Host SSH agent | Forwarded signing request | Use an agent that confirms operations when signing authority is sensitive |
| OpenPGP/GPG or YubiKey signing | Host GPG agent and hardware token | Nothing is forwarded into the VM | Use `sbxr commit-pgp-sign`; commits created inside the sandbox remain unsigned until signed or re-signed on the host |
| VS Code Remote-SSH | Docker's host-side `*.sbx` SSH integration | Remote editor connection, not an account token copied by sbxr | Extensions installed in the remote environment execute inside the sandbox and may have their own login behavior |
| Paseo | Sandbox-local daemon state | SSH connection through the managed sandbox host entry; optional authenticated relay | Anyone able to open the SSH connection or authenticate to an enabled relay can drive the exposed agents; relay is off by default |
| Orca | Sandbox-local server state and private pairing payload | Docker-allocated ephemeral loopback pairing URL | Treat the pairing URL as a secret; sbxr diagnostics do not record it and reject non-loopback mappings |
| Browser automation | No host browser profile or cookies | Fresh sandbox-local Chromium contexts | Credentials typed into the guest browser become available to guest processes |

## Provider setup and renewal

### OpenAI

`sbxr setup` checks Docker's host store and, when needed, runs the browser-based
host OAuth flow equivalent to:

```bash
sbx secret set openai --oauth
```

Codex and Pi use that host-managed login. Do not repair an OpenAI problem by
running a guest Codex or Pi login or by copying an `auth.json` into the
sandbox. Renew it and run the post-renewal audits with:

```bash
sbxr auth renew openai .
```

### Anthropic

If Docker has no Anthropic OAuth entry, create the sandbox, run:

```bash
sbxr claude .
```

and complete Claude's native `/login` once. Docker captures the OAuth exchange
behind its credential proxy. The resulting real access and refresh tokens
remain in Docker's host store; the sandbox retains provider-shaped sentinels.

`sbxr` deliberately removed its legacy credential-import path. The command
`sbxr auth-import` now explains why copying a host Claude credential file into
the VM would break this boundary.

Later renewals use the same proxy-captured native flow:

```bash
sbxr auth renew anthropic .
```

### GitHub and GHCR

GitHub HTTPS/API proxying is enabled by the normal development profile. The
environment declaration contains only this fixed host command:

```text
/usr/bin/gh auth token
```

Docker executes it on the host and supplies proxy authentication to approved
GitHub domains. The command output is not placed in `.sbxenv.yaml` or copied to
the guest. Optional GHCR support uses the same host command as a registry
credential source.

SSH Git access is separate from the HTTPS proxy. `sbxr` retains Docker's
forwarded SSH agent for private dependencies, pushes, and SSH-format signing.
Renew the host `gh` login and re-audit the sandbox with:

```bash
sbxr auth renew github .
```

Each `auth renew` operation audits supported guest stores before and after the
interactive provider flow, then runs normal verification. It does not accept a
credential pasted into a project file or arbitrary guest store. OpenAI and
Anthropic renewal require a project preset that uses that provider; GitHub
renewal requires the host-proxied GitHub profile.

## Configuration mirroring

`sbxr` can mirror trusted host agent preferences, plugins, instructions, hooks,
prompts, themes, status-line helpers, Pi extensions, Bash aliases, selected
`~/bin` utilities, and compatible VS Code extensions. These are executable
inputs and expand the trusted computing base.

The mirror excludes supported authentication files, histories, sessions, logs,
project-trust records, and runtime state. Docker-managed provider configuration
and safety settings win when configuration is merged. Docker's shared agent
skills remain on read-only guest mounts. Credential-shaped fields are removed
from mirrored Claude settings. Before mirroring Pi `models.json`, sbxr removes
literal `apiKey` values and credential-like header values; only strict
environment-variable references such as `$NAME` or `${NAME}` survive.

Mirroring is not a generic secret filter for arbitrary files. Do not place
tokens in prompts, hooks, aliases, extensions, project files, or utilities and
expect sbxr to discover them.

## Workspace modes and credential exposure

| Mode | Host workspace | Agent OAuth | GitHub SSH | Intended use |
|---|---|---|---|---|
| Default development | Mounted read/write | Enabled for the selected preset | Forwarded | Comfortable daily development on trusted projects |
| `isolated` | Mountless sandbox-local clone | Enabled for the selected preset | Forwarded | Long-lived development without a writable host checkout; connected accounts remain usable |
| `review` | Mountless sandbox-local clone | Disabled | Disabled | Inspect unfamiliar code without agent OAuth, GitHub SSH, host extension/config mirroring, or Docker's shared host skills mount |

The credential proxy does not change workspace permissions. In default mode,
sandbox code can modify the mounted host project and its `.git` directory. Use
`sbxr review` for unfamiliar code and keep durable backups.

## SSH forwarding is a separate trust decision

SSH-agent forwarding is not credential proxying. The private key remains on
the host, but a sandbox process can ask the agent to authenticate or sign while
the forwarded connection is active. This preserves private Git dependency,
push, and SSH-signing workflows at the cost of exposing signing authority.

Use a dedicated least-privilege GitHub key and enable agent confirmation where
available. `sbxr review` omits the GitHub SSH kit. OpenPGP/GPG and YubiKey
signers are not forwarded; host-only `sbxr commit-pgp-sign` is the supported
workflow for them.

## Unsupported and guest-entered secrets

There is no generic way to make an arbitrary secret safe after it is supplied
to sandbox code. The following are not automatically protected by sbxr:

- project `.env` files and application secrets;
- cloud credentials, deployment keys, Cargo publish tokens, and blockchain
  signing keys;
- credentials entered into the sandbox browser;
- arbitrary MCP server credentials;
- logins performed by mirrored or manually installed extensions;
- secrets embedded in prompts, hooks, scripts, aliases, or project files.

Keep these out of the sandbox, use disposable test accounts, or add a dedicated
host-side broker whose interface and permissions can be reviewed. A read-only
mount or guest keyring still gives guest code access to the usable secret.

## Verification and diagnostics

Use the following checks without printing credential values:

```bash
sbxr doctor .
sbxr auth-status -y .
sbxr verify .
sbxr verify --live .
sbxr repair .
sbxr diagnose .
sbxr inspect .
```

- `doctor` checks the host Docker OAuth state and host prerequisites.
- `auth-status` reports provider configuration and makes a real,
  non-generation Anthropic request using the guest sentinel.
- `verify` validates runtime identity, known guest credential stores, exact
  managed tool versions, proxy routes, and SSH-agent availability. For Codex
  and Pi, only exact Docker-managed credential schemas are accepted; supported
  custom `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and `PI_CODING_AGENT_DIR` stores
  are included. Credential files must be regular, owned by the guest user, and
  private where required. Symlinks, malformed JSON, unexpected managed fields,
  and reusable values fail closed.
- Every normal reconciliation validates that the managed Codex, Claude, and Pi
  store paths are regular guest-owned files with no symlinked component, then
  enforces `0600` mode before verification. This corrects broader modes that an
  upstream workload may initially assign without relaxing the audit.
- `verify --live` additionally sends a minimal request through every selected
  model provider, suppresses raw output, and repeats the credential audit. It
  is opt-in because it consumes a small amount of subscription usage.
- `repair` reconciles idempotent integration state and performs normal (not
  live) verification. The host-side health ledger makes interrupted work and
  stages needing attention visible to later runs.
- `diagnose` summarizes only sanitized report fields and the project health
  ledger; it does not reveal raw errors, paths, arguments, or provider output.
- `inspect` prints the protected environment declaration and contract state so
  service domains and host commands can be reviewed.

The credential audit covers supported, known stores. It is not a
whole-filesystem scan and cannot prove that arbitrary project files, browser
memory, extension data, or logs contain no secrets. Diagnostic reports exclude
arguments, paths, environment values, raw errors, command output, credential
values, and private pairing links.

Within those supported stores the audit is fail-closed: managed Codex, Claude,
and Pi schemas must match their proxy sentinels; credential-shaped singular and
qualified plural fields are rejected; Docker auth must decode as
`username:sentinel`; and guest Docker credential-helper configuration is
rejected because it could point to another reusable credential store.

## Implementation references

- `src/envfile.rs`: service bindings, approved domains, host GitHub/GHCR
  commands, and credential-free environment rendering;
- `src/auth.rs`: provider status and the real Anthropic proxy probe;
- `src/health.rs`: protected resumable integration-health state;
- `src/node_tools.rs`: fail-closed Pi sentinel compatibility and launch-time
  reassertion;
- `src/sync.rs`: trusted configuration selection and credential exclusions;
- `src/verify.rs`: guest credential audit and runtime verification;
- `src/git.rs` and `src/sign.rs`: forwarded SSH behavior and host-only OpenPGP
  signing.

