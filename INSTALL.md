# Installing nvm-rust from a release archive

You downloaded `nvm-<version>-<os>-<arch>.tar.gz` (macOS / Linux) or
`nvm-<version>-windows-<arch>.zip` (Windows). This guide covers installing from
that file — online or fully offline.

中文版本：[INSTALL.ZH_CN.md](INSTALL.ZH_CN.md)

Rather not handle archives at all? Use the one-liner installer
([README → Quick Start](README.md#quick-start)), or
`brew install mose-x/tap/nvm-rust` on macOS / Linux.

## What is inside the archive

The archive has **no top-level folder**: every file sits at its root, so extract
it into a directory of its own.

| File | Purpose |
| --- | --- |
| `nvm` (Unix) / `nvm.exe` (Windows) | The nvm-rust binary — the only file you strictly need. |
| `install.sh` | macOS / Linux installer. Run it from the extracted folder and it installs offline from the binary beside it. |
| `install.ps1` | Windows installer, same offline behaviour. |
| `shell/nvm.sh` | bash / zsh integration. |
| `shell/nvm.fish` | fish integration. |
| `shell/nvm.psm1` | PowerShell module. |
| `README.md`, `README.ZH_CN.md` | Full documentation: every command, mirrors, proxies, i18n, troubleshooting. |
| `LICENSE` | MIT. |

## macOS / Linux

### 1. Extract

```bash
mkdir nvm-install
tar -xzf nvm-<version>-<os>-<arch>.tar.gz -C nvm-install
cd nvm-install
```

Extracting without `-C` scatters the files into whatever directory you are in.

### 2. Run the bundled installer

```bash
./install.sh
source ~/.zshrc        # or ~/.bashrc; fish users restart the shell
nvm --version
```

`install.sh` notices the `nvm` binary next to it and installs offline — it
prints `Found bundled binary at ... (offline install)` and never contacts
GitHub. It then:

- copies `nvm` to `/usr/local/bin/nvm` when that directory is writable
  (EDR-friendly) and symlinks `~/.nvm.rust/bin/nvm` to it; otherwise it installs
  to `~/.nvm.rust/bin/nvm` and **prints** the `sudo cp ...` command rather than
  running sudo itself
- creates the `node`, `npm`, `npx` and `corepack` shims
- copies `shell/*` into place and appends the completion lines to `~/.zshrc`,
  `~/.bash_profile` / `~/.bashrc` or `~/.config/fish/config.fish`
- clears the macOS `com.apple.quarantine` attribute

Pick a different destination with `NVM_INSTALL_DIR`:

```bash
NVM_INSTALL_DIR="$HOME/.local/bin" ./install.sh
```

### macOS Gatekeeper

Release binaries are neither code-signed nor notarized, so a browser download
carries `com.apple.quarantine` and Gatekeeper blocks the first run.
`./install.sh` clears it for you. Installing by hand? Clear it once:

```bash
xattr -dr com.apple.quarantine ./nvm
```

### Fully manual, no installer

```bash
install -m 755 nvm /usr/local/bin/nvm   # or ~/.local/bin — keep it on PATH
nvm init                                # shims + completions + shell rc lines
exec "$SHELL" -l                        # or just open a new terminal
nvm --version
```

## Windows

### 1. Extract

Right-click the `.zip` → **Extract All**, or:

```powershell
Expand-Archive .\nvm-<version>-windows-x64.zip -DestinationPath .\nvm-install
cd .\nvm-install
```

### 2. Run the bundled installer

```powershell
Unblock-File .\install.ps1     # downloaded files are marked as remote
.\install.ps1
```

If PowerShell refuses to run scripts at all, allow them for this window only:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
.\install.ps1
```

`install.ps1` finds the `nvm.exe` beside it and installs offline (it prints
`Found bundled binary at ... (offline install)`). It has to be run **from the
extracted folder** as `.\install.ps1` — the `irm ... | iex` form has no script
directory, so it always downloads instead. It then:

- installs to `%USERPROFILE%\.nvm.rust\bin\nvm.exe` (override with
  `$env:NVM_DIR`)
- adds the shims directory and the bin directory to your user `PATH`
- copies `shell\nvm.psm1` and sets the execution policy to `RemoteSigned` for
  the current user

Open a **new** terminal afterwards, then run `nvm --version`.

### Fully manual, no installer

Copy `nvm.exe` into any directory on your `PATH`, run `nvm init`, then open a
new terminal.

## First steps

```bash
nvm --version
nvm doctor              # sanity check; --fix repairs what it can, --network tests reachability
nvm install --lts       # or a specific version: nvm install 22
nvm use 22
node --version
```

## Upgrading

```bash
nvm upgrade               # in place, verified against the release sha256sums.txt
nvm upgrade --check       # only report whether a newer version exists
nvm upgrade --rollback    # restore the previous binary from nvm.bak
```

Or extract the newer archive and run its `install.sh` / `install.ps1` again —
it overwrites the binary in place.

## Uninstalling

```bash
nvm uninstall --self        # remove nvm, keep Node versions and config
nvm uninstall --all         # remove nvm and every Node version
./install.sh --uninstall    # same as --all
./install.sh --uninstall --self
```

Windows: `.\install.ps1 -Uninstall` (add `-Self` to keep the Node versions).

## Installing Node.js afterwards

The archive install itself needs no network, but `nvm install <version>`
downloads Node.js. Behind a proxy or a firewall:

```bash
export https_proxy=http://127.0.0.1:7890     # Windows: $env:HTTPS_PROXY = "..."
nvm mirror taobao                             # npmmirror instead of nodejs.org
nvm install 22 --offline                      # cached tarball only, no network
```

See [README → Mirror source](README.md#mirror-source) and
[README → Proxy](README.md#proxy).

## When something is wrong

Run `nvm doctor` first, then
[README → Troubleshooting](README.md#troubleshooting).
