# Environment variables

`sbxr` requires no custom environment variables for normal use. Every
`SBXR_*` variable is an optional override. Command-line options take precedence
over equivalent environment variables.

## Launcher variables

| Variable | Accepted value | Default and behavior |
|---|---|---|
| `SBXR_PRESET` | `rust-multi-agent`, `rust-codex`, or `rust-claude` | `rust-multi-agent`; overridden by `--preset` |
| `SBXR_NAME` | Sandbox name | Deterministic name derived from preset, project path, and optional RocksDB prefix |
| `SBXR_ROOT` | Directory path | Root for materialized embedded kits; see platform defaults below |
| `SBXR_KIT_ROOT` | Directory path | Uses external kit directories instead of materializing the embedded kits |
| `SBXR_STATE_ROOT` | Directory path | Protected host root for generated `.sbxenv.yaml` and contract metadata; see platform defaults below |
| `SBXR_ROCKSDB_HOST` | RocksDB installation prefix | Unset and disabled; overridden by `--rocksdb-host[=PREFIX]` |
| `SBXR_SYNC_AGENT_CONFIG` | `0` to disable | Trusted-host Codex, Claude, and Pi extension/plugin and configuration mirroring is enabled |
| `SBXR_SYNC_VSCODE_EXTENSIONS` | `0` to disable | Host VS Code extension mirroring is enabled |
| `SBXR_SYNC_PI_EXTENSIONS` | `0` or `1` | Unset/`1` mirrors raw Pi source extensions; `0` omits them |
| `SBXR_SYNC_HOST_SHELL` | `0` to disable | The profile-selected trusted host shell overlay is mirrored |
| `SBXR_PRESERVE_XDG_STATE_HOME` | `1` to preserve | `XDG_STATE_HOME` is removed from child `sbx` and VS Code commands |

Values other than the documented control value generally behave like the
default. `SBXR_SYNC_PI_EXTENSIONS` is stricter and rejects values other than
`0` or `1`.

## Presets and sandbox names

Set a default preset for the current shell:

```bash
export SBXR_PRESET=rust-claude
sbxr new .
```

The equivalent CLI option wins over the environment:

```bash
SBXR_PRESET=rust-claude sbxr --preset rust-codex new .
```

`SBXR_NAME` bypasses project-specific name generation. Reusing one override
across unrelated projects can unintentionally select the same sandbox, so it
is primarily a debugging and automation option. It must be 1–128 ASCII
letters, digits, dots, dashes, or underscores, beginning with a letter or
digit; path separators and traversal components are rejected:

```bash
SBXR_NAME=rust-experiment sbxr up .
```

## Storage and kit development

Without `SBXR_ROOT` or `SBXR_KIT_ROOT`, embedded kits are materialized under:

| Platform | Default directory |
|---|---|
| Linux and other Unix | `$XDG_DATA_HOME/sbxr`, or `~/.local/share/sbxr` |
| macOS | `$XDG_DATA_HOME/sbxr`, or `~/Library/Application Support/sbxr` |
| Windows | `%XDG_DATA_HOME%\sbxr`, `%LOCALAPPDATA%\sbxr`, or `%USERPROFILE%\.local\share\sbxr` |

Use an alternate materialization root:

```bash
SBXR_ROOT=/opt/team/sbxr sbxr doctor
```

Use unpacked kits during launcher development:

```bash
SBXR_KIT_ROOT=/path/to/kits sbxr doctor
```

`SBXR_KIT_ROOT` must contain all expected kit subdirectories and their
`spec.yaml` files.

Declarative environment definitions and desired/applied contract metadata are
stored separately from kit data:

| Platform | Default protected state root |
|---|---|
| Linux and other Unix | `$XDG_STATE_HOME/sbxr`, or `~/.local/state/sbxr` |
| macOS | `$XDG_STATE_HOME/sbxr`, or `~/Library/Application Support/sbxr/state` |
| Windows | `%XDG_STATE_HOME%\sbxr`, `%LOCALAPPDATA%\sbxr\state`, or `%USERPROFILE%\.local\state\sbxr` |

Override it only with a trusted path outside every project/additional
workspace:

```bash
SBXR_STATE_ROOT=/secure/host/state/sbxr sbxr up .
```

On Unix, `sbxr` protects directories with mode `0700` and files with `0600`,
uses atomic replacement, and rejects symlinked authoritative path components
and files. The
generated environment contains paths and kit references but no OAuth tokens,
private keys, or literal secrets. GitHub/GHCR declarations may contain the
fixed absolute host command `/usr/bin/gh auth token`; Docker executes it on the
host and exposes only its proxy sentinel, never the command output, to guests.
See [CREDENTIAL_ARCHITECTURE.md](CREDENTIAL_ARCHITECTURE.md) for the complete
credential data flow and its limits.

## Protected project profiles and direct options

Optional settings are command-line/profile inputs rather than environment
variables. They can appear before or after the command:

```bash
sbxr --wizard .
sbxr --memory 8g --cpus 4 --browser new -y .
sbxr --isolated --network restricted up -y .
sbxr --paseo-relay up -y .
sbxr --config /trusted/team-profile.json up -y .
```

