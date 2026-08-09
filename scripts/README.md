# Build Scripts

Local build scripts for nvm-rust. Pick the one for your platform:

| Platform | Build Script | Package Script |
|----------|-------------|----------------|
| Linux | `scripts/build-linux.sh` | `scripts/package.sh` |
| macOS | `scripts/build-macos.sh` | `scripts/package.sh` |
| Windows | `scripts/build-windows.bat` | `scripts/package.sh` |

## Build Scripts Usage

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

## Package Script

After a release build, create a release-ready archive:

```bash
# Linux/macOS:
./scripts/build-linux.sh release   # or build-macos.sh
./scripts/package.sh target/release/nvm 2.1.0 linux x64
# → nvm-2.1.0-linux-x64.tar.gz

# Windows:
scripts\build-windows.bat release
bash scripts/package.sh target/release/nvm.exe 2.1.0 windows x64
# → nvm-2.1.0-windows-x64.zip
```

The archive contains: binary + README (en/cn) + LICENSE + install scripts + shell integration — the same "friendly-pack" that CI produces.

## CI vs Local

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

## Prerequisites

### Linux
- [Rust](https://rustup.rs/)
- For musl (Alpine): `sudo apt-get install musl-tools`

### macOS
- [Rust](https://rustup.rs/)
- Xcode Command Line Tools: `xcode-select --install`

### Windows
- [Rust](https://rustup.rs/)
- [Visual Studio Build Tools](https://aka.ms/vs/17/release/vs_BuildTools.exe) — select "Desktop development with C++"
- For packaging: Git Bash (provides `7z` and `tar`)
