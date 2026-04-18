# ExecManager

[中文说明 / 中文版 README](README.zh-CN.md)

ExecManager is a local installer-grade integration layer for Kimi Code.

Run one command, set up the local hook + per-user daemon, use it normally, and remove it cleanly when you no longer want it.

## Quick start

Install the latest released binary, then run the installer-grade setup flow:

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash
execmanager
```

To install the latest CI snapshot build from `main` instead of the latest formal release:

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash -s -- --snapshot
execmanager
```

If you are developing locally instead of installing from a release:

```bash
cargo build -p execmanager-cli
./target/debug/execmanager
```

Useful follow-up commands:

```bash
execmanager
execmanager status
execmanager doctor
execmanager uninstall --restore
```

## What it does

- installs the local Kimi integration
- registers and manages a per-user daemon
- keeps hook and service state explicit
- supports safe uninstall and best-effort restore uninstall

## First run

When you run `execmanager` in an interactive terminal and it is not installed yet, it behaves like a local installer:

1. detects the current-user environment
2. prepares the Kimi hook + per-user service plan
3. asks for confirmation
4. applies the install transaction
5. starts the daemon path
6. verifies readiness

If no interactive terminal is available, it does not apply changes silently. It prints guidance and asks you to rerun from an interactive terminal.

## Command surface

The current CLI surface is:

```bash
execmanager
execmanager init
execmanager status
execmanager doctor
execmanager service start
execmanager service stop
execmanager service restart
execmanager hooks install
execmanager hooks repair
execmanager uninstall
execmanager uninstall --restore
```

## Documentation

For a Chinese overview, installation notes, and command summary, see:

- [README.zh-CN.md](README.zh-CN.md)

## Release install vs snapshot artifacts

The install script downloads the latest GitHub Release binary for Linux/macOS.

Pull requests and pushes to `main` run CI and publish snapshot artifacts for testing and early validation.
Pushing a version tag like `v0.1.0` triggers the release workflow and publishes Linux/macOS `execmanager` binaries for normal installation.

`install.sh --snapshot` is the explicit opt-in path for installing the latest snapshot build from CI. The default install path remains the latest formal GitHub Release.

## Current limits

- only `kimi` is supported as the selected adapter
- macOS command mapping is implemented, but the current direct validation environment is still Linux-first
- the product now has installer-grade core flows, but it is still actively evolving
