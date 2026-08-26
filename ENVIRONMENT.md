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
| `SBXR_CLAUDE_AUTH_FILE` | File path | `~/.claude/.credentials.json`; used by the first-open import offer and `auth-import` |
| `SBXR_YES` | `1` to confirm | Automatically accepts the first-open Claude import offer and non-interactive `auth-import` |
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
is primarily a debugging and automation option:

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
private keys, literal secrets, or host secret commands.

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
mirrored. npm-based extensions are resolved with `npm ci` from the copied host
lockfile, with lifecycle scripts disabled, so their exact locked versions are
installed for the sandbox instead of copying platform-specific `node_modules`.
Disable copying raw source extensions when the host profile is not trusted:

```bash
SBXR_SYNC_PI_EXTENSIONS=0 sbxr update .  # omit raw source extensions
SBXR_SYNC_PI_EXTENSIONS=1 sbxr update .  # explicitly enable them (the default)
```

The setting does not remove Pi package declarations from a mirrored
`settings.json`; use `SBXR_SYNC_AGENT_CONFIG=0` to disable all agent capability
and configuration mirroring.

## Authentication import variables

`SBXR_CLAUDE_AUTH_FILE` selects a trusted host Claude OAuth cache for the
first-open import offer and the explicit `auth-import` command:

```bash
SBXR_CLAUDE_AUTH_FILE=/secure/path/claude-credentials.json \
  sbxr auth-import .
```

`SBXR_YES=1` bypasses the interactive warning for automation:

```bash
SBXR_YES=1 sbxr auth-import .
```

This copies Claude refresh credentials into the project sandbox. Code, build
scripts, procedural macros, and agents running as the sandbox user can read
them. Use `SBXR_YES=1` only for an already trusted project and automation
environment. Codex OAuth is host-managed by Docker and is never selected with
these variables.

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
