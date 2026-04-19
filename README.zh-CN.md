# ExecManager

对 Code Agent 的 Exec Tool 进行 Hook，接管命令执行，记录日志，生成快照，统一管理。

目前 ExecManager 仅支持 Kimi Code。

## 安装

安装最新稳定版：

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash
execmanager
```

安装基于 `main` 分支最新提交生成的快照版：

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash -s -- --snapshot
execmanager
```

快照版是从 `main` 分支最近一次成功 CI 构建发布的预发布版本。适合想先体验最新改动的场景，但稳定性可能不如常规正式版。

## 用法

查看 CLI 帮助以获取命令和选项：

```bash
execmanager -h
```

---

[English](README.md)
