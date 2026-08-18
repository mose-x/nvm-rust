# nvm.sh — Shell integration for nvm-rs
# Source this file from your ~/.bashrc, ~/.zshrc, or ~/.profile
#
# Usage:
#   Linux/macOS (bash/zsh): source /path/to/nvm.sh
#   Fish: source /path/to/nvm.fish  (or use the Fish module)
#   PowerShell: Import-Module /path/to/nvm.psm1

NVM_RUST_DIR="${NVM_DIR:-$HOME/.nvm.rust}"
NVM_RUST_SH="${NVM_RUST_DIR}/bin/nvm.sh"
NVM_RUST_BIN="${NVM_RUST_DIR}/bin"
NVM_RUST_SHIMS="${NVM_RUST_DIR}/shims"
NVM_RUST_ACTIVE="${NVM_RUST_DIR}/active"

# On Windows (Git Bash/MSYS), Node.js executables are in the version root,
# not in a bin/ subdirectory. Use active directly instead of active/bin.
case "$(uname -s 2>/dev/null)" in
    MINGW*|MSYS*|CYGWIN*)
        NVM_RUST_ACTIVE_BIN="${NVM_RUST_ACTIVE}"
        ;;
    *)
        NVM_RUST_ACTIVE_BIN="${NVM_RUST_ACTIVE}/bin"
        ;;
esac

# Guard against infinite recursion: _nvm_init sources this file, which
# calls _nvm_init at the bottom. Without this guard, each source triggers
# another _nvm_init → another source → stack overflow.
NVM_RUST_SOURCED="${NVM_RUST_SOURCED:-0}"

# Check if nvm binary exists
_nvm_binary_exists() {
    [ -f "${NVM_RUST_BIN}/nvm" ] || [ -f "${NVM_RUST_BIN}/nvm.exe" ]
}

# Add nvm shims + bin + active/bin to PATH if not already there.
# Prepend bin first, then shims, then active/bin, so the final order is:
#   active/bin:shims:bin:<rest>
# active/bin resolves to the current version's bin via symlink,
# so global npm packages (tsc/eslint/codex) work immediately
# after `nvm use` — no `source` needed.
_nvm_prepend_path() {
    case ":${PATH}:" in
        *":${NVM_RUST_BIN}:"*) ;;
        *) export PATH="${NVM_RUST_BIN}:${PATH}" ;;
    esac
    case ":${PATH}:" in
        *":${NVM_RUST_SHIMS}:"*) ;;
        *) export PATH="${NVM_RUST_SHIMS}:${PATH}" ;;
    esac
    # Full shim mode: active/bin resolves to current version's bin via symlink
    # On Git Bash for Windows, [ -e ] may not detect junctions, so also
    # check [ -L ] (symlink/reparse point) and [ -d ] (dir through link).
    if [ -e "${NVM_RUST_ACTIVE}" ] || [ -L "${NVM_RUST_ACTIVE}" ] || [ -d "${NVM_RUST_ACTIVE_BIN}" ]; then
        case ":${PATH}:" in
            *":${NVM_RUST_ACTIVE_BIN}:"*) ;;
            *) export PATH="${NVM_RUST_ACTIVE_BIN}:${PATH}" ;;
        esac
    fi
}

# Initialize — called on first source and after upgrade/refresh.
# Does NOT re-source nvm.sh (that caused infinite recursion).
# The upgrade/refresh cases below handle re-sourcing explicitly.
_nvm_init() {
    if ! _nvm_binary_exists; then
        return 0
    fi

    _nvm_prepend_path

    # Auto-switch on cd (bash/zsh)
    if [ -z "$NVM_RUST_AUTO_SWITCH_DONE" ]; then
        export NVM_RUST_AUTO_SWITCH_DONE=1
        _nvm_auto_switch() {
            if [ -f ".nvmrc" ]; then
                local ver
                ver=$(cat .nvmrc | tr -d '[:space:]')
                if [ -n "$ver" ]; then
                    local current
                    current=$("${NVM_RUST_BIN}/nvm" current 2>/dev/null | tr -d '[:space:]')
                    if [ "$current" != "$ver" ]; then
                        "${NVM_RUST_BIN}/nvm" use "$ver" >/dev/null 2>&1
                    fi
                fi
            fi
        }

        # Hook into PROMPT_COMMAND (bash) or precmd (zsh)
        case "$OSTYPE" in
            darwin*|linux*)
                if [ -n "$BASH_VERSION" ]; then
                    PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND;}_nvm_auto_switch"
                elif [ -n "$ZSH_VERSION" ]; then
                    autoload -Uz add-zsh-hook
                    add-zsh-hook precmd _nvm_auto_switch
                fi
                ;;
        esac
    fi
}

