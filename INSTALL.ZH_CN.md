# 从 release 压缩包安装 nvm-rust

你下载的是 `nvm-<version>-<os>-<arch>.tar.gz`（macOS / Linux）或
`nvm-<version>-windows-<arch>.zip`（Windows）。本文讲怎么从这个文件安装 ——
联网或完全离线都可以。

English: [INSTALL.md](INSTALL.md)

不想折腾压缩包？用一键安装脚本（[README → 快速开始](README.ZH_CN.md#快速开始)），
或者在 macOS / Linux 上 `brew install mose-x/tap/nvm-rust`。

## 包里有什么

压缩包**没有顶层目录**：所有文件都在根上，所以请解压到一个单独的目录里。

| 文件 | 用途 |
| --- | --- |
| `nvm`（Unix）/ `nvm.exe`（Windows） | nvm-rust 主程序 —— 严格来说只有它是必需的。 |
| `install.sh` | macOS / Linux 安装脚本。在解压出来的目录里执行，它会用旁边的二进制**离线**安装。 |
| `install.ps1` | Windows 安装脚本，同样支持离线。 |
| `shell/nvm.sh` | bash / zsh 集成。 |
| `shell/nvm.fish` | fish 集成。 |
| `shell/nvm.psm1` | PowerShell 模块。 |
| `README.md`、`README.ZH_CN.md` | 完整文档：所有命令、镜像、代理、多语言、排错。 |
| `LICENSE` | MIT 许可证。 |

## macOS / Linux

### 1. 解压

```bash
mkdir nvm-install
tar -xzf nvm-<version>-<os>-<arch>.tar.gz -C nvm-install
cd nvm-install
```

不加 `-C` 会把文件散落在你当前所在的目录里。

### 2. 执行包内的安装脚本

```bash
./install.sh
source ~/.zshrc        # 或 ~/.bashrc；fish 用户重开一个 shell
nvm --version
```

`install.sh` 会发现旁边的 `nvm` 二进制并走离线安装 —— 打印
`Found bundled binary at ... (offline install)`，全程不访问 GitHub。它接着会：

- 当 `/usr/local/bin` 可写时，把 `nvm` 拷到 `/usr/local/bin/nvm`（对 EDR 更友好），
  并把 `~/.nvm.rust/bin/nvm` 软链过去；不可写时安装到 `~/.nvm.rust/bin/nvm`，
  并**打印**出 `sudo cp ...` 命令让你自己决定，不会自动 sudo
- 创建 `node`、`npm`、`npx`、`corepack` 的 shim
- 把 `shell/*` 拷到位，并向 `~/.zshrc`、`~/.bash_profile` / `~/.bashrc` 或
  `~/.config/fish/config.fish` 追加补全配置
- 清除 macOS 的 `com.apple.quarantine` 属性

想换安装位置，用 `NVM_INSTALL_DIR`：

```bash
NVM_INSTALL_DIR="$HOME/.local/bin" ./install.sh
```

### macOS Gatekeeper

release 二进制既没有签名也没有公证，浏览器下载会带上
`com.apple.quarantine`，Gatekeeper 会拦住第一次运行。`./install.sh` 会替你清掉；
如果你是手工安装，执行一次：

```bash
xattr -dr com.apple.quarantine ./nvm
```

### 完全手工，不跑脚本

```bash
install -m 755 nvm /usr/local/bin/nvm   # 或 ~/.local/bin —— 确保它在 PATH 上
nvm init                                # 生成 shim、补全，并写入 shell rc
exec "$SHELL" -l                        # 或者直接开个新终端
nvm --version
```

## Windows

### 1. 解压

右键 `.zip` → **全部解压缩**，或者：

```powershell
Expand-Archive .\nvm-<version>-windows-x64.zip -DestinationPath .\nvm-install
cd .\nvm-install
```

### 2. 执行包内的安装脚本

```powershell
Unblock-File .\install.ps1     # 下载来的文件会被标记为“来自 Internet”
.\install.ps1
```

如果 PowerShell 完全不允许跑脚本，只对当前窗口放开：

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
.\install.ps1
```

`install.ps1` 会发现旁边的 `nvm.exe` 并走离线安装（打印
`Found bundled binary at ... (offline install)`）。注意必须**在解压出来的目录里**
以 `.\install.ps1` 的方式运行 —— `irm ... | iex` 那种写法拿不到脚本所在目录，
只会走联网下载。它接着会：

- 安装到 `%USERPROFILE%\.nvm.rust\bin\nvm.exe`（可用 `$env:NVM_DIR` 改）
- 把 shim 目录和 bin 目录加进用户 `PATH`
- 拷入 `shell\nvm.psm1`，并把当前用户的执行策略设为 `RemoteSigned`

之后请开一个**新**终端，再执行 `nvm --version`。

### 完全手工，不跑脚本

把 `nvm.exe` 拷到 `PATH` 上的任意目录，执行 `nvm init`，然后开个新终端。

## 装完之后

```bash
nvm --version
nvm doctor              # 自检；--fix 能修的顺手修掉，--network 测连通性
nvm install --lts       # 或指定版本：nvm install 22
nvm use 22
node --version
```

## 升级

```bash
nvm upgrade               # 原地升级，按 release 的 sha256sums.txt 校验
nvm upgrade --check       # 只看有没有新版本
nvm upgrade --rollback    # 用 nvm.bak 回滚到上一个版本
```

也可以解压新版压缩包，再跑一次它的 `install.sh` / `install.ps1`，会原地覆盖。

## 卸载

```bash
nvm uninstall --self        # 只删 nvm，保留已装的 Node 版本和配置
nvm uninstall --all         # nvm 和所有 Node 版本一起删
./install.sh --uninstall    # 等价于 --all
./install.sh --uninstall --self
```

Windows：`.\install.ps1 -Uninstall`（加 `-Self` 保留 Node 版本）。

## 之后安装 Node.js

装 nvm 本身不需要网络，但 `nvm install <version>` 要下载 Node.js。
在代理或防火墙环境下：

```bash
export https_proxy=http://127.0.0.1:7890     # Windows: $env:HTTPS_PROXY = "..."
nvm mirror taobao                             # 换成 npmmirror，不走 nodejs.org
nvm install 22 --offline                      # 只用本地缓存，不联网
```

详见 [README → 镜像源](README.ZH_CN.md#镜像源) 和
[README → 代理](README.ZH_CN.md#代理)。

## 出问题了

先跑 `nvm doctor`，再看
[README → 安装排错](README.ZH_CN.md#安装排错)。
