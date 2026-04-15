# ExecManager Repository Standardization Design

**Date:** 2026-04-16  
**Scope:** First-pass repository standardization only  
**Status:** Approved design draft

## Goal

Turn `/workspace/Projects/execmanager` into a clean, publishable Git repository that can be connected to `https://github.com/xycld/execmanager`, while intentionally limiting scope to repository basics.

## Included in Scope

This first pass will include:

1. **Git initialization**
   - Initialize the current directory as a Git repository.
   - Set the default local branch to `main`.
   - Configure remote `origin` to `https://github.com/xycld/execmanager`.

2. **Essential repository files**
   - Add a Rust-workspace-aware `.gitignore`.
   - Add a `README.md` describing the project and current state.
   - Add an `MIT` `LICENSE` file.

3. **Repository hygiene decisions**
   - Ignore build outputs such as `target/`.
   - Ignore local/editor artifacts.
   - Ignore `.sisyphus/` state and other local execution/planning artifacts that should not be published as project source.

## Explicitly Out of Scope

This pass will **not** include:

- `CONTRIBUTING.md`
- issue templates or PR templates
- GitHub Actions / CI workflows
- release automation
- badges / polished public branding work
- package publishing metadata beyond what already exists in the Rust workspace
- pushing to GitHub automatically

## Design Details

### Git Layout

- The repository root remains `/workspace/Projects/execmanager`.
- No worktree or branch restructuring is part of this pass.
- No history rewriting is needed because the directory is not yet a Git repository.

### `.gitignore`

The `.gitignore` should be conservative and repo-focused:

- Rust build outputs: `target/`
- OS/editor artifacts: `.DS_Store`, `.idea/`, `.vscode/`, swap files
- local execution/state artifacts: `.sisyphus/`

The key policy decision is that `.sisyphus/` is treated as local planning/runtime state rather than publishable source.

### `README.md`

The README should optimize for clarity over polish. It should include:

1. **Project summary**
   - What ExecManager is
   - Why it exists

2. **Current capabilities**
   - Kimi-hosted managed exec ingress
   - daemon-owned execution and replayable journal
   - rm safety layer
   - Linux/macOS governance model
   - service/port visibility
   - reconciliation / ghost handling
   - TUI + viewer attachment

3. **Architecture overview**
   - host ingress → daemon → TUI / viewers

4. **Workspace structure**
   - short explanation of the crates

5. **Commands**
   - `cargo verify`
   - `cargo test --workspace --all-targets -- --nocapture`
   - `cargo build --workspace`

6. **Platform scope**
   - Linux and macOS only

### `LICENSE`

- License choice: **MIT**

## Implementation Principles

- Keep the pass minimal and reversible.
- Do not mix repository standardization with product feature work.
- Do not introduce community/CI/process files yet.
- Do not push or publish automatically.

## Risks and Mitigations

### Risk: `.sisyphus/` contains useful context

Mitigation: treat it as local working state for now. If parts of it later need to become public docs, they can be selectively promoted into normal tracked documentation.

### Risk: README becomes stale quickly

Mitigation: keep it high-signal and capability-oriented, not over-detailed or roadmap-heavy.

### Risk: remote URL is configured before repo is actually pushed

Mitigation: this is acceptable. Setting `origin` does not imply pushing.

## Acceptance Criteria

- The directory is a valid Git repository.
- The local default branch is `main`.
- `origin` points to `https://github.com/xycld/execmanager`.
- `.gitignore` exists and correctly excludes build/local state artifacts.
- `README.md` exists and accurately describes the current project.
- `LICENSE` exists with MIT text.
- No community/CI/release files are added in this pass.
