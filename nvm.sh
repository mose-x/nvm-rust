#!/usr/bin/env bash
# nvm.sh - shell integration for nvm (Rust version)
# Usage: source /path/to/nvm.sh

# Locate the nvm binary
export NVM_DIR="${NVM_DIR:-$HOME/.nvm.rust}"
_nvm_rs_bin="${NVM_RS_BIN:-$(command -v nvm 2>/dev/null || echo "$NVM_DIR/bin/nvm")}"

# Get the current active version's bin path
_nvm_rs_current_bin() {
    if [ -f "$NVM_DIR/current" ]; then
        local v
        v=$(cat "$NVM_DIR/current" 2>/dev/null)
        if [ -n "$v" ] && [ "$v" != "system:"* ]; then
            echo "$NVM_DIR/$v/bin"
            return 0
        fi
    fi
    return 1
}

# Wrap the nvm command: make `nvm use` take effect immediately
nvm() {
    local cmd="$1"
    shift

    case "$cmd" in
        use)
            "$_nvm_rs_bin" use "$@"
            local rc=$?
            if [ $rc -eq 0 ]; then
                local bin
                bin=$(_nvm_rs_current_bin)
                if [ -n "$bin" ] && [ -d "$bin" ]; then
                    export PATH="$bin:$PATH"
                    unset NVM_RC
                    export NVM_RC=0
                fi
            fi
            return $rc
            ;;
        deactivate)
            "$_nvm_rs_bin" deactivate "$@"
            # Remove nvm-injected paths from PATH
            if [ -n "$NVM_RC" ]; then
                export PATH=$(echo "$PATH" | sed -E "s|:?$NVM_DIR/[^:]+/bin||g" | sed -E "s|^$NVM_DIR/[^:]+/bin:?||g")
                unset NVM_RC
            fi
            return $?
            ;;
        cd)
            # cd hook: auto-read .nvmrc / .node-version
            \cd "$@"
            local rc=$?
            if [ $rc -eq 0 ] && [ -f .nvmrc ] || [ -f .node-version ]; then
                nvm auto >/dev/null 2>&1 || true
            fi
            return $rc
            ;;
        *)
            "$_nvm_rs_bin" "$cmd" "$@"
            ;;
    esac
}

# Export internal variables
export NVM_DIR
