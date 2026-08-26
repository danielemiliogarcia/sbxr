# Changelog

## Unreleased

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
