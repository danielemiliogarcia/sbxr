# Changelog

## Unreleased

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
