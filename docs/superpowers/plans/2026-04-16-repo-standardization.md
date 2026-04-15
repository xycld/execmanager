# Repo Standardization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `/workspace/Projects/execmanager` into a clean, publishable Git repository with essential metadata files and a GitHub remote, without adding community/CI overhead.

**Architecture:** This work is intentionally minimal. The repository root becomes the Git root, repository metadata lives in root-level files, and no product/runtime behavior is changed. The pass standardizes Git identity and publishable documentation while leaving future community/process files for later.

**Tech Stack:** Git, GitHub remote configuration, Markdown docs, MIT license text, Rust workspace conventions

---

## File Structure

**Create:**
- `.gitignore` — ignore build outputs, editor/OS junk, and local `.sisyphus/` state
- `README.md` — project description, architecture overview, crate layout, and run/test commands
- `LICENSE` — MIT license text
- `docs/superpowers/plans/2026-04-16-repo-standardization.md` — this plan file

**Modify / initialize:**
- `.git/` — initialized local Git repository
- local Git config / branch metadata — set default branch to `main`
- local Git remote metadata — set `origin` to `https://github.com/xycld/execmanager`

## Task 1: Initialize local Git repository

**Files:**
- Modify: repository metadata under `.git/`

- [ ] **Step 1: Verify the directory is not already a Git repo**

Run:

```bash
git rev-parse --is-inside-work-tree
```

Expected: command fails with a non-zero exit status because the directory is not yet a Git repository.

- [ ] **Step 2: Initialize the repository and set the primary branch**

Run:

```bash
git init
git branch -M main
```

Expected: `.git/` is created and the current branch name becomes `main`.

- [ ] **Step 3: Verify Git initialization succeeded**

Run:

```bash
git status --short --branch
```

Expected: output begins with `## No commits yet on main`.

## Task 2: Add root repository hygiene files

**Files:**
- Create: `.gitignore`
- Create: `LICENSE`

- [ ] **Step 1: Write the failing repository-hygiene check**

Run:

```bash
test -f .gitignore && test -f LICENSE
```

Expected: FAIL because those files do not exist yet.

- [ ] **Step 2: Create `.gitignore`**

Write this file:

```gitignore
target/

.sisyphus/

.DS_Store
.idea/
.vscode/
*.swp
*.swo
```

- [ ] **Step 3: Create `LICENSE` with MIT text**

Write the standard MIT license text using the repository owner name `xycld` as the copyright holder.

- [ ] **Step 4: Run the hygiene check again**

Run:

```bash
test -f .gitignore && test -f LICENSE
```

Expected: PASS.

## Task 3: Add repository README

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write the failing README check**

Run:

```bash
test -f README.md
```

Expected: FAIL because the file does not exist yet.

- [ ] **Step 2: Create `README.md`**

Write a concise README with these exact sections:

```markdown
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
```

- [ ] **Step 3: Verify the README contains the required sections**

Run:

```bash
grep -q "# ExecManager" README.md && grep -q "## Architecture" README.md && grep -q "cargo verify" README.md
```

Expected: PASS.

## Task 4: Configure GitHub remote

**Files:**
- Modify: local Git remote metadata only

- [ ] **Step 1: Write the failing remote check**

Run:

```bash
git remote get-url origin
```

Expected: FAIL because `origin` is not configured yet.

- [ ] **Step 2: Add the remote**

Run:

```bash
git remote add origin https://github.com/xycld/execmanager
```

- [ ] **Step 3: Verify the remote URL**

Run:

```bash
git remote get-url origin
```

Expected:

```text
https://github.com/xycld/execmanager
```

## Task 5: Final repo-standardization verification

**Files:**
- Verify: `.gitignore`
- Verify: `README.md`
- Verify: `LICENSE`

- [ ] **Step 1: Verify Git branch and status**

Run:

```bash
git status --short --branch
```

Expected: output references branch `main` and lists the newly created files as untracked or staged, depending on implementation progress.

- [ ] **Step 2: Verify ignore rules behave correctly**

Run:

```bash
git check-ignore target .sisyphus .vscode 2>/dev/null
```

Expected: each path is reported as ignored by `.gitignore`.

- [ ] **Step 3: Verify repository basics exist together**

Run:

```bash
test -f .gitignore && test -f README.md && test -f LICENSE && git remote get-url origin
```

Expected: PASS, and the final command prints the GitHub URL.

- [ ] **Step 4: Commit**

If the user wants a commit after implementation, use a single commit for this repo-standardization pass because the changed files form one atomic repository-metadata unit.

Recommended commit message:

```bash
git add .gitignore README.md LICENSE .cargo/config.toml docs/superpowers/specs/2026-04-16-repo-standardization-design.md docs/superpowers/plans/2026-04-16-repo-standardization.md
git commit -m "Initialize repository metadata and GitHub remote"
```

If the user does not request a commit, skip this step.

## Self-Review

- **Spec coverage:** This plan covers all approved first-pass scope: git init, `main` branch, `.gitignore`, `README.md`, MIT `LICENSE`, and GitHub remote setup.
- **Placeholder scan:** No TBD/TODO placeholders remain in the executable steps.
- **Type consistency:** File paths and command expectations are consistent with the approved design and current workspace layout.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-16-repo-standardization.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
