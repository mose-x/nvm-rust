#Requires -Version 5.1
# nvm.psm1 — PowerShell module for nvm-rs
# Usage: in your PowerShell profile:
#   Import-Module "$env:USERPROFILE\.nvm.rust\shell\nvm.psm1"

$ErrorActionPreference = 'Stop'

# Configuration
$NvmDir = "$env:USERPROFILE\.nvm.rust"
$NvmBin = Join-Path $NvmDir 'bin'
$NvmExe = Join-Path $NvmBin 'nvm.exe'

# Module-scoped PATH for unload
$script:OriginalPath = $env:Path

# Ensure nvm bin is in PATH
function Initialize-NvmPath {
    if ($env:Path -notlike "*$NvmBin*") {
        $env:Path = "$NvmBin;$env:Path"
    }
}

Initialize-NvmPath

# Resolve nvm alias or version
function Get-NvmVersion {
    param([string]$Version)

    if ([string]::IsNullOrEmpty($Version)) {
        return $null
    }

    # Check if it's a valid installed version
    $versionDir = Join-Path $NvmDir $Version
    if (Test-Path $versionDir) {
        return $Version
    }

    # Try to resolve alias
    try {
        $resolved = & $NvmExe alias $Version 2>$null
        if ($resolved) {
            return $resolved.Trim()
        }
    } catch {}

    return $Version
}

# Main nvm function
function nvm {
    [CmdletBinding()]
    param(
        [Parameter(Position = 0)]
        [ValidateSet('use', 'install', 'uninstall', 'ls', 'list', 'ls-remote', 'remote',
                     'current', 'which', 'run', 'exec', 'alias', 'unalias', 'auto',
                     'deactivate', 'unload', 'cache', 'language', 'proxy', 'completion',
                     'corepack', 'install-npm', 'reinstall-packages', 'version',
                     'version-remote', 'mirror', 'help')]
        [string]$Command,

        [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    if (-not $Command) {
        Show-NvmHelp
        return
    }

    switch ($Command) {
        'use' {
            if (-not $Arguments) {
                Write-Host "Usage: nvm use <version>" -ForegroundColor Yellow
                return
            }
            & $NvmExe use $Arguments
            Initialize-NvmPath
        }
        'install' {
            & $NvmExe install $Arguments
        }
        'uninstall' {
            if (-not $Arguments) {
                Write-Host "Usage: nvm uninstall <version>" -ForegroundColor Yellow
                return
            }
            & $NvmExe uninstall $Arguments
        }
        { $_ -in 'ls', 'list' } {
            & $NvmExe list $Arguments
        }
        { $_ -in 'ls-remote', 'remote' } {
            & $NvmExe remote $Arguments
        }
        'current' {
            & $NvmExe current
        }
        'which' {
            if (-not $Arguments) {
                Write-Host "Usage: nvm which <version>" -ForegroundColor Yellow
                return
            }
            & $NvmExe which $Arguments
        }
        'run' {
            & $NvmExe run $Arguments
        }
        'exec' {
            & $NvmExe exec $Arguments
        }
        'alias' {
            & $NvmExe alias $Arguments
        }
        'unalias' {
            if (-not $Arguments) {
                Write-Host "Usage: nvm unalias <name>" -ForegroundColor Yellow
                return
            }
            & $NvmExe unalias $Arguments
        }
        'auto' {
            & $NvmExe auto
        }
        'deactivate' {
            & $NvmExe deactivate
            Remove-NvmFromPath
        }
        'unload' {
            Remove-NvmFromPath
            Remove-Module -Name 'nvm' -Force -ErrorAction SilentlyContinue
        }
        'cache' {
            & $NvmExe cache $Arguments
        }
        'language' {
            & $NvmExe language $Arguments
        }
        'proxy' {
            & $NvmExe proxy $Arguments
        }
        'completion' {
            & $NvmExe completion $Arguments
        }
        'corepack' {
            & $NvmExe corepack $Arguments
        }
        'install-npm' {
            & $NvmExe install-npm $Arguments
        }
        'reinstall-packages' {
            & $NvmExe reinstall-packages $Arguments
        }
        'version' {
            & $NvmExe version
        }
        'version-remote' {
            & $NvmExe version-remote $Arguments
        }
        'mirror' {
            & $NvmExe mirror $Arguments
        }
        'help' {
            Show-NvmHelp
        }
        default {
            # Pass through to nvm binary
            & $NvmExe $Command $Arguments
        }
    }
}

function Show-NvmHelp {
    Write-Host @"
nvm-rs — Node.js version manager (PowerShell)

Usage: nvm <command> [options]

Commands:
  use <version>          Switch to a version
  install <version>       Install a version
  uninstall <version>    Uninstall a version
  ls, list               List installed versions
  ls-remote, remote      List remote versions
  current                Show current version
  which <version>        Show binary path
  run <ver> [args]       Run with version
  exec <ver> [cmd]       Execute with version
  alias [name] [ver]     Manage aliases
  unalias <name>         Remove alias
  auto                   Auto-switch via .nvmrc
  deactivate             Restore PATH
  unload                 Remove from session
  cache <sub>            Cache management (dir|list|clear)
  language [en|cn]       Set language
  proxy [on|off]         Proxy settings
  completion <shell>      Generate shell completions
  corepack <action>       Corepack support
  install-npm            Upgrade npm
  reinstall-packages     Migrate packages
  mirror <source>        Set download mirror
  version                Show nvm version
  help                   Show this help

Examples:
  nvm install 20
  nvm use 20
  nvm ls
  nvm ls --lts
  nvm auto
"@ -ForegroundColor Cyan
}

function Remove-NvmFromPath {
    $env:Path = $script:OriginalPath
}

# Auto-switch when changing directories
function Set-Location {
    param(
        [Parameter(Position = 0, ValueFromPipeline = $true, ValueFromRemainingArguments = $true)]
        [string]$Path,
        [switch]$PassThru
    )

    process {
        # Call original Set-Location
        Microsoft.PowerShell\Set-Location -Path $Path -PassThru:$PassThru

        # Check for .nvmrc
        $nvmrcPath = Join-Path (Get-Location) '.nvmrc'
        if (Test-Path $nvmrcPath) {
            $version = Get-Content $nvmrcPath -Raw | ForEach-Object { $_.Trim() }
            if ($version) {
                # Get current version
                $currentVersion = & $NvmExe current 2>$null
                if ($currentVersion -and $currentVersion -ne $version) {
                    Write-Host "Switching to Node.js $version via .nvmrc" -ForegroundColor Cyan
                    & $NvmExe use $version
                }
            }
        }
    }
}

# Export functions
Export-ModuleMember -Function nvm, Initialize-NvmPath, Remove-NvmFromPath