# Load shell completions if available
_nvm_load_completions() {
    local completions_dir="${NVM_RUST_DIR}/completions"

    case "$OSTYPE" in
        darwin*|linux*)
            if [ -n "$BASH_VERSION" ] && [ -f "${completions_dir}/nvm.bash" ]; then
                . "${completions_dir}/nvm.bash"
            elif [ -n "$ZSH_VERSION" ] && [ -f "${completions_dir}/_nvm" ]; then
                fpath=( "${completions_dir}" $fpath )
                autoload -Uz _nvm
            fi
            ;;
    esac
}

# Wrapper function for nvm command
nvm() {
    if ! _nvm_binary_exists; then
        echo "nvm-rust not found. Install with:"
        echo "  curl -fsSL https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.sh | bash" >&2
        return 1
    fi

    local cmd="${1:-}"

    case "$cmd" in
        use)
            if [ $# -lt 2 ]; then
                echo "Usage: nvm use <version>" >&2
                return 1
            fi
            "${NVM_RUST_BIN}/nvm" "$@"
            _nvm_prepend_path
            ;;
        auto)
            "${NVM_RUST_BIN}/nvm" auto
            ;;
        deactivate)
            "${NVM_RUST_BIN}/nvm" deactivate 2>/dev/null
            export PATH="${PATH#${NVM_RUST_ACTIVE_BIN}:}"
            export PATH="${PATH#${NVM_RUST_SHIMS}:}"
            export PATH="${PATH#${NVM_RUST_BIN}:}"
            echo "nvm-rust deactivated (PATH updated)"
            ;;
        unload)
            "${NVM_RUST_BIN}/nvm" unload 2>/dev/null
            export PATH="${PATH#${NVM_RUST_ACTIVE_BIN}:}"
            export PATH="${PATH#${NVM_RUST_SHIMS}:}"
            export PATH="${PATH#${NVM_RUST_BIN}:}"
            unset -f nvm _nvm_auto_switch
            echo "nvm-rust unloaded from shell"
            ;;
        upgrade|update)
            "${NVM_RUST_BIN}/nvm" "$@"
            # Re-source updated nvm.sh so new shell function logic takes effect.
            # Use the guard to prevent recursion during re-source.
            if [ -f "${NVM_RUST_SH}" ]; then
                unset NVM_RUST_SOURCED
                . "${NVM_RUST_SH}"
            fi
            _nvm_prepend_path
            ;;
        refresh)
            "${NVM_RUST_BIN}/nvm" "$@"
            # Re-source updated nvm.sh so new shell function logic takes effect.
            if [ -f "${NVM_RUST_SH}" ]; then
                unset NVM_RUST_SOURCED
                . "${NVM_RUST_SH}"
            fi
            _nvm_prepend_path
            ;;
        shell)
            echo "NVM_RUST_DIR: $NVM_RUST_DIR"
            echo "NVM_RUST_BIN: $NVM_RUST_BIN"
            echo "NVM_RUST_ACTIVE: $NVM_RUST_ACTIVE"
            ;;
        *)
            "${NVM_RUST_BIN}/nvm" "$@"
            ;;
    esac
}

# Auto-initialize — only on first source (guarded against recursion)
if [ "$NVM_RUST_SOURCED" = "0" ]; then
    export NVM_RUST_SOURCED=1
    _nvm_init
fi

# Load completions if shell is interactive
if [[ $- == *i* ]] || [ -z "$PS1" ]; then
    _nvm_load_completions
fi
