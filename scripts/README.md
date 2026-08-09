# Build Scripts / 构建脚本

> [English](#english) | [中文](#中文)

---

## English

### Overview

Local build scripts for nvm-rust. Pick the one for your platform:

| Platform | Build Script | Package Script |
|----------|-------------|----------------|
| Linux | `scripts/build-linux.sh` | `scripts/package.sh` |
| macOS | `scripts/build-macos.sh` | `scripts/package.sh` |
| Windows | `scripts/build-windows.bat` | `scripts/package.sh` |

### Build Scripts Usage

```bash
# Debug build
<script>

# Release build (optimized + LTO, static CRT on Windows)
<script> release

# Full verification (same as CI: fmt + clippy + test)
<script> check

# Quick check (fmt + clippy only, skip tests)
<script> check quick

# Release build with custom version (skips nvm upgrade checks)
<script> release --version 9.9.9
```

Replace `<script>` with your platform's script:
- Linux: `./scripts/build-linux.sh`
- macOS: `./scripts/build-macos.sh`
- Windows: `scripts\build-windows.bat`

### Custom Version (`--version`)

The `--version X.Y.Z` flag temporarily overrides `Cargo.toml`'s version field
before building, then restores it after. This is useful for local builds that
shouldn't trigger `nvm upgrade` checks:

```bash
# Build with version 9.9.9 — nvm upgrade --check will think you're on the latest
./scripts/build-linux.sh release --version 9.9.9
# → Binary reports version 9.9.9
# → Cargo.toml restored to original after build
```

How it works:
- Linux/macOS: saves original `Cargo.toml` content, `sed` replaces version,
  `trap` restores on exit (even on failure).
- Windows: backs up to `Cargo.toml.bak`, PowerShell replaces version,
  restores on success or failure.

### Package Script

After a release build, create a release-ready archive (same as CI produces):

```bash
# Linux/macOS:
./scripts/build-linux.sh release   # or build-macos.sh
./scripts/package.sh target/release/nvm 2.1.0 linux x64
# → nvm-2.1.0-linux-x64.tar.gz

# Windows:
scripts\build-windows.bat release
bash scripts/package.sh target/release/nvm.exe 2.1.0 windows x64
# → nvm-2.1.0-windows-x64.zip

# With custom version:
./scripts/build-linux.sh release --version 9.9.9
./scripts/package.sh target/release/nvm 9.9.9 linux x64
# → nvm-9.9.9-linux-x64.tar.gz
```

The archive contains: binary + README (en/cn) + LICENSE + install scripts
(`install.sh` + `install.ps1`) + shell integration (`nvm.sh` + `nvm.fish`
+ `nvm.psm1`) — the same "friendly-pack" that CI produces.

### CI vs Local

| Aspect | CI (`.github/workflows/`) | Local (`scripts/`) |
|--------|--------------------------|---------------------|
| Rust install | `dtolnay/rust-toolchain` | rustup or `.svc` |
| Cache | `Swatinem/rust-cache` | None |
| Security scan | `rustsec/audit-check` | None |
| Cross-compile | 8 targets in release.yml | Current platform only |
| **Packaging** | **`scripts/package.sh`** | **`scripts/package.sh`** |
| Release upload | GitHub Release API | Manual |

Both CI and local users call the SAME `scripts/package.sh` for packaging.
This is the single source of truth for the friendly-pack layout.

### Prerequisites

#### Linux
- [Rust](https://rustup.rs/)
- For musl (Alpine): `sudo apt-get install musl-tools`

#### macOS
- [Rust](https://rustup.rs/)
- Xcode Command Line Tools: `xcode-select --install`

#### Windows
- [Rust](https://rustup.rs/)
- [Visual Studio Build Tools](https://aka.ms/vs/17/release/vs_BuildTools.exe) — select "Desktop development with C++"
- For packaging: Git Bash (provides `7z` and `tar`)

---

## 中文

### 概述

nvm-rust 的本地构建脚本。根据你的平台选择：

| 平台 | 构建脚本 | 打包脚本 |
|------|---------|---------|
| Linux | `scripts/build-linux.sh` | `scripts/package.sh` |
| macOS | `scripts/build-macos.sh` | `scripts/package.sh` |
| Windows | `scripts/build-windows.bat` | `scripts/package.sh` |

### 构建脚本用法

```bash
# Debug 构建
<script>

# Release 构建（优化 + LTO，Windows 静态链接 CRT）
<script> release

# 完整验证（和 CI 一致：fmt + clippy + test）
<script> check

# 快速检查（只跑 fmt + clippy，跳过测试）
<script> check quick

# 指定自定义版本号构建（跳过 nvm upgrade 检查）
<script> release --version 9.9.9
```

将 `<script>` 替换为你的平台脚本：
- Linux：`./scripts/build-linux.sh`
- macOS：`./scripts/build-macos.sh`
- Windows：`scripts\build-windows.bat`

### 自定义版本号（`--version`）

`--version X.Y.Z` 参数会在构建前临时修改 `Cargo.toml` 的版本字段，构建完成后恢复。
适合不想被 `nvm upgrade` 提示更新的本地构建：

```bash
# 用 9.9.9 版本构建 — nvm upgrade --check 会认为你已经是最新版
./scripts/build-linux.sh release --version 9.9.9
# → 二进制报告版本号为 9.9.9
# → 构建后 Cargo.toml 恢复原值
```

实现原理：
- Linux/macOS：保存原始 `Cargo.toml` 内容，`sed` 替换版本号，
  `trap` 在退出时恢复（即使构建失败也会恢复）。
- Windows：备份到 `Cargo.toml.bak`，PowerShell 替换版本号，
  成功或失败都恢复。

### 打包脚本

Release 构建后，创建和 CI 产出一致的发布包：

```bash
# Linux/macOS：
./scripts/build-linux.sh release   # 或 build-macos.sh
./scripts/package.sh target/release/nvm 2.1.0 linux x64
# → nvm-2.1.0-linux-x64.tar.gz

# Windows：
scripts\build-windows.bat release
bash scripts/package.sh target/release/nvm.exe 2.1.0 windows x64
# → nvm-2.1.0-windows-x64.zip

# 自定义版本号：
./scripts/build-linux.sh release --version 9.9.9
./scripts/package.sh target/release/nvm 9.9.9 linux x64
# → nvm-9.9.9-linux-x64.tar.gz
```

发布包含：二进制 + README（中英文）+ LICENSE + 安装脚本
（`install.sh` + `install.ps1`）+ shell 集成（`nvm.sh` + `nvm.fish`
+ `nvm.psm1`）—— 和 CI 产出的 "friendly-pack" 完全一致。

### CI 与本地对比

| 方面 | CI（`.github/workflows/`） | 本地（`scripts/`） |
|------|---------------------------|-------------------|
| Rust 安装 | `dtolnay/rust-toolchain` | rustup 或 `.svc` |
| 缓存 | `Swatinem/rust-cache` | 无 |
| 安全扫描 | `rustsec/audit-check` | 无 |
| 交叉编译 | release.yml 中 8 个目标 | 仅当前平台 |
| **打包** | **`scripts/package.sh`** | **`scripts/package.sh`** |
| 发布上传 | GitHub Release API | 手动 |

CI 和本地用户调用的是**同一个** `scripts/package.sh`。
这是 friendly-pack 布局的唯一真相来源。

### 前置依赖

#### Linux
- [Rust](https://rustup.rs/)
- musl（Alpine）：`sudo apt-get install musl-tools`

#### macOS
- [Rust](https://rustup.rs/)
- Xcode 命令行工具：`xcode-select --install`

#### Windows
- [Rust](https://rustup.rs/)
- [Visual Studio Build Tools](https://aka.ms/vs/17/release/vs_BuildTools.exe) — 选择"使用 C++ 的桌面开发"
- 打包需要：Git Bash（提供 `7z` 和 `tar`）
