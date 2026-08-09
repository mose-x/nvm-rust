# Build Scripts

Local build scripts for nvm-rust. Pick the one for your platform:

| Platform | Script | How to run |
|----------|--------|------------|
| Linux | `scripts/build-linux.sh` | `./scripts/build-linux.sh` |
| macOS | `scripts/build-macos.sh` | `./scripts/build-macos.sh` |
| Windows | `scripts/build-windows.bat` | `scripts\build-windows.bat` |

## Usage (all platforms)

```bash
# Debug build
<script>

# Release build (optimized + LTO, static CRT on Windows)
<script> release

# Full verification (same as CI: fmt + clippy + test)
<script> check

# Quick check (fmt + clippy only, skip tests)
<script> check quick
```

## Prerequisites

### Linux
- [Rust](https://rustup.rs/) (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- For musl (Alpine): `sudo apt-get install musl-tools` (or `apk add musl-dev`)

### macOS
- [Rust](https://rustup.rs/)
- Xcode Command Line Tools: `xcode-select --install`

### Windows
- [Rust](https://rustup.rs/)
- [Visual Studio Build Tools](https://aka.ms/vs/17/release/vs_BuildTools.exe) — select "Desktop development with C++"

## Relationship to CI

These scripts are for **local development only**. CI uses:
- `.github/workflows/ci.yml` — fmt + clippy + cargo audit + test (3 OSes) + commit-lint
- `.github/workflows/release.yml` — 8 cross-compile targets + artifact staging + GitHub Release

The local scripts do NOT replace CI. They exist so developers can verify
locally before committing (AGENTS.md Step 3: Local Verification).
