# nvm-rs

nvm-rs 是一个用 Rust 编写、高性能且功能丰富的 Node.js 版本管理器 —— [nvm](https://github.com/nvm-sh/nvm) 的 Rust 原生重写。以单一静态二进制交付，亚毫秒级启动，中英双语界面，并具备 15 项 nvm 和 fnm 都没有的独有功能：GPG 签名校验、断点续传、源码编译、离线安装，以及内置 yarn/pnpm/corepack 集成。为现代工作流而生。

## 特性

- 安装、卸载、切换多个 Node.js 版本
- 支持模糊版本号（`20` → 最新的 `v20.x.x`）
- 内置特殊别名：`node`、`stable`、`unstable`、`lts`、`lts/<代号>`、`system`、`default`
- LTS 代号别名：`lts/argon`、`lts/boron`、`lts/carbon`、`lts/dubnium`、`lts/erbium`、`lts/fermium`、`lts/gallium`、`lts/hydrogen`、`lts/iron`、`lts/jod`
- **io.js 支持** — 安装和管理历史 io.js 版本（`nvm install iojs`）
- 支持 `.nvmrc` 和 `.node-version` 自动切换
- 用户自定义别名
- 镜像源配置（npmmirror、官方、自定义 URL）
- 下载文件 SHA256 校验
- 多架构支持：`x64`、`arm64`（Apple Silicon、ARM Linux）
- 跨平台：Linux、macOS、Windows
- 读取 `NVM_DIR` 环境变量
- Shell 集成：通过 `nvm.sh` 让 `nvm use` 在当前 Shell 立即生效
- 版本间全局 npm 包迁移（`reinstall-packages`）
- 升级指定版本的 npm 到最新（`install-latest-npm`）
- **缓存管理** — `nvm cache dir / list / clear`
- **离线安装** — `nvm install --offline`（仅从缓存安装）
- **源码编译** — `nvm install -s`（从源码包编译安装）
- **复合安装选项** — `--latest-npm` 和 `--reinstall-packages-from=<ver>`
- **LTS 卸载** — `nvm uninstall --lts`
- **国际化** — `nvm language <en|cn>`（英文 / 中文）
- **代理管理** — `nvm proxy on/off`（使用系统代理环境变量）
- **Shell 补全** — `nvm completion <bash|zsh|fish|powershell>`
- **Corepack 支持** — `nvm corepack <enable|disable|status>`
- **递归 .nvmrc 查找** — 自动切换时向上搜索父目录
- **package.json engines.node** — 从 package.json 读取 Node.js 版本要求
- **多 Shell 支持** — bash、zsh、Fish、PowerShell，带自动切换

## 与 fnm、nvm-sh 的对比

| 功能 | nvm-rust（本项目） | fnm | nvm-sh |
|------|------|------|------|
| **实现语言** | Rust | Rust | Bash |
| **启动开销** | 低（原生二进制） | 低（原生二进制） | 高（shell 解析） |
| **单一二进制无运行时依赖** | ✅ | ✅ | ❌（需 bash） |
| **Linux x86_64 (glibc)** | ✅ | ✅ | ✅（shell） |
| **Linux aarch64 (glibc)** | ✅ | ✅ | ✅（shell） |
| **Linux x86_64 (musl/Alpine 静态)** | ✅ | ✅ | ❌（需 bash+coreutils） |
| **macOS x86_64 / aarch64** | ✅ | ✅ | ✅ |
| **Windows 原生** | ❌ | ✅ | ❌（用 nvm-windows） |
| **bash 集成** | ✅ | ✅ | ✅ |
| **zsh 集成** | ✅ | ✅ | ✅ |
| **fish 集成** | ✅ | ✅ | ✅ |
| **PowerShell 集成** | ✅ | ✅ | ❌ |
| **安装指定版本 `install 20`** | ✅ | ✅ | ✅ |
| **安装 LTS `install --lts`** | ✅ | ✅ | ✅ |
| **安装最新版 `install --latest`** | ✅ | ✅ | ✅ |
| **仅未安装时装 LTS `--lts-newer`** | ✅ | ❌ | ❌ |
| **版本号简写 `20` → 最新 20.x** | ✅ | ✅ | ✅ |
| **`lts/*`、`lts/iron` 代号解析** | ✅ | ✅ | ✅ |
| **从源码编译 `--source`** | ✅ | ❌ | ❌ |
| **离线安装 `--offline`（用缓存）** | ✅ | ❌ | ❌ |
| **io.js 安装** | ✅ | ❌ | ✅ |
| **卸载 `uninstall` / `--lts` / `--latest`** | ✅ | ✅ | ✅（仅指定版本） |
| **进度条下载** | ✅（indicatif） | ✅ | ✅（curl/wget） |
| **断点续传** | ✅（HTTP Range + .part） | ❌ | ❌ |
| **镜像源切换 `mirror`** | ✅（taobao/official/自定义） | ❌（靠 env） | ✅（`NVM_NODEJS_ORG_MIRROR`） |
| **本地缓存复用** | ✅ | ✅ | ❌ |
| **缓存 dir/list/clear** | ✅ | ❌ | ❌ |
| **SHA-256 校验** | ✅ | ✅ | ✅ |
| **GPG 签名验证 `SHASUMS256.txt.sig`** | ✅（按需导入公钥） | ❌ | ✅ |
| **跳过验证 `--no-gpg-verify`** | ✅ | — | ❌ |
| **离线自动跳过校验** | ✅ | — | ❌ |
| **装完升级 npm `--latest-npm`** | ✅ | ❌ | ❌ |
| **装完装 yarn `--latest-yarn`** | ✅ | ❌ | ❌ |
| **装完装 pnpm `--latest-pnpm`** | ✅ | ❌ | ❌ |
| **独立命令 `install-latest-npm/yarn/pnpm`** | ✅ | ❌ | ❌ |
| **全局包迁移 `reinstall-packages`** | ✅ | ❌ | ✅ |
| **跨版本迁移 `migrate`（从 nvm-sh/nvm-windows）** | ✅ | ❌ | ❌ |
| **corepack 启用/禁用/状态** | ✅ | ❌ | ❌ |
| **`use <ver>` 切换** | ✅ | ✅ | ✅ |
| **`use` 读 `.nvmrc`/`.node-version`/`package.json#engines`** | ✅ | ✅（`.nvmrc`/`.node-version`） | ✅（`.nvmrc`） |
| **`use --install-if-missing`** | ✅ | ✅ | ✅（`nvm install` 自动） |
| **`use --save` 持久化为默认** | ✅ | ❌ | ✅（`nvm alias default`） |
| **`use --use-on-cd` 装 cd 钩子** | ✅ | ❌（靠 shell 集成） | ❌ |
| **`run <ver> <script>`** | ✅ | ❌ | ✅ |
| **`exec <ver> <cmd>`** | ✅ | ❌ | ✅ |
| **`which [ver]`** | ✅ | ✅ | ✅ |
| **`current` 当前版本** | ✅ | ✅ | ✅ |
| **`deactivate` 还原 PATH** | ✅ | ❌ | ✅ |
| **`unload` 移除 shell 配置** | ✅ | ❌ | ✅ |
| **`alias <name> <ver>`** | ✅ | ✅（`default`/别名） | ✅ |
| **`unalias`** | ✅ | ✅ | ✅ |
| **内置 `node`/`stable`/`unstable`** | ✅ | ❌ | ✅ |
| **内置 `lts`/`lts/<codename>`** | ✅ | ✅ | ✅ |
| **内置 `system`/`default`** | ✅ | ✅ | ✅ |
| **`auto` 按 `.nvmrc` 自动切换** | ✅ | ✅（`--use-on-cd`） | ✅（shell 函数） |
| **`auto --silent` 静默** | ✅ | ❌ | ❌ |
| **`remote`/`ls-remote` 列远程版本** | ✅ | ✅ | ✅ |
| **`--lts` 仅 LTS** | ✅ | ✅ | ✅ |
| **`--lts-old` 仅 LTS ≤18** | ✅ | ❌ | ❌ |
| **`--filter <pattern>` 过滤** | ✅ | ❌ | ❌ |
| **`--sort desc/asc`** | ✅ | ❌ | ✅（默认降序） |
| **`--page <n>` 分页** | ✅ | ❌ | ❌ |
| **美观表格输出（边框/对齐/CJK 宽度）** | ✅ | ❌ | ❌ |
| **`proxy on/off/status` 命令** | ✅ | ❌ | ❌ |
| **自动检测系统代理** | ✅ | ❌ | ❌ |
| **连通性测试（google/baidu）** | ✅ | ❌ | ❌ |
| **自定义 HTTP client** | ✅（reqwest） | ✅ | ❌（curl/wget） |
| **中英文双语 `language en/cn`** | ✅ | ❌ | ❌ |
| **彩色输出** | ✅（colored） | ✅ | ❌ |
| **i18n 帮助文本** | ✅ | ❌ | ❌ |
| **Shell 补全生成 `completion`** | ✅（bash/zsh/fish/powershell） | ✅ | ❌ |
| **统一 `dir` 路径展示** | ✅ | ❌ | ✅（`NVM_DIR`） |
| **GitHub Actions 多平台自动构建** | ✅（6 target） | ✅ | ❌（手动/脚本） |
| **Release 自动生成 + sha256sums** | ✅ | ✅ | ❌ |
| **Homebrew formula** | ✅ | ✅ | ❌（tap 安装脚本） |
| **并发安全** | ✅ | ✅ | ⚠️（shell 依赖） |
| **大版本列表渲染速度** | 快 | 快 | 慢 |
| **nvm-sh 生态成熟度** | — | — | ✅（社区最广、文档最多） |

## 安装

### 一键安装（macOS / Linux）

```bash
curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash
```

**国内镜像加速（脚本本体也走镜像）：**

```bash
curl -fsSL https://ghproxy.com/https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | GITHUB_MIRROR=ghproxy bash
```

### Windows（PowerShell）

```powershell
irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex
```

**国内镜像加速（脚本本体也走镜像）：**

```powershell
$env:GITHUB_MIRROR = 'ghproxy'
irm https://ghproxy.com/https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex
```

### Homebrew（macOS）

```bash
brew tap mose-x/tap
brew install nvm-rust
```

### 从源码编译

```bash
git clone https://github.com/mose-x/nvm-rust.git
cd nvm-rust
cargo build --release
sudo cp target/release/nvm /usr/local/bin/
```

### 手动下载

从 [Releases 页面](https://github.com/mose-x/nvm-rust/releases) 下载最新的二进制文件并加入 PATH。

支持的平台：
- Linux: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- macOS: `x86_64-apple-darwin`, `aarch64-apple-darwin`
- Windows: `x86_64-pc-windows-msvc`

## Shell 集成

安装后，根据你的 Shell 选择对应的脚本：

### Bash / Zsh

```bash
source ~/.nvm.rust/shell/nvm.sh
```

### Fish Shell

```fish
source ~/.nvm.rust/shell/nvm.fish
```

或添加到 `~/.config/fish/config.fish`:

```fish
set -gx PATH ~/.nvm.rust/bin $PATH
source ~/.nvm.rust/shell/nvm.fish
```

### PowerShell

添加到 PowerShell 配置文件 (`$PROFILE`):

```powershell
Import-Module "$env:USERPROFILE\.nvm.rust\shell\nvm.psm1"
```

### 自动切换

当存在 `.nvmrc` 文件时，nvm-rs 会在进入该目录时自动切换到指定的 Node.js 版本。

## 快速开始

```bash
nvm install 20        # 安装 Node.js 20
nvm use 20           # 切换到 Node.js 20
nvm ls               # 列出已安装版本
nvm ls --lts         # 列出 LTS 版本
nvm auto             # 根据 .nvmrc 自动切换
```

### Shell 集成（可选）

```bash
source /path/to/nvm.sh
```

这样 `nvm use` 会立即修改当前 Shell 的 `PATH`，并通过 `cd` hook 在进入目录时根据 `.nvmrc` / `.node-version` 自动切换版本。

## 使用

### 安装 Node.js 版本

```bash
nvm install 20                    # 最新的 v20.x.x
nvm install 20.11.0               # 精确版本
nvm install v18.19.0              # 带 v 前缀
nvm install --lts                 # 最新的 LTS
nvm install --latest              # 最新的发布版
nvm install lts/iron              # 指定 LTS 系列
nvm install node                  # 别名，等同于 "latest"
nvm install stable
nvm install unstable
```

### 使用已安装的版本

```bash
nvm use 20
nvm use v18.19.0
nvm use lts/iron
nvm use lts                       # 任意已安装的 LTS
nvm use system                    # 切换到系统自带的 node
```

如果已 source `nvm.sh`，当前 Shell 立即生效。

### 列出版本

```bash
nvm list            # 本地
nvm ls              # 本地（同 list）
nvm remote          # 远程可下载版本
nvm remote --lts    # 只显示 LTS
nvm current         # 当前使用版本
```

### 卸载

```bash
nvm uninstall 18
nvm remove 18        # uninstall 的别名
```

### NVM 目录

```bash
nvm dir              # 显示 NVM 安装路径和 .nvm.rust 路径
```

### 别名

```bash
nvm alias work 18           # 创建别名
nvm alias work              # 查看别名
nvm alias                   # 列出所有别名
nvm unalias work            # 删除别名
```

### 镜像源

```bash
nvm mirror                       # 查看当前
nvm mirror taobao                # 使用 npmmirror
nvm mirror official              # 切回 nodejs.org
nvm mirror https://example.com   # 自定义 URL
```

### 项目版本文件

在项目中创建 `.nvmrc` 或 `.node-version`：

```
20.11.0
```

然后执行 `nvm auto`（或在已 source `nvm.sh` 时 `cd` 进项目目录）会自动切换到对应版本。

### 使用指定版本运行

```bash
nvm run 20 app.js
nvm exec 20 npm install
```

### 迁移全局包

```bash
nvm use 20
nvm reinstall-packages 18         # 把 18 的全局包安装到 20
```

### 升级 npm

```bash
nvm install-latest-npm            # 升级当前版本的 npm
nvm install-latest-npm 20
```

### io.js

像管理 Node.js 一样安装和管理历史 io.js 版本：

```bash
nvm install iojs                  # 安装最新 io.js
nvm install iojs-3.3.1            # 指定版本
nvm install io.js-v2.5.0          # 带点前缀
nvm use iojs-v3.3.1               # 切换到 io.js
nvm ls                            # io.js 显示为 "io.js" 类型标签
```

### 缓存管理

下载的安装包会自动缓存。

```bash
nvm cache dir                     # 显示缓存目录
nvm cache list                    # 列出缓存文件及大小
nvm cache clear                   # 清理所有缓存文件
```

### 离线安装

不联网，仅从缓存安装：

```bash
nvm install 20 --offline          # 仅使用缓存包
```

如果版本不在缓存中，会报错并给出提示。

### 源码编译

从源码编译安装（适用于没有预编译包的 ARM 等平台）：

```bash
nvm install 20 -s                 # 从源码编译
nvm install 20 --source
```

需要 Python 3、C++ 编译器（GCC/Clang）和 `make`。

### 复合安装操作

安装后立即升级 npm 或迁移包：

```bash
nvm install 22 --latest-npm       # 安装后升级 npm
nvm install 22 --reinstall-packages-from=20  # 从 20 迁移全局包
nvm install 22 --latest-npm --reinstall-packages-from=20  # 同时执行
```

### 语言切换

在英文和中文之间切换显示语言：

```bash
nvm language                      # 显示当前语言
nvm language cn                   # 切换到中文
nvm lang en                       # 切换到英文（别名）
```

### 代理

管理代理设置（使用系统代理环境变量如 `HTTPS_PROXY`、`HTTP_PROXY`）：

```bash
nvm proxy                         # 显示代理状态
nvm proxy on                      # 启用代理（测试连通性）
nvm proxy off                     # 禁用代理
```

执行 `nvm proxy on` 时，会测试 baidu.com 和 google.com 的连通性并显示结果。

### Shell 补全

生成 Shell 补全脚本，提升 CLI 使用体验：

```bash
nvm completion bash        # 生成 bash 补全
nvm completion zsh        # 生成 zsh 补全
nvm completion fish       # 生成 fish 补全
nvm completion powershell  # 生成 PowerShell 补全
```

### Corepack

为指定版本启用或禁用 Corepack（支持 pnpm, yarn）：

```bash
nvm corepack status           # 显示 corepack 状态
nvm corepack enable           # 为当前版本启用
nvm corepack enable 20        # 为指定版本启用
nvm corepack disable          # 禁用当前版本
```

### 自动切换与 package.json

`nvm auto` 命令现在支持从 `package.json` 读取 Node.js 版本：

```json
{
  "engines": {
    "node": ">=18"
  }
}
```

如果存在 `package.json` 且包含 `engines.node`，`nvm auto` 会使用该版本。

同时支持递归查找 `.nvmrc` 和 `.node-version` 文件——从当前目录向上搜索到根目录。

## 配置

| 变量       | 默认值              | 说明              |
|----------|------------------|-----------------|
| `NVM_DIR` | `~/.nvm.rust`     | nvm 存放版本的目录      |

`NVM_DIR` 下的配置文件：

- `config.json` — 镜像源、默认版本
- `alias.json`  — 用户自定义别名
- `current`     — 当前激活的版本

## 命令参考

| 命令 | 说明 |
|------|------|
| `nvm install [ver] [--lts] [--latest]` | 安装 Node.js 版本 |
| `nvm install [ver] --offline` | 仅从缓存安装 |
| `nvm install [ver] -s / --source` | 从源码编译安装 |
| `nvm install [ver] --latest-npm` | 安装后升级 npm 到最新 |
| `nvm install [ver] --reinstall-packages-from=<ver>` | 安装后迁移全局包 |
| `nvm uninstall <ver>` | 卸载已安装的版本 |
| `nvm uninstall --lts` | 卸载最新 LTS 版本 |
| `nvm remove <ver>` | uninstall 的别名 |
| `nvm dir` | 显示 NVM 安装路径和 .nvm.rust 路径 |
| `nvm use <ver>` | 切换到指定版本 |
| `nvm use <ver> --install-if-missing` | 切换，未安装时自动安装 |
| `nvm list` / `ls` | 列出本地版本 |
| `nvm remote` / `ls-remote` [--lts] | 列出远程版本 |
| `nvm remote --filter <pattern>` | 过滤远程版本 |
| `nvm remote --sort asc\|desc` | 排序方式（默认：倒序） |
| `nvm current` | 显示当前版本 |
| `nvm which [ver]` | 显示可执行文件路径 |
| `nvm run <ver> [args...]` | 用指定版本运行脚本 |
| `nvm exec <ver> <cmd...>` | 用指定版本执行命令 |
| `nvm alias [name] [ver]` | 管理别名 |
| `nvm unalias <name>` | 删除别名 |
| `nvm mirror [url]` | 管理下载镜像源 |
| `nvm cache dir` | 显示缓存目录 |
| `nvm cache list` | 列出缓存文件 |
| `nvm cache clear` | 清理缓存文件 |
| `nvm language / lang [en\|cn]` | 显示或设置显示语言 |
| `nvm proxy [on\|off]` | 管理代理设置 |
| `nvm completion <shell>` | 生成 Shell 补全脚本 |
| `nvm corepack <enable\|disable\|status>` | Corepack 管理 |
| `nvm auto` | 根据 .nvmrc/.node-version/package.json 自动切换 |
| `nvm deactivate` | 恢复 PATH（撤销 `nvm use`） |
| `nvm unload` | 从 Shell 配置中移除 nvm |
| `nvm install-latest-npm [ver]` | 升级 npm 到最新 |
| `nvm reinstall-packages <ver>` | 迁移全局包 |
| `nvm version` | 显示当前 node/npm |
| `nvm version-remote` | 显示最近的远程版本 |

## 支持的平台

- Linux x64 / arm64
- macOS x64（Intel） / arm64（Apple Silicon）
- Windows x64（需先安装 7-Zip）

## 许可证

MIT
