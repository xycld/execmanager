# ExecManager

ExecManager is a managed execution layer for code-agent-driven development workflows.

## What it does

- routes supported Kimi-hosted shell exec through a daemon-owned execution path
- records append-only execution history and replayable projections
- applies narrow fail-closed safety controls for destructive commands like `rm`
- exposes Linux/macOS resource-governance state honestly
- tracks service/port visibility, reconciliation state, TUI views, and attach-only viewer handles

## Architecture

The system is structured as:

- host ingress (`execmanager-host-kimi`)
- daemon/source of truth (`execmanager-daemon`)
- platform governance (`execmanager-platform`)
- attach-only viewer adapters (`execmanager-viewers`)
- projection-backed TUI (`execmanager-tui`)

## Workspace layout

- `crates/execmanager-contracts`
- `crates/execmanager-host-kimi`
- `crates/execmanager-daemon`
- `crates/execmanager-platform`
- `crates/execmanager-viewers`
- `crates/execmanager-tui`

## Supported platforms

- Linux
- macOS

## Verification commands

```bash
cargo verify
cargo test --workspace --all-targets -- --nocapture
cargo build --workspace
```
