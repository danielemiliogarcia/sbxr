# Agent instructions

These instructions apply to the whole repository. Keep this file concise: the
linked documents are authoritative and should not be duplicated here.

## Read before changing the project

- Read [DEVELOPMENT.md](DEVELOPMENT.md) for architecture, dependency handling,
  required checks, live verification, and handoff requirements.
- Read [SECURITY.md](SECURITY.md) before changing authentication, credentials,
  host configuration mirroring, network policy, mounts, signing, or supply-chain
  behavior.
- Read [CREDENTIAL_ARCHITECTURE.md](CREDENTIAL_ARCHITECTURE.md) before changing
  any agent authentication or credential flow.
- Read [RELEASING.md](RELEASING.md) before preparing or publishing a release.

## Change and release authority

- Record user-visible changes under `Unreleased` in `CHANGELOG.md`.
- A normal implementation or test request does not authorize a version bump,
  commit, push, pull request, merge, tag, or GitHub Release.
- Do not perform any of those repository-writing or publishing actions unless
  the user explicitly requests them. Treat each requested scope literally; for
  example, permission to push does not by itself authorize merging or releasing.
- Do not bump versions merely because the working tree changed. When the user
  explicitly requests a release, follow `RELEASING.md` exactly and keep
  `Cargo.toml`, `Cargo.lock`, `VERSION`, and the changelog release heading in
  agreement.
- After an explicitly requested release, prepare a short user-facing GitHub
  Release summary derived from the matching changelog section. Do not replace
  the detailed changelog with release-page prose.
- Release tags require the configured OpenPGP/YubiKey signer. Immediately before
  user interaction is expected, clearly notify the user and make a best-effort
  audible alert with an available host sound mechanism or terminal bell. Never
  automate, suppress, or bypass PIN entry or physical touch confirmation.

## Verification and sandbox safety

- Run the checks required by `DEVELOPMENT.md` and report exactly which checks and
  live scenarios ran. Never describe an unexecuted or incomplete check as
  passing.
- Changes to lifecycle, kits, authentication, host mirroring, VS Code, shell
  behavior, Docker Sandboxes compatibility, or platform integration require a
  real disposable-sandbox test. Recreate the disposable sandbox when creation,
  bootstrap, or environment-contract behavior changed.
- Never remove or recreate an existing sandbox unless the user explicitly
  identifies it as disposable or directly authorizes its removal. Use a separate
  test project so live verification cannot destroy the user's active sandbox or
  sandbox-local state.
- Preserve unrelated working-tree changes and host project files throughout
  testing.

## Host and security boundaries

- Do not install, upgrade, downgrade, or remove `sbx`, Docker packages, host
  tools, or other host dependencies without explicit user authorization.
- When assessing compatibility with a new Docker Sandboxes release, inspect its
  official release information and, when practical, audit the candidate package,
  CLI surface, structured output, and environment schema without installing it.
- Keep reusable credentials on the host behind Docker Sandboxes credential
  proxying. Do not copy host credential stores into a sandbox.
- SSH-agent forwarding is an intentional, separately documented exception; do
  not silently disable it or broaden it. OpenPGP/GPG and YubiKey signing remain
  host-side through `sbxr commit-pgp-sign`.
- Never put credentials, personalized host configuration, or other user state in
  a cached or published sandbox image.
