#!/bin/bash
# devbuild.sh — build debug binary and auto-copy to system path or ~/.nvm.rust/bin/
# Usage: ./scripts/devbuild.sh  or  bash scripts/devbuild.sh
#
# EDR probe-first: tries each system candidate by copying a probe binary
# and executing --version. No auto-sudo. Falls back to user path if all
# candidates fail the probe.
set -e
cargo build

USER_BIN="$HOME/.nvm.rust/bin/nvm"

mkdir -p "$HOME/.nvm.rust/bin"

if [ -f target/debug/nvm ]; then
    # EDR probe-first: try each system candidate
    SYSTEM_CANDIDATES="/usr/local/bin /opt/homebrew/bin"
    INSTALL_DONE=0
    for cand in $SYSTEM_CANDIDATES; do
        [ -d "$cand" ] || continue
        [ -w "$cand" ] || continue
        cp -f target/debug/nvm "$cand/.nvm_probe_$$"
        chmod +x "$cand/.nvm_probe_$$"
        if "$cand/.nvm_probe_$$" --version >/dev/null 2>&1; then
            cp -f target/debug/nvm "$cand/nvm"
            chmod +x "$cand/nvm"
            ln -sf "$cand/nvm" "$USER_BIN"
            echo "✓ Copied to $cand/nvm (system path, EDR-safe)"
            INSTALL_DONE=1
            break
        fi
        rm -f "$cand/.nvm_probe_$$"
    done
    if [ "$INSTALL_DONE" = "0" ]; then
        cp target/debug/nvm "$USER_BIN"
        chmod +x "$USER_BIN"
        echo "✓ Copied to ~/.nvm.rust/bin/nvm (user path — EDR may block)"
    fi
elif [ -f target/debug/nvm.exe ]; then
    cp target/debug/nvm.exe "$HOME/.nvm.rust/bin/nvm.exe"
    echo "✓ Copied to ~/.nvm.rust/bin/nvm.exe"
else
    echo "⚠ Binary not found in target/debug/"
    exit 1
fi
