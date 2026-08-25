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
| `SBXR_ROCKSDB_HOST` | RocksDB installation prefix | Unset and disabled; overridden by `--rocksdb-host[=PREFIX]` |
| `SBXR_SYNC_AGENT_CONFIG` | `0` to disable | Safe Codex, Claude, and Pi capability mirroring is enabled |
| `SBXR_SYNC_VSCODE_EXTENSIONS` | `0` to disable | Host VS Code extension mirroring is enabled |
| `SBXR_SYNC_PI_EXTENSIONS` | `0` or `1` | Unset means executable Pi extensions/packages are copied only when host and sandbox Pi versions match |
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

## Optional RocksDB acceleration

Setting `SBXR_ROCKSDB_HOST` opts into the read-only host RocksDB integration:

```bash
SBXR_ROCKSDB_HOST=/opt/rocksdb-10.4.2 sbxr new .
```

The prefix affects the deterministic sandbox identity. Repeat the variable or
the corresponding CLI option on later `shell`, `audit`, `stop`, and `rm`
commands for that sandbox.

## Mirroring controls

Disable all host agent capability/configuration mirroring:

```bash
SBXR_SYNC_AGENT_CONFIG=0 sbxr new .
```

Disable host VS Code extension mirroring:

```bash
SBXR_SYNC_VSCODE_EXTENSIONS=0 sbxr new .
```

Pi prompts, themes, skills, and safe settings are normally mirrored. Executable
Pi extensions/packages additionally require matching host and sandbox Pi
versions. Override that decision only after a compatibility and trust review:

```bash
SBXR_SYNC_PI_EXTENSIONS=0 sbxr new .  # always disable executable Pi additions
SBXR_SYNC_PI_EXTENSIONS=1 sbxr new .  # force-enable them
```

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
| `XDG_STATE_HOME` | Passed to child CLIs only with `SBXR_PRESERVE_XDG_STATE_HOME=1` |

The former `RUST_SBXR_*` names are not recognized after the project was renamed
to `sbxr`; update shell profiles and automation to use `SBXR_*`.