`--wizzard` is retained as a spelling alias for `--wizard`. Direct flags and
the wizard save a validated version-1 JSON file below
`$SBXR_STATE_ROOT/profiles`. Allowed settings are `workspace`, `network`,
`githubProxy: true`, `integrations.browser`, `integrations.orca`,
`integrations.ghcr`, `integrations.paseoRelay`, and
`resources.memory/cpus/rootDisk/dockerDisk`, plus `shell.profile`,
`shell.program`, `shell.prompt`, `shell.promptFile`, `shell.aliasFiles`, and
`shell.binDirs`. Unknown keys, secret-looking keys,
malformed types, unsafe sizes, excessive CPU/memory, and unsafe disk budgets
are rejected. External `--config` files are parsed with the same schema and
copied into protected state; profiles must never contain credentials.

The shell object defaults to:

```json
{
  "profile": "familiar",
  "program": "bash",
  "prompt": "classic",
  "promptFile": null,
  "aliasFiles": [],
  "binDirs": []
}
```

`profile` accepts `familiar`, `minimal`, or `custom`; `program` accepts `bash`,
`zsh`, or `fish`; and `prompt` accepts `classic`, `plain`, or `custom`.
`promptFile` is required for a custom prompt. `aliasFiles` and `binDirs` are
valid only with `profile: "custom"`; their direct CLI flags select that profile
automatically. Every path is relative to the host home
(`~/...` is accepted and normalized), must contain no traversal component, and
retains that relative suffix below the sbxr-owned guest mirror
`~/.local/share/sbxr/host-shell`. Alias and bin lists accept at most 32 unique
entries each. These files are trusted executable inputs and must not contain
secrets. The managed startup files `.bashrc`,
`.zshrc`, and `.config/fish/config.fish` cannot be selected directly; use a
dedicated alias or prompt file so sbxr can retain its integration hook.

`paseoRelay` defaults to `false`. When enabled, sbxr atomically enables Paseo's
authenticated relay in sandbox-local configuration and adds the
sandbox-scoped `relay.paseo.sh:443` endpoint; it does not place relay
credentials in the profile. Disabling it removes that endpoint rule. It is
rejected with the `offline` network mode. Unlike creation-time VM resources,
toggling it does not require sandbox recreation; `sbxr repair` or the next
ensuring command reconciles it.

`network` is also runtime-reconciled and does not change the environment
contract. Switching among `development`, `restricted`, and `offline` adds the
new sandbox-scoped deny rules before removing the prior sbxr-owned rules. Rule
ownership is recorded as exact Docker rule IDs in protected host metadata;
manual and host-global rules are never claimed or removed.

`--root-disk` and `--docker-disk` are passed to Docker Sandboxes only for the
environment lifecycle command. Memory and CPU limits are declared in the
environment file. All four values are contract inputs and require explicit
recreation when changed.

## Optional RocksDB acceleration

Setting `SBXR_ROCKSDB_HOST` opts into the read-only host RocksDB integration:

```bash
SBXR_ROCKSDB_HOST=/opt/rocksdb-10.4.2 sbxr new .
```

The prefix affects the deterministic sandbox identity. Repeat the variable or
the corresponding CLI option on later `shell`, `audit`, `stop`, and `rm`
commands for that sandbox.

## Mirroring controls

Mirroring runs during the sandbox's first bootstrap and is then skipped on
ordinary `sbxr new`/`up`/agent commands. Use `sbxr update .` to deliberately
apply current host configuration and extension inventory to an existing
sandbox. The variables below affect that initial bootstrap or explicit update.

Disable all host agent capability/configuration mirroring:

```bash
SBXR_SYNC_AGENT_CONFIG=0 sbxr update .
```

Disable host VS Code extension mirroring:

```bash
SBXR_SYNC_VSCODE_EXTENSIONS=0 sbxr update .
```

Pi prompts, themes, skills, settings, and raw source extensions are normally
mirrored. npm-based extensions are resolved with `npm install`, with lifecycle
scripts disabled, from the semver ranges declared in the copied host
`package.json`, so each one is selected for the sandbox's own kit-pinned Pi
release instead of copying platform-specific `node_modules` or reusing a host
lockfile pinned against the host's Pi. Disable copying raw source extensions
when the host profile is not trusted:

```bash
SBXR_SYNC_PI_EXTENSIONS=0 sbxr update .  # omit raw source extensions
SBXR_SYNC_PI_EXTENSIONS=1 sbxr update .  # explicitly enable them (the default)
```

The setting does not remove Pi package declarations from a mirrored
`settings.json`; use `SBXR_SYNC_AGENT_CONFIG=0` to disable all agent capability
and configuration mirroring.

