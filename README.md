# nvm-rs

[![中文文档](https://img.shields.io/badge/Documentation-Chinese-brightgreen?style=for-the-badge)](./README.ZH_CN.md)

A simple Node.js version manager written in Rust — a drop-in replacement for the [nvm](https://github.com/nvm-sh/nvm) shell script.

## Features

- Install, uninstall, switch between multiple Node.js versions
- Support for fuzzy version numbers (`20` → latest `v20.x.x`)
- Special aliases: `node`, `stable`, `unstable`, `lts`, `lts/<codename>`, `system`, `default`
- LTS code-name aliases: `lts/argon`, `lts/boron`, `lts/carbon`, `lts/dubnium`, `lts/erbium`, `lts/fermium`, `lts/gallium`, `lts/hydrogen`, `lts/iron`, `lts/jod`
- **io.js support** — install and manage historical io.js versions (`nvm install iojs`)
- `.nvmrc` and `.node-version` auto-switch support
- User-defined aliases
- Mirror source configuration (npmmirror, official, custom URL)
- SHA256 checksum verification on download
- Multi-architecture support: `x64`, `arm64` (Apple Silicon, ARM Linux)
- Cross-platform: Linux, macOS, Windows
- Resume from `NVM_DIR` environment variable
- Shell integration: `nvm use` takes effect in current shell (via `nvm.sh`)
- Migrate global npm packages between versions (`reinstall-packages`)
- Upgrade npm to latest for a specific version (`install-latest-npm`)
- **Cache management** — `nvm cache dir / list / clear`
- **Offline install** — `nvm install --offline` (install from cache only)
- **Source compile** — `nvm install -s` (compile from source tarball)
- **Compound install flags** — `--latest-npm` and `--reinstall-packages-from=<ver>`
- **LTS uninstall** — `nvm uninstall --lts`
- **i18n** — `nvm language <en|cn>` (English / 中文)
- **Proxy management** — `nvm proxy on/off` (leverages system proxy env vars)
- **Shell completions** — `nvm completion <bash|zsh|fish|powershell>`
- **Corepack support** — `nvm corepack <enable|disable|status>`
- **Recursive .nvmrc search** — auto-switch searches parent directories
- **package.json engines.node** — reads Node.js version requirement from package.json
- **Multi-shell support** — bash, zsh, Fish, PowerShell with auto-switch

## Installation

### One-liner install (macOS / Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash
```

**China mirror acceleration (script itself also goes through mirror):**

```bash
curl -fsSL https://ghproxy.com/https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | GITHUB_MIRROR=ghproxy bash
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex
```

**China mirror acceleration (script itself also goes through mirror):**

```powershell
$env:GITHUB_MIRROR = 'ghproxy'
irm https://ghproxy.com/https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex
```

### Homebrew (macOS)

```bash
brew tap mose-x/tap
brew install nvm-rust
```

### Build from source

```bash
git clone https://github.com/mose-x/nvm-rust.git
cd nvm-rust
cargo build --release
sudo cp target/release/nvm /usr/local/bin/
```

### Download manually

Download the latest binary from the [Releases page](https://github.com/mose-x/nvm-rust/releases) and add it to your PATH.

Supported platforms:
- Linux: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- macOS: `x86_64-apple-darwin`, `aarch64-apple-darwin`
- Windows: `x86_64-pc-windows-msvc`

## Shell Integration

After installation, source the appropriate script for your shell:

### Bash / Zsh

```bash
source ~/.nvm.rust/shell/nvm.sh
```

### Fish Shell

```fish
source ~/.nvm.rust/shell/nvm.fish
```

Or add to `~/.config/fish/config.fish`:

```fish
set -gx PATH ~/.nvm.rust/bin $PATH
source ~/.nvm.rust/shell/nvm.fish
```

### PowerShell

Add to your PowerShell profile (`$PROFILE`):

```powershell
Import-Module "$env:USERPROFILE\.nvm.rust\shell\nvm.psm1"
```

### Auto-switch

When an `.nvmrc` file is present, nvm-rs will automatically switch to the specified Node.js version when you enter that directory.

## Quick Start

```bash
nvm install 20        # Install Node.js 20
nvm use 20           # Switch to Node.js 20
nvm ls               # List installed versions
nvm ls --lts         # List LTS versions
nvm auto             # Auto-switch via .nvmrc
```

### Shell integration (optional)

```bash
source /path/to/nvm.sh
```

This makes `nvm use` change the current shell's `PATH` immediately, and adds a `cd` hook for auto-switching via `.nvmrc` / `.node-version`.

## Usage

### Install a Node.js version

```bash
nvm install 20                    # latest v20.x.x
nvm install 20.11.0               # exact version
nvm install v18.19.0              # with v prefix
nvm install --lts                 # latest LTS
nvm install --latest              # latest release
nvm install lts/iron              # specific LTS line
nvm install node                  # alias for "latest"
nvm install stable
nvm install unstable
```

### Use an installed version

```bash
nvm use 20
nvm use v18.19.0
nvm use lts/iron
nvm use lts                       # any installed LTS
nvm use system                    # use the system-wide node
```

If you have sourced `nvm.sh`, this takes effect in the current shell immediately.

### List versions

```bash
nvm list            # local
nvm ls              # alias
nvm list            # local
nvm remote          # remote (download index)
nvm remote --lts    # only LTS
nvm current         # show current version
```

### Uninstall

```bash
nvm uninstall 18
nvm remove 18        # alias for uninstall
```

### NVM directories

```bash
nvm dir              # Show NVM installation and .nvm.rust paths
```

### Aliases

```bash
nvm alias work 18           # create alias
nvm alias work              # show alias
nvm alias                   # list all aliases
nvm unalias work            # delete alias
```

### Mirror source

```bash
nvm mirror                       # show current
nvm mirror taobao                # use npmmirror
nvm mirror official              # back to nodejs.org
nvm mirror https://example.com   # custom URL
```

### Project version files

Create `.nvmrc` or `.node-version` in your project:

```
20.11.0
```

Then `nvm auto` (or `cd` into the directory when `nvm.sh` is sourced) will switch to that version automatically.

### Run with a specific version

```bash
nvm run 20 app.js
nvm exec 20 npm install
```

### Migrate global packages

```bash
nvm use 20
nvm reinstall-packages 18         # install all global packages from 18 into 20
```

### Upgrade npm

```bash
nvm install-latest-npm            # upgrade npm of the current version
nvm install-latest-npm 20
```

### io.js

Install and manage historical io.js versions, just like Node.js:

```bash
nvm install iojs                  # install latest io.js
nvm install iojs-3.3.1            # specific version
nvm install io.js-v2.5.0          # with dot prefix
nvm use iojs-v3.3.1               # switch to io.js
nvm ls                            # io.js shows with "io.js" type tag
```

### Cache management

Downloaded archives are automatically cached.

```bash
nvm cache dir                     # show cache directory
nvm cache list                    # list cached files with sizes
nvm cache clear                   # clear all cached files
```

### Offline install

Install from cache without network access:

```bash
nvm install 20 --offline          # use cached tarball only
```

If the version isn't cached, an error is shown with instructions.

### Source compile

Compile and install from source (useful for ARM / platforms without prebuilt binaries):

```bash
nvm install 20 -s                 # compile from source
nvm install 20 --source
```

Requires Python 3, a C++ compiler (GCC/Clang), and `make`.

### Compound install actions

Upgrade npm or migrate packages right after install:

```bash
nvm install 22 --latest-npm       # upgrade npm after install
nvm install 22 --reinstall-packages-from=20  # migrate global packages from 20
nvm install 22 --latest-npm --reinstall-packages-from=20  # both
```

### Language

Switch display language between English and Chinese:

```bash
nvm language                      # show current language
nvm language cn                   # switch to Chinese
nvm lang en                       # switch to English (alias)
```

### Proxy

Manage proxy settings (uses system proxy environment variables like `HTTPS_PROXY`, `HTTP_PROXY`):

```bash
nvm proxy                         # show proxy status
nvm proxy on                      # enable proxy (tests connectivity)
nvm proxy off                     # disable proxy
```

When `nvm proxy on` is run, it tests connectivity to both baidu.com and google.com and reports results.

### Shell Completions

Generate shell completions for better CLI experience:

```bash
nvm completion bash        # generate bash completions
nvm completion zsh        # generate zsh completions
nvm completion fish       # generate fish completions
nvm completion powershell  # generate PowerShell completions
```

### Corepack

Enable or disable Corepack for package managers (pnpm, yarn):

```bash
nvm corepack status           # show corepack status
nvm corepack enable           # enable for current version
nvm corepack enable 20        # enable for specific version
nvm corepack disable          # disable for current version
```

### Auto-switch with package.json

The `nvm auto` command now supports reading Node.js version from `package.json`:

```json
{
  "engines": {
    "node": ">=18"
  }
}
```

If `package.json` exists with `engines.node`, `nvm auto` will use that version.

Also supports recursive `.nvmrc` and `.node-version` search — searches from current directory up to root.

## Configuration

| Variable    | Default               | Description                  |
|-------------|-----------------------|------------------------------|
| `NVM_DIR`   | `~/.nvm.rust`         | Where nvm stores versions    |

Config files inside `NVM_DIR`:

- `config.json` — mirror, default version
- `alias.json`  — user-defined aliases
- `current`     — currently active version

## Command Reference

| Command | Description |
|---------|-------------|
| `nvm install [ver] [--lts] [--latest]` | Install a Node.js version |
| `nvm install [ver] --offline` | Install from cache only |
| `nvm install [ver] -s / --source` | Compile and install from source |
| `nvm install [ver] --latest-npm` | Upgrade npm to latest after install |
| `nvm install [ver] --reinstall-packages-from=<ver>` | Migrate global packages after install |
| `nvm uninstall <ver>` | Remove an installed version |
| `nvm uninstall --lts` | Uninstall the latest LTS version |
| `nvm remove <ver>` | Alias for uninstall |
| `nvm dir` | Show NVM installation and .nvm.rust paths |
| `nvm use <ver>` | Switch to a version |
| `nvm use <ver> --install-if-missing` | Switch, install if not installed |
| `nvm list` / `ls` | List local versions |
| `nvm remote` / `ls-remote` [--lts] | List remote versions |
| `nvm remote --filter <pattern>` | Filter remote versions |
| `nvm remote --sort asc\|desc` | Sort order (default: desc) |
| `nvm current` | Show current version |
| `nvm which [ver]` | Path to the binary |
| `nvm run <ver> [args...]` | Run a script with a specific version |
| `nvm exec <ver> <cmd...>` | Run a command with a specific version |
| `nvm alias [name] [ver]` | Manage aliases |
| `nvm unalias <name>` | Remove an alias |
| `nvm mirror [url]` | Manage download mirror |
| `nvm cache dir` | Show cache directory |
| `nvm cache list` | List cached files |
| `nvm cache clear` | Clear all cached files |
| `nvm language / lang [en\|cn]` | Show or set display language |
| `nvm proxy [on\|off]` | Manage proxy settings |
| `nvm completion <shell>` | Generate shell completions |
| `nvm corepack <enable\|disable\|status>` | Corepack management |
| `nvm auto` | Auto-switch via .nvmrc/.node-version/package.json |
| `nvm deactivate` | Restore PATH (revert `nvm use`) |
| `nvm unload` | Remove nvm from shell config |
| `nvm install-latest-npm [ver]` | Upgrade npm to latest |
| `nvm reinstall-packages <ver>` | Migrate global packages |
| `nvm version` | Show current node/npm |
| `nvm version-remote` | Show recent remote versions |

## Supported Platforms

- Linux x64 / arm64
- macOS x64 (Intel) / arm64 (Apple Silicon)
- Windows x64 (requires 7-Zip installed)

## License

MIT
