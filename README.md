# nvm-rs

[![中文文档](https://img.shields.io/badge/Documentation-Chinese-brightgreen?style=for-the-badge)](./README.ZH_CN.md)

A fast, feature-rich Node.js version manager written in Rust — a Rust-native reimagining of [nvm](https://github.com/nvm-sh/nvm). Delivered as a single static binary with sub-millisecond startup, bilingual UI, and 15 features nvm and fnm don't offer: GPG-verified downloads, resumable transfers, source builds, offline installs, and built-in yarn/pnpm/corepack integration. Built for modern workflows.

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
- **i18n** — `nvm language <en|cn>` (English / 中文, pluggable via `locales/*.toml`)
- **Proxy management** — `nvm proxy on/off` (leverages system proxy env vars)
- **Shell completions** — `nvm completion <bash|zsh|fish|powershell>`
- **Corepack support** — `nvm corepack <enable|disable|status>`
- **Recursive .nvmrc search** — auto-switch searches parent directories
- **package.json engines.node** — reads Node.js version requirement from package.json
- **Multi-shell support** — bash, zsh, Fish, PowerShell with auto-switch

## Comparison with fnm and nvm-sh

| Feature | nvm-rust (this project) | fnm | nvm-sh |
|------|------|------|------|
| **Implementation language** | Rust | Rust | Bash |
| **Startup overhead** | Low (native binary) | Low (native binary) | High (shell parsing) |
| **Single binary, no runtime deps** | ✅ | ✅ | ❌ (needs bash) |
| **Linux x86_64 (glibc)** | ✅ | ✅ | ✅ (shell) |
| **Linux aarch64 (glibc)** | ✅ | ✅ | ✅ (shell) |
| **Linux x86_64 (musl/Alpine static)** | ✅ | ✅ | ❌ (needs bash+coreutils) |
| **macOS x86_64 / aarch64** | ✅ | ✅ | ✅ |
| **Windows native** | ❌ | ✅ | ❌ (use nvm-windows) |
| **bash integration** | ✅ | ✅ | ✅ |
| **zsh integration** | ✅ | ✅ | ✅ |
| **fish integration** | ✅ | ✅ | ✅ |
| **PowerShell integration** | ✅ | ✅ | ❌ |
| **Install specific version `install 20`** | ✅ | ✅ | ✅ |
| **Install LTS `install --lts`** | ✅ | ✅ | ✅ |
| **Install latest `install --latest`** | ✅ | ✅ | ✅ |
| **Install LTS only if missing `--lts-newer`** | ✅ | ❌ | ❌ |
| **Fuzzy version `20` → latest 20.x** | ✅ | ✅ | ✅ |
| **`lts/*`, `lts/iron` codename resolution** | ✅ | ✅ | ✅ |
| **Compile from source `--source`** | ✅ | ❌ | ❌ |
| **Offline install `--offline` (cache only)** | ✅ | ❌ | ❌ |
| **io.js install** | ✅ | ❌ | ✅ |
| **Uninstall `uninstall` / `--lts` / `--latest`** | ✅ | ✅ | ✅ (version only) |
| **Progress bar download** | ✅ (indicatif) | ✅ | ✅ (curl/wget) |
| **Resumable download** | ✅ (HTTP Range + .part) | ❌ | ❌ |
| **Mirror switching `mirror`** | ✅ (taobao/official/custom) | ❌ (via env) | ✅ (`NVM_NODEJS_ORG_MIRROR`) |
| **Local cache reuse** | ✅ | ✅ | ❌ |
| **Cache dir/list/clear** | ✅ | ❌ | ❌ |
| **SHA-256 verification** | ✅ | ✅ | ✅ |
| **GPG signature verification `SHASUMS256.txt.sig`** | ✅ (auto key import) | ❌ | ✅ |
| **Skip verification `--no-gpg-verify`** | ✅ | — | ❌ |
| **Auto-skip verification when offline** | ✅ | — | ❌ |
| **Upgrade npm after install `--latest-npm`** | ✅ | ❌ | ❌ |
| **Install yarn after install `--latest-yarn`** | ✅ | ❌ | ❌ |
| **Install pnpm after install `--latest-pnpm`** | ✅ | ❌ | ❌ |
| **Standalone `install-latest-npm/yarn/pnpm`** | ✅ | ❌ | ❌ |
| **Global package migration `reinstall-packages`** | ✅ | ❌ | ✅ |
| **Cross-tool migration `migrate` (from nvm-sh/nvm-windows)** | ✅ | ❌ | ❌ |
| **corepack enable/disable/status** | ✅ | ❌ | ❌ |
| **`use <ver>` switch** | ✅ | ✅ | ✅ |
| **`use` reads `.nvmrc`/`.node-version`/`package.json#engines`** | ✅ | ✅ (`.nvmrc`/`.node-version`) | ✅ (`.nvmrc`) |
| **`use --install-if-missing`** | ✅ | ✅ | ✅ (auto via `nvm install`) |
| **`use --save` persist as default** | ✅ | ❌ | ✅ (`nvm alias default`) |
| **`use --use-on-cd` install cd hook** | ✅ | ❌ (via shell integration) | ❌ |
| **`run <ver> <script>`** | ✅ | ❌ | ✅ |
| **`exec <ver> <cmd>`** | ✅ | ❌ | ✅ |
| **`which [ver]`** | ✅ | ✅ | ✅ |
| **`current` active version** | ✅ | ✅ | ✅ |
| **`deactivate` restore PATH** | ✅ | ❌ | ✅ |
| **`unload` remove shell config** | ✅ | ❌ | ✅ |
| **`alias <name> <ver>`** | ✅ | ✅ (`default`/aliases) | ✅ |
| **`unalias`** | ✅ | ✅ | ✅ |
| **Built-in `node`/`stable`/`unstable`** | ✅ | ❌ | ✅ |
| **Built-in `lts`/`lts/<codename>`** | ✅ | ✅ | ✅ |
| **Built-in `system`/`default`** | ✅ | ✅ | ✅ |
| **`auto` switch via `.nvmrc`** | ✅ | ✅ (`--use-on-cd`) | ✅ (shell function) |
| **`auto --silent`** | ✅ | ❌ | ❌ |
| **`remote`/`ls-remote` list remote versions** | ✅ | ✅ | ✅ |
| **`--lts` LTS only** | ✅ | ✅ | ✅ |
| **`--lts-old` LTS ≤18 only** | ✅ | ❌ | ❌ |
| **`--filter <pattern>`** | ✅ | ❌ | ❌ |
| **`--sort desc/asc`** | ✅ | ❌ | ✅ (default desc) |
| **`--page <n>` pagination** | ✅ | ❌ | ❌ |
| **Pretty table output (border/align/CJK width)** | ✅ | ❌ | ❌ |
| **`proxy on/off/status` command** | ✅ | ❌ | ❌ |
| **Auto-detect system proxy** | ✅ | ❌ | ❌ |
| **Connectivity test (google/baidu)** | ✅ | ❌ | ❌ |
| **Custom HTTP client** | ✅ (reqwest) | ✅ | ❌ (curl/wget) |
| **Bilingual EN/CN `language en/cn`** | ✅ (pluggable, drop-in `locales/*.toml`) | ❌ | ❌ |
| **Colored output** | ✅ (colored) | ✅ | ❌ |
| **i18n help text** | ✅ | ❌ | ❌ |
| **Shell completion generation `completion`** | ✅ (bash/zsh/fish/powershell) | ✅ | ❌ |
| **Unified `dir` path display** | ✅ | ❌ | ✅ (`NVM_DIR`) |
| **GitHub Actions multi-platform build** | ✅ (6 targets) | ✅ | ❌ (manual/script) |
| **Release auto-generation + sha256sums** | ✅ | ✅ | ❌ |
| **Homebrew formula** | ✅ | ✅ | ❌ (tap install script) |
| **Concurrency safety** | ✅ | ✅ | ⚠️ (shell-dependent) |
| **Large version list render speed** | Fast | Fast | Slow |
| **nvm-sh ecosystem maturity** | — | — | ✅ (largest community, most docs) |

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

### Language / i18n

Switch the display language. nvm-rs ships with English and Chinese built in,
and supports adding new languages with **zero source code changes**.

```bash
nvm language                      # show current language
nvm language cn                   # switch to Chinese
nvm lang en                       # switch to English (alias)
nvm language zh                   # alias for Chinese
nvm lang en-us                    # alias for English
```

The setting persists in `~/.nvm.rust/config.json`.

#### Adding a new language (convention over configuration)

Drop a `xx.toml` file into the `locales/` directory and rebuild — that's it.
`build.rs` scans `locales/*.toml` at compile time and auto-registers the new
language. No edits to `i18n.rs`, `Cargo.toml`, or `build.rs` are needed.

1. **Create the locale file.** Copy `locales/en.toml` to `locales/xx.toml`
   (where `xx` is the language code, e.g. `jp`, `ca`, `de`) and translate
   every value. The key set must match `en.toml` exactly — `cargo test`
   enforces this (the `all_locales_have_same_keys_as_en` test fails on drift).

2. **Add a `[_meta]` table** at the end of the file (optional but recommended):

   ```toml
   [_meta]
   display_name = "日本語"            # shown in `nvm language` listing
   aliases      = ["ja", "jpn"]       # accepted by `nvm language <alias>`
   ```

   - `display_name` — human-readable name shown in `nvm language` output
   - `aliases` — alternative codes/names that resolve to this language

   If `_meta` is omitted, `display_name` defaults to the file stem and no
   aliases are registered.

3. **Rebuild** — `cargo build`. The new language is now available:

   ```
   $ nvm language
     ▶ Current language: English
     → Usage: nvm language <en|cn|jp>

   $ nvm language jp
     ✓ Language set to: 日本語
   ```

#### Fallback behavior

- Missing keys in a non-English locale fall back to the English value,
  then to the raw key name — so a partially translated locale never shows
  broken strings.
- `en.toml` is the mandatory baseline; `build.rs` fails the build if it's
  missing.
- Malformed TOML in one locale prints a warning at runtime and skips that
  language, rather than crashing the whole binary.

#### How it works

- `build.rs` parses each `locales/*.toml` at compile time, extracts the
  `[_meta]` table, and emits `OUT_DIR/locales_generated.rs` containing four
  `&'static` arrays: `LANG_CODES`, `LANG_DISPLAY_NAMES`, `LANG_ALIASES`,
  `LANG_STRINGS` (the latter embeds each TOML via `include_str!`).
- `src/i18n.rs` `include!`s that generated file and exposes `Lang`,
  `from_str`, `T`, `format_t`, and `available_lang_codes`.
- Locale files are parsed into a `HashMap<String, String>` on first use
  (lazy_static); the `[_meta]` table is filtered out so it never appears
  as a translatable string.

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
| `nvm language / lang [en\|cn\|...]` | Show or set display language (pluggable via `locales/*.toml`) |
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
- Windows x64

## License

MIT
