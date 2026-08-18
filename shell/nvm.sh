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
# Fall back to OSTYPE if uname is unavailable.
if [ -n "${OSTYPE:-}" ]; then
    case "$OSTYPE" in
        msys*|cygwin*) NVM_RUST_ACTIVE_BIN="${NVM_RUST_ACTIVE}" ;;
        *) NVM_RUST_ACTIVE_BIN="${NVM_RUST_ACTIVE}/bin" ;;
    esac
else
    case "$(uname -s 2>/dev/null)" in
        MINGW*|MSYS*|CYGWIN*) NVM_RUST_ACTIVE_BIN="${NVM_RUST_ACTIVE}" ;;
        *) NVM_RUST_ACTIVE_BIN="${NVM_RUST_ACTIVE}/bin" ;;
    esac
fi

# Guard against double-init: the upgrade/refresh cases below re-source this
# file; without a guard, the re-source would call _nvm_init again.
NVM_RUST_SOURCED="${NVM_RUST_SOURCED:-0}"

# Check if nvm binary exists
_nvm_binary_exists() {
    [ -f "${NVM_RUST_BIN}/nvm" ] || [ -f "${NVM_RUST_BIN}/nvm.exe" ]
}

# Remove all occurrences of an entry from PATH (not just prefix).
_nvm_path_remove() {
    local entry="$1"
    local newpath=""
    local IFS_OLD="$IFS"
    IFS=":"
    for d in $PATH; do
        if [ "$d" != "$entry" ]; then
            newpath="${newpath:+$newpath:}$d"
        fi
    done
    IFS="$IFS_OLD"
    export PATH="$newpath"
}

# Add nvm shims + bin + active/bin to PATH if not already there.
# Prepend bin first, then shims, then active/bin, so the final order is:
#   active/bin:shims:bin:<rest>
_nvm_prepend_path() {
    case ":${PATH}:" in
        *":${NVM_RUST_BIN}:"*) ;;
        *) export PATH="${NVM_RUST_BIN}:${PATH}" ;;
    esac
    case ":${PATH}:" in
        *":${NVM_RUST_SHIMS}:"*) ;;
        *) export PATH="${NVM_RUST_SHIMS}:${PATH}" ;;
    esac
    # Full shim mode: active/bin resolves to current version's bin via symlink.
    if [ -e "${NVM_RUST_ACTIVE}" ] || [ -L "${NVM_RUST_ACTIVE}" ] || [ -d "${NVM_RUST_ACTIVE_BIN}" ]; then
        case ":${PATH}:" in
            *":${NVM_RUST_ACTIVE_BIN}:"*) ;;
            *) export PATH="${NVM_RUST_ACTIVE_BIN}:${PATH}" ;;
        esac
    fi
}

# Remove nvm entries from PATH.
_nvm_strip_path() {
    _nvm_path_remove "${NVM_RUST_ACTIVE_BIN}"
    _nvm_path_remove "${NVM_RUST_SHIMS}"
    _nvm_path_remove "${NVM_RUST_BIN}"
}

# Remove the auto-switch hook from PROMPT_COMMAND (bash) or precmd (zsh).
_nvm_remove_auto_switch_hook() {
    if [ -n "$BASH_VERSION" ]; then
        case "$PROMPT_COMMAND" in
            *_nvm_auto_switch*) PROMPT_COMMAND="${PROMPT_COMMAND//_nvm_auto_switch/}"
                PROMPT_COMMAND="${PROMPT_COMMAND//;;/;}"
                PROMPT_COMMAND="${PROMPT_COMMAND#;}"
                PROMPT_COMMAND="${PROMPT_COMMAND%;}"
                ;;
        esac
    elif [ -n "$ZSH_VERSION" ]; then
        autoload -Uz add-zsh-hook 2>/dev/null
        add-zsh-hook -d precmd _nvm_auto_switch 2>/dev/null
    fi
}

# Initialize — called on first source and after upgrade/refresh re-source.
_nvm_init() {
    if ! _nvm_binary_exists; then
        return 0
    fi

    _nvm_prepend_path

    # Auto-switch on cd (bash/zsh, Unix only)
    if [ -z "$NVM_RUST_AUTO_SWITCH_DONE" ]; then
        NVM_RUST_AUTO_SWITCH_DONE=1
        _nvm_auto_switch() {
            if [ -f ".nvmrc" ]; then
                # Read first token only — handles "18 # comment" correctly
                local ver
                ver=$(head -1 .nvmrc | awk '{print $1}')
                if [ -n "$ver" ]; then
                    local current
                    current=$("${NVM_RUST_BIN}/nvm" current 2>/dev/null | tr -d '[:space:]')
                    if [ "$current" != "$ver" ]; then
                        "${NVM_RUST_BIN}/nvm" use "$ver" >/dev/null 2>&1
                    fi
                fi
            fi
        }

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
                # Guard against duplicate fpath entries on re-source
                case " ${fpath[*]} " in
                    *" ${completions_dir} "*) ;;
                    *) fpath=( "${completions_dir}" $fpath ) ;;
                esac
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
            "${NVM_RUST_BIN}/nvm" "$@"
            _nvm_prepend_path
            ;;
        auto)
            "${NVM_RUST_BIN}/nvm" "$@"
            _nvm_prepend_path
            ;;
        deactivate)
            "${NVM_RUST_BIN}/nvm" deactivate 2>/dev/null
            _nvm_strip_path
            # Disarm auto-switch so cd into .nvmrc dir doesn't re-activate
            _nvm_remove_auto_switch_hook
            unset NVM_RUST_AUTO_SWITCH_DONE
            echo "nvm-rust deactivated (PATH updated)"
            ;;
        unload)
            "${NVM_RUST_BIN}/nvm" unload 2>/dev/null
            _nvm_strip_path
            _nvm_remove_auto_switch_hook
            unset -f nvm _nvm_auto_switch _nvm_prepend_path _nvm_strip_path
            unset -f _nvm_path_remove _nvm_remove_auto_switch_hook _nvm_init
            unset -f _nvm_binary_exists _nvm_load_completions
            unset NVM_RUST_SOURCED NVM_RUST_AUTO_SWITCH_DONE
            echo "nvm-rust unloaded from shell"
            ;;
        upgrade|update)
            "${NVM_RUST_BIN}/nvm" "$@"
            if [ -f "${NVM_RUST_SH}" ]; then
                unset NVM_RUST_SOURCED
                . "${NVM_RUST_SH}"
            fi
            ;;
        refresh)
            "${NVM_RUST_BIN}/nvm" "$@"
            if [ -f "${NVM_RUST_SH}" ]; then
                unset NVM_RUST_SOURCED
                . "${NVM_RUST_SH}"
            fi
            ;;
        shell)
            echo "NVM_RUST_DIR: $NVM_RUST_DIR"
            echo "NVM_RUST_BIN: $NVM_RUST_BIN"
            echo "NVM_RUST_ACTIVE: $NVM_RUST_ACTIVE"
            echo "NVM_RUST_ACTIVE_BIN: $NVM_RUST_ACTIVE_BIN"
            ;;
        *)
            "${NVM_RUST_BIN}/nvm" "$@"
            ;;
    esac
}

# Auto-initialize — only on first source (guarded against recursion)
if [ "$NVM_RUST_SOURCED" = "0" ]; then
    NVM_RUST_SOURCED=1
    _nvm_init
fi

# Load completions if shell is interactive
if [[ $- == *i* ]]; then
    _nvm_load_completions
fi