The default `familiar` Bash profile extracts alias definitions from the host
`~/.bashrc`, mirrors the whole of `~/.bash_aliases` and `~/bin`, and wires them
into interactive shells. For Zsh it uses alias definitions from `~/.zshrc`
plus `~/.zsh_aliases`; for Fish it uses alias definitions from
`~/.config/fish/config.fish` plus
`~/.config/fish/conf.d/aliases.fish`. Missing conventional files are simply
omitted. `minimal` selects no conventional host files. `custom` uses only the
explicit repeatable `--alias-file`, `--bin-dir`, and optional `--prompt-file`
paths. The wizard configures the same fields. Known credential stores, their
children, and parent selections that would contain them are rejected. The same
check runs when the profile is validated and immediately before transfer.
Selected top-level paths may be symlinks (as dotfile managers create for
`~/.bash_aliases` or `~/bin`); they are mirrored by content, and both the
selected name and its resolved target must clear the protected-store check.
The resolved target of an explicit selection must also remain inside the
canonical host home, so a link cannot route an external file or credential
store into the mirror. A conventional alias file or `bin` directory discovered
automatically by `familiar` is instead omitted with a warning when its target
leaves the home; it never blocks an otherwise valid launch. Login and
environment files (`.profile`, `.bash_profile`, `.zprofile`, `.zshenv`,
`.env`, Fish `fish_variables`) are also refused. Nested symlinks are preserved,
as relative links, only when they resolve inside the selected tree; escaping
and broken links are skipped.
Disable all host shell mirroring separately:

```bash
SBXR_SYNC_HOST_SHELL=0 sbxr update .
```

Selected files and directories are copied below one sbxr-owned guest root,
`~/.local/share/sbxr/host-shell/<host-relative path>`, never onto the matching
path in the sandbox home, so a selection such as `.local/bin` cannot replace
sandbox-managed tools. Host paths that point into a mirrored selection
(`/host/home/bin/x`, `~/bin/x`, `$HOME/bin/x`) are rewritten to the mirror;
other absolute host-home paths map to `/home/agent`. Mirrored host utilities
are appended to `PATH`, so a sandbox-managed toolchain of the same name keeps
priority. Any `.git` directory below a mirrored shell tree is never copied.
Each refresh rebuilds the whole mirror in a staging directory and swaps it in,
which removes stale selections when changing to `minimal` or another profile.
sbxr uses a source metadata fingerprint (type, size, mtime, and permission
bits, plus the generated overlay itself) to detect host alias or utility
changes on the next ensuring command. `sbxr update` forces a complete refresh.
Fish familiar-mode extraction copies only top-level one-line `alias`
statements; functions belong in a dedicated selected `.fish` file.

## Local kit-composition cache

Docker Sandboxes automatically assembles source-form Kits v3 into
content-addressed images in its private local runtime during normal sandbox
creation. The committed kit specifications are sbxr's deterministic generic
base recipe; personal configuration is applied later and is not part of that
cache.

Run `sbxr setup --prebuild` to warm the default preset's composition before
creating the first project. The disposable warm-up sandbox has no workspace or
host-personalization inputs and is removed afterward. Nothing is exported or
pushed to a registry, and no host Docker Engine is required. The warm-up uses
the default Bash profile; selecting Zsh or Fish adds that shell's small kit
layer on first use. A later prebuild invocation removes abandoned
`sbxr-prebuild-*` sandboxes whose owning process no longer exists.

The cache reuses kit assembly only. A real project sandbox still creates fresh
persistent volumes and runs the kits' idempotent install hooks to initialize
and verify them. sbxr does not save the warm-up sandbox as a template because
Docker templates omit mounted filesystems, including sbxr's volume-backed Rust
toolchain and Pi state. Docker manages the composition image cache; the broad,
destructive `sbx reset` command clears it together with all sandbox state and
authentication.

## `XDG_STATE_HOME` compatibility

By default, `sbxr` removes `XDG_STATE_HOME` when starting the `sbx` and VS Code
CLIs. Docker Sandboxes creates Unix sockets below its state directory, and a
long custom state path can exceed the operating system's Unix-socket length
limit.

This removal applies only to child CLIs. `sbxr` itself still uses
`XDG_STATE_HOME/sbxr` for protected declarative state unless
`SBXR_STATE_ROOT` overrides it.

Only preserve the host value when it is intentionally configured and short:

```bash
SBXR_PRESERVE_XDG_STATE_HOME=1 sbxr new .
```

Leaving this variable unset avoids the long-socket-path startup failure.

## Standard host variables

The launcher also respects standard operating-system variables:

| Variable | Use |
|---|---|
| `PATH` | Finds `sbx`, VS Code, SSH, Git, and optional host Pi |
| `PATHEXT` | Finds executable extensions on Windows |
| `HOME` or `USERPROFILE` | Locates user configuration and authentication files |
| `XDG_DATA_HOME` | Selects the base data directory when `SBXR_ROOT` is unset |
| `LOCALAPPDATA` | Windows data-directory fallback |
| `XDG_STATE_HOME` | Selects `sbxr`'s protected state root when `SBXR_STATE_ROOT` is unset; passed to child CLIs only with `SBXR_PRESERVE_XDG_STATE_HOME=1` |
| `SSH_AUTH_SOCK` | Preferred host SSH agent; if it is unusable, `sbxr` checks standard GCR, GnuPG, and GNOME Keyring sockets for an agent with loaded keys |

The former `RUST_SBXR_*` names are not recognized after the project was renamed
to `sbxr`; update shell profiles and automation to use `SBXR_*`.
