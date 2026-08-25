# Platform support

`sbxr` is a native host launcher for Docker Sandboxes. The launcher runs on the
developer's operating system, while the Rust toolchain, coding agents, and
project commands run in a Linux microVM managed by Docker Sandboxes.

The labels below distinguish platforms exercised end to end from platforms
that the code is designed to support but which still need live validation.

| Host platform | `sbxr` status | Notes |
|---|---|---|
| Linux Mint 22.3, x86-64 | Tested | Live development and kit validation have passed; Docker does not officially support Ubuntu derivatives |
| Ubuntu 24.04+, x86-64 or ARM64 | Expected | Supported by Docker Sandboxes; dedicated `sbxr` validation is pending |
| macOS 14+ on Apple silicon | Expected | Supported by Docker Sandboxes; dedicated `sbxr` validation is pending |
| Windows 11, x86-64 | Expected | Supported by Docker Sandboxes; dedicated `sbxr` validation is pending |
| Windows through WSL | Experimental | Native Windows is preferred; WSL paths have additional Git and filesystem caveats |
| macOS on Intel | Unsupported | Docker Sandboxes requires Apple silicon |

“Expected” is not a release guarantee. A platform should move to “Tested” only
after the build, host diagnostics, sandbox lifecycle, authentication, native VS
Code connection, and Rust workflow have all been exercised on that host.

## Common requirements

Every host needs:

- the Docker Sandboxes `sbx` CLI, signed in with `sbx login`;
- Rust and Cargo 1.85 or newer to build and install `sbxr`;
- native VS Code with its `code` launcher on `PATH`;
- an OpenSSH client and the VS Code Remote-SSH extension;
- Git for `sbxr review`;
- hardware virtualization supported and enabled.

Docker Desktop and Docker Engine are not required merely to run Docker
Sandboxes. Refer to Docker's current
[installation guide](https://docs.docker.com/ai/sandboxes/install/) before
installing because its supported versions and commands can change.

After installing the host prerequisites, the common setup is:

```text
sbx login
sbxr setup
sbxr doctor
```

## Linux

Docker officially supports Ubuntu 24.04 or newer on 64-bit Intel, AMD, and Arm
processors. KVM must be available, hardware virtualization must be enabled,
and the developer must be able to access `/dev/kvm`.

Install the `sbx` package by following Docker's
[Ubuntu instructions](https://docs.docker.com/ai/sandboxes/install/#install-on-ubuntu).
Docker warns that it does not test or support Ubuntu derivatives such as Linux
Mint, even though `sbxr` has been exercised successfully on Linux Mint 22.3.

Linux is currently the only host platform supported by the optional
`--rocksdb-host` acceleration. Without that option, RocksDB and other native
dependencies build normally inside the sandbox.

For targeted graceful closing before `sbxr stop`, install `wmctrl` on an X11
desktop (`sudo apt-get install wmctrl` on Ubuntu). If the window cannot be
targeted safely, `sbxr` does not stop the microVM until the developer closes
that Remote-SSH window manually. Native Wayland, macOS, and Windows automatic
window closing still require dedicated live validation.

## VS Code state across a microVM stop

`sbxr stop` closes the matching Remote-SSH window gracefully before powering
off the microVM. VS Code stores its window/workspace state on the host, while
the remote VS Code Server data remains on the sandbox's persistent disk. The
launcher also enables VS Code's terminal process-revive settings and requests
10,000 lines of persistent scrollback.

A powered-off microVM cannot retain processes that existed only in guest RAM,
including the remote PTY host and shells. VS Code therefore has to relaunch the
shells and replay its serialized terminal buffer on the next connection; this
is different from closing and reopening VS Code while the sandbox remains
running, where it can reconnect to the original PTY. VS Code documents these
as [process revive and process reconnection](https://code.visualstudio.com/docs/terminal/advanced/#_persistent-sessions).

Live testing with VS Code 1.134.0 and Remote-SSH 0.124.0 confirmed that `sbxr`
successfully caused the terminal snapshot—including command/output text—to be
written to the host workspace database. That VS Code release consumed the
snapshot and recreated the terminal tab and shell after `sbx stop`, but did not
render the saved scrollback. Full remote session persistence across loss of the
remote host is also still an open upstream feature area; see
[microsoft/vscode#320044](https://github.com/microsoft/vscode/issues/320044).
Consequently, terminal tabs and working directories should revive, but exact
scrollback restoration after a full microVM stop is currently best-effort and
must be retested when VS Code or Remote-SSH changes. Persistent project files,
agent configuration, installed extensions, and normal remote workspace state
are unaffected by this limitation.

## macOS

Docker Sandboxes currently requires macOS Sonoma 14 or newer and Apple
silicon. Install it with Homebrew:

```bash
brew trust docker/tap
brew install docker/tap/sbx
sbx login
```

`sbxr` already selects the normal macOS application-data location, handles
Unix permissions, and does not run Linux-only KVM checks on macOS. Its embedded
kits include the ARM64 Linux Rust target used by an Apple-silicon sandbox.

Live macOS validation is still pending. Also ensure that the VS Code `code`
launcher is available in the shell; VS Code can install it through the command
palette action **Shell Command: Install 'code' command in PATH**.

The optional `--rocksdb-host` integration is unavailable on macOS.

## Windows

Docker Sandboxes currently requires Windows 11, a 64-bit Intel or AMD
processor, and Windows Hypervisor Platform. Enable the platform from an
elevated PowerShell session, reboot if requested, and install `sbx`:

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform -All
winget install -h Docker.sbx
sbx login
```

Run `sbxr` natively from PowerShell or Windows Terminal for the initial
Windows validation. The launcher handles `%LOCALAPPDATA%`, `%USERPROFILE%`,
Windows executable extensions, and native project paths. Docker's SSH setup
writes its managed configuration under `%USERPROFILE%\.ssh\config`.

Live Windows compilation and end-to-end validation are still pending. Some
`sbxr` missing-dependency messages currently contain Ubuntu-oriented examples;
the linked vendor documentation remains authoritative for Windows setup. The
optional `--rocksdb-host` integration is unavailable on Windows.

## WSL considerations

WSL is not the recommended first Windows setup for `sbxr`. Prefer installing
and invoking `sbx`, `sbxr`, VS Code, SSH, and Git as native Windows programs.

Docker documents a known clone-mode issue for repositories addressed through
`\\wsl.localhost\...`: native Windows Git can reject the repository because its
owner differs. This particularly affects `sbxr review`, which uses clone mode.
See Docker's
[WSL clone-mode troubleshooting](https://docs.docker.com/ai/sandboxes/troubleshooting/#clone-mode-reports-not-in-a-git-repository-on-wsl)
for the current workaround.

## What must be validated on a new platform

Before marking another host as tested, verify:

1. `cargo build --frozen` and `cargo test --frozen` pass.
2. `cargo install --locked --force --path .` installs a working native binary.
3. `sbxr setup` and `sbxr doctor` correctly diagnose the host.
4. `sbxr new .`, `status`, `stop`, and reconnecting to the same sandbox work.
5. Native VS Code connects through Remote-SSH, mirrors compatible extensions,
   preserves remote window state, and opens the intended project directory.
6. Codex, Claude, and Pi subscription authentication works as documented.
7. Rust build, test, debugger, source navigation, and Cargo audit workflows run
   inside the sandbox.
8. Paths containing spaces and non-ASCII characters work correctly.

Docker's [editor integration guide](https://docs.docker.com/ai/sandboxes/integrations/)
describes the platform-specific SSH configuration used by `sbxr`.
