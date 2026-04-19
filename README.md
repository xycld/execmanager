# ExecManager

Hook into your Code Agent's exec tool to intercept, record, snapshot, and manage command execution.

Currently, ExecManager only supports Kimi Code.

## Install

Install the latest stable release:

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash
execmanager
```

Install the latest snapshot build from `main`:

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash -s -- --snapshot
execmanager
```

Snapshot builds are pre-release builds published from the latest successful CI run on `main`. They are useful when you want the newest changes before the next tagged release, but they may be less stable than the regular release build.

## Usage

Run the CLI help for commands and options:

```bash
execmanager -h
```

---

[中文](README.zh-CN.md)
