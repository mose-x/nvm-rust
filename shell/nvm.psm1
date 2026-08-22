#Requires -Version 5.1
# nvm.psm1 - PowerShell module for nvm (Rust implementation)
#
# This module is OPTIONAL. The nvm binary is on PATH permanently, so `nvm`
# works without importing anything. Import this module only if you want
# automatic version switching on `cd` (reads .nvmrc):
#
#   Import-Module "$env:USERPROFILE\.nvm.rust\shell\nvm.psm1"
#
# Keep this file ASCII-only: Windows PowerShell 5.1 mis-decodes non-ASCII
# characters depending on system locale, which used to produce mojibake.

$ErrorActionPreference = 'Stop'

# Configuration - respect NVM_DIR if set (consistent with nvm.sh and nvm.fish)
$NvmDir = if ($env:NVM_DIR) { $env:NVM_DIR } else { Join-Path $env:USERPROFILE '.nvm.rust' }
$NvmBin = Join-Path $NvmDir 'bin'
$NvmExe = Join-Path $NvmBin 'nvm.exe'
$NvmShims = Join-Path $NvmDir 'shims'

# Ensure nvm bin + shims are in PATH - use element comparison (not -notlike
# substring match which falsely matches "bin-old" etc.)
function Initialize-NvmPath {
    $pathElements = if ($env:Path) { $env:Path -split ';' } else { @() }
    if ($pathElements -notcontains $NvmBin) {
        $env:Path = "$NvmBin;$env:Path"
    }
    if ($pathElements -notcontains $NvmShims) {
        $env:Path = "$NvmShims;$env:Path"
    }
}

Initialize-NvmPath

# Pure pass-through to nvm.exe. Deliberately NO command whitelist and NO
# argument parsing here: a whitelist rejects every new subcommand the binary
# gains, and PowerShell binds leading-dash arguments (-v, --version, -h) as
# parameter names instead of positional values, so any param block breaks
# version/help flags. `& exe @args` forwards everything verbatim and
# $LASTEXITCODE propagates to the caller automatically.
function nvm {
    & $NvmExe @args
}

function Remove-NvmFromPath {
    # Remove nvm entries from current PATH (not a stale snapshot from module
    # load). Matches nvm.sh's _nvm_strip_path: removes bin + shims, keeps
    # everything else.
    $pathElements = $env:Path -split ';'
    $filtered = $pathElements | Where-Object {
        $_ -ne $NvmBin -and $_ -ne $NvmShims
    }
    $env:Path = ($filtered -join ';')
}

# Auto-switch when changing directories. Every parameter is forwarded to the
# real Set-Location via @PSBoundParameters, so -LiteralPath / -StackName /
# -PassThru / pipeline input keep working (the old wrapper only knew -Path
# and silently dropped the rest).
function Set-Location {
    [CmdletBinding()]
    param(
        [Parameter(Position = 0, ValueFromPipeline = $true)]
        [string]$Path,
        [string]$LiteralPath,
        [string]$StackName,
        [switch]$PassThru
    )

    process {
        Microsoft.PowerShell.Management\Set-Location @PSBoundParameters

        # Skip auto-switch when deactivated: the binary writes "none" to the
        # current file on `nvm deactivate`, so read the real state instead of
        # an in-memory flag (which reset on every module load anyway).
        $currentFile = Join-Path $NvmDir 'current'
        $current = ''
        if (Test-Path $currentFile) {
            $line = Get-Content $currentFile -TotalCount 1 -ErrorAction SilentlyContinue
            if ($line) { $current = $line.Trim() }
        }
        if (-not $current -or $current -eq 'none') { return }

        # Check for .nvmrc
        $nvmrcPath = Join-Path (Get-Location) '.nvmrc'
        if (Test-Path $nvmrcPath) {
            # Extract first token from first line - handles "20 # comment"
            $firstLine = (Get-Content $nvmrcPath -TotalCount 1 -ErrorAction SilentlyContinue)
            $version = if ($firstLine) { ($firstLine -split '\s+')[0] } else { '' }
            if ($version -and $version -ne $current) {
                Write-Host "Switching to Node.js $version via .nvmrc" -ForegroundColor Cyan
                & $NvmExe use $version
            }
        }
    }
}

# Export functions - Set-Location MUST be exported so the auto-switch-on-cd
# override actually replaces the global `cd`/`Set-Location` in the session.
Export-ModuleMember -Function nvm, Initialize-NvmPath, Remove-NvmFromPath, Set-Location
