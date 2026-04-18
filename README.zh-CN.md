# ExecManager

[English README](README.md)

ExecManager 是一个面向 Kimi Code 的本地 installer-grade 集成层。

运行一个命令，完成本地 hook + per-user daemon 安装；需要时显式管理；不用时尽量干净卸载。

## 快速开始

先通过官方安装脚本安装最新 release 二进制，然后运行本地安装流程：

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash
execmanager
```

如果你想安装 `main` 上最新成功 CI 的 snapshot 测试版，而不是最新正式 release：

```bash
curl -fsSL https://raw.githubusercontent.com/xycld/execmanager/main/install.sh | bash -s -- --snapshot
execmanager
```

如果你是在本地开发，而不是从 release 安装：

```bash
cargo build -p execmanager-cli
./target/debug/execmanager
```

后续常用命令：

```bash
execmanager
execmanager status
execmanager doctor
execmanager uninstall --restore
```

## 它能做什么

- 安装本地 Kimi 集成
- 注册并管理 per-user daemon
- 显式管理 hook 和 service 状态
- 支持安全卸载与尽量恢复安装前状态的卸载

## 首次运行

当你在交互式终端中第一次运行 `execmanager`，且当前尚未安装时，它会表现为一个本地安装器：

1. 检测当前用户环境
2. 选择 Kimi 集成路径
3. 准备 hook、service、runtime 状态
4. 请求确认
5. 执行安装事务
6. 启动 daemon 路径
7. 验证 readiness

如果当前终端不可交互，它不会静默应用变更，而是明确提示你回到交互式终端重新执行。

## 命令面

当前 CLI 提供：

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

## 文档

当前仓库默认提供中英文双 README：

- [English README](README.md)

## Release 安装与 snapshot 制品

安装脚本默认下载 GitHub Releases 中最新的 Linux/macOS 二进制。

Pull request 和推送到 `main` 会运行 CI，并产出用于测试和提前验证的 snapshot 制品。
推送类似 `v0.1.0` 的版本标签时，会触发 release workflow，并发布面向正常安装的 Linux/macOS `execmanager` 二进制文件。

`install.sh --snapshot` 是显式安装最新 snapshot 测试版的入口。默认安装路径仍然是最新正式 GitHub Release。

## 当前限制

- 当前只支持 `kimi` 作为已选适配器
- macOS 命令映射已实现，但当前直接验证环境仍以 Linux 为主
- 产品已经具备 installer-grade 核心流程，但仍处于持续演进阶段
