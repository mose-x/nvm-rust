# nvm-rs PowerShell installer
# Usage: irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex

param(
    [string]$Version = "",
    [switch]$Uninstall,
    [switch]$Self
)

$ErrorActionPreference = "Stop"

$RepoOwner = "mose-x"
$RepoName = "nvm-rust"
$BinaryName = "nvm"
$InstallDir = Join-Path $env:USERPROFILE ".nvm.rust\bin"

# GitHub mirror for China users
$GithubPrefix = ""
if ($env:GITHUB_MIRROR) {
    if ($env:GITHUB_MIRROR -eq "ghproxy" -or $env:GITHUB_MIRROR -eq "gh-proxy") {
        $GithubPrefix = "https://ghproxy.com/"
    } else {
        $GithubPrefix = $env:GITHUB_MIRROR
    }
}

$GithubApi = "https://api.github.com/repos/$RepoOwner/$RepoName"
$GithubDownload = "https://github.com/$RepoOwner/$RepoName/releases/download"

if ($GithubPrefix) {
    $GithubDownload = "$GithubPrefix$GithubDownload"
}

function Write-Info($msg) {
    Write-Host "[INFO] " -ForegroundColor Cyan -NoNewline
    Write-Host $msg
}

function Write-Success($msg) {
    Write-Host "[OK] " -ForegroundColor Green -NoNewline
    Write-Host $msg
}

function Write-Warn($msg) {
    Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline
    Write-Host $msg
}

function Write-Error($msg) {
    Write-Host "[ERROR] " -ForegroundColor Red -NoNewline
    Write-Host $msg
}

function Get-OS {
    return "windows"
}

function Get-Arch {
    $arch = $env:PROCESSOR_ARCHITECTURE
    switch ($arch) {
        "AMD64"   { return "x64" }
        "x64"     { return "x64" }
        "ARM64"   { return "arm64" }
        default   {
            Write-Error "Unsupported architecture: $arch"
            exit 1
        }
    }
}

function Get-LatestVersion {
    $url = "$GithubPrefix$GithubApi/releases/latest"
    try {
        $response = Invoke-WebRequest -Uri $url -UseBasicParsing
        $json = $response.Content | ConvertFrom-Json
        return $json.tag_name
    } catch {
        Write-Error "Failed to get latest version: $_"
        exit 1
    }
}

function Download-File($url, $dest) {
    try {
        Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
        return $true
    } catch {
        Write-Error "Download failed: $_"
        return $false
    }
}

# Install-Completion — auto-install tab-completion for PowerShell.
# Runs `nvm completion powershell` so the binary (single source of truth,
# see src/completions.rs) generates the script, then dot-sources it in
# $PROFILE. Skip on NVM_NO_COMPLETION=1, missing binary, or older binary
# without the `completion` subcommand. Idempotent: checks profile for the
# completion file path before appending.
function Install-Completion {
    param([string]$NvmExePath, [string]$NvmDir)

    if ($env:NVM_NO_COMPLETION -eq "1") {
        Write-Info "NVM_NO_COMPLETION=1, skipping completion"
        return
    }

    if (-not (Test-Path $NvmExePath)) {
        Write-Warn "nvm.exe not found at $NvmExePath, skipping completion"
        return
    }

    $NvmExePath = (Resolve-Path $NvmExePath).Path
    & $NvmExePath completion powershell 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Info "nvm completion not available, skipping"
        return
    }

    $completionFile = Join-Path $NvmDir "completions\nvm.ps1"
    if (-not (Test-Path $completionFile)) {
        Write-Warn "Completion file was not generated at $completionFile"
        return
    }

    $dotSourceLine = ". `"$completionFile`""

    # $PROFILE can be empty on a locked-down fresh install.
    $profilePath = $PROFILE
    if (-not $profilePath) {
        $profilePath = Join-Path $env:USERPROFILE "Documents\PowerShell\Microsoft.PowerShell_profile.ps1"
    }
    $profileDir = Split-Path $profilePath -Parent
    if (-not (Test-Path $profileDir)) {
        New-Item -ItemType Directory -Path $profileDir -Force | Out-Null
    }

    if (Test-Path $profilePath) {
        $content = Get-Content $profilePath -Raw -ErrorAction SilentlyContinue
        if ($content -and $content.Contains($completionFile)) {
            Write-Info "PowerShell completion already configured"
            return
        }
        Add-Content -Path $profilePath -Value "`n# nvm-rs completion`n$dotSourceLine"
        Write-Success "Added PowerShell completion to $profilePath"
    } else {
        Set-Content -Path $profilePath -Value "# nvm-rs completion`n$dotSourceLine"
        Write-Success "Created profile with nvm completion at $profilePath"
    }
}

function Main {
    Write-Info "Installing nvm-rs..."

    $os = Get-OS
    $arch = Get-Arch
    Write-Info "Detected OS: $os, Architecture: $arch"

    $offline = $false
    $sourceDir = ""
    $tmpDir = $null

    # Offline detection: if the script sits next to a bundled nvm.exe
    # (extracted release zip), use it directly — no download. When piped
    # via `irm | iex`, $PSScriptRoot is empty and we fall through to online.
    if ($PSScriptRoot -and (Test-Path (Join-Path $PSScriptRoot "nvm.exe"))) {
        $offline = $true
        $sourceDir = $PSScriptRoot
        Write-Info "Found bundled binary at $PSScriptRoot (offline install)"
        if (-not $Version) {
            $bundledExe = Join-Path $PSScriptRoot "nvm.exe"
            $verOutput = & $bundledExe --version 2>$null
            if ($verOutput) {
                $Version = $verOutput.Trim()
                Write-Success "Detected version: $Version"
            } else {
                $Version = "unknown"
            }
        } else {
            Write-Info "Using specified version: $Version"
        }
    } else {
        # Online mode: fetch latest release zip from GitHub.
        if (-not $Version) {
            Write-Info "Checking latest version..."
            $Version = Get-LatestVersion
            Write-Success "Latest version: $Version"
        } else {
            Write-Info "Using specified version: $Version"
        }

        $versionNum = $Version -replace '^v', ''
        $archive = "nvm-$versionNum-windows-$arch.zip"
        $downloadUrl = "$GithubDownload/$Version/$archive"

        Write-Info "Downloading $archive..."
        Write-Info "URL: $downloadUrl"

        $tmpDir = Join-Path $env:TEMP "nvm-rs-install-$(Get-Random)"
        New-Item -ItemType Directory -Path $tmpDir | Out-Null

        $archivePath = Join-Path $tmpDir $archive

        if (-not (Download-File $downloadUrl $archivePath)) {
            Write-Error "Failed to download $archive"
            Remove-Item $tmpDir -Recurse -Force
            exit 1
        }
        Write-Success "Download complete"

        Write-Info "Extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tmpDir -Force
        $sourceDir = $tmpDir
    }

    # Install binary — Copy-Item (not Move) so offline mode doesn't damage
    # the extracted bundle.
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $exeSource = Join-Path $sourceDir "nvm.exe"
    $exeDest = Join-Path $InstallDir "nvm.exe"
    Copy-Item -Path $exeSource -Destination $exeDest -Force
    Write-Success "Installed to $exeDest"

    # Install shell integration scripts shipped inside the tarball.
    # The release archive includes `shell/nvm.psm1` so we copy it from the
    # local extraction — no extra network round-trip to raw.githubusercontent.com
    # (which would fail behind proxies, on offline machines, or for users who
    # deleted the repo's `main` branch tag). Fall back to a raw download only
    # if the bundled file is missing (e.g. an older / hand-rolled zip).
    $nvmDir = Join-Path $env:USERPROFILE ".nvm.rust"
    $shellDir = Join-Path $nvmDir "shell"

    if (-not (Test-Path $shellDir)) {
        New-Item -ItemType Directory -Path $shellDir -Force | Out-Null
    }

    Write-Info "Installing shell integration scripts..."
    $bundledShell = Join-Path $sourceDir "shell"
    if (Test-Path $bundledShell) {
        $psm1Source = Join-Path $bundledShell "nvm.psm1"
        $psm1Dest = Join-Path $shellDir "nvm.psm1"
        Copy-Item -Path $psm1Source -Destination $psm1Dest -Force
        Write-Success "Shell integration scripts installed (bundled)"
    } elseif ($offline) {
        Write-Warn "Bundle has no shell/ dir; skipping shell integration"
    } else {
        # Legacy fallback: zip without bundled shell/ dir.
        Write-Warn "Tarball does not contain shell/ dir, falling back to download"
        $rawBase = "https://raw.githubusercontent.com/$RepoOwner/$RepoName"
        if ($GithubPrefix) {
            $rawBase = "$GithubPrefix$rawBase"
        }
        $ps1Url = "$rawBase/$Version/shell/nvm.psm1"
        if (-not $Version) {
            $ps1Url = "$rawBase/main/shell/nvm.psm1"
        }
        try {
            $shellDest = Join-Path $shellDir "nvm.psm1"
            Invoke-WebRequest -Uri $ps1Url -OutFile $shellDest -UseBasicParsing -ErrorAction SilentlyContinue
            Write-Success "Shell integration scripts installed (downloaded)"
        } catch {
            Write-Warn "Could not download shell scripts, but nvm binary is installed"
        }
    }

    # Create shim scripts for node/npm/npx/corepack
    $shimsDir = Join-Path $nvmDir "shims"
    if (-not (Test-Path $shimsDir)) {
        New-Item -ItemType Directory -Path $shimsDir -Force | Out-Null
    }
    Write-Info "Creating shim scripts..."
    $shimScript = @"
@echo off
setlocal
set NVM_DIR=%USERPROFILE%\.nvm.rust
set CMD=%~n0
set CURRENT=
if exist "%NVM_DIR%\current" for /f "delims=" %%a in (%NVM_DIR%\current) do set CURRENT=%%a
if "%CURRENT%"=="none" (
    echo nvm: deactivated. Run 'nvm use ^<version^>' to reactivate.
    exit /b 1
)
call :resolve
if not defined BIN (
    "%NVM_DIR%\bin\nvm.exe" auto --silent 2>nul
    set CURRENT=
    if exist "%NVM_DIR%\current" for /f "delims=" %%a in (%NVM_DIR%\current) do set CURRENT=%%a
    call :resolve
)
if not defined BIN (
    echo nvm: %CMD% not found. Run 'nvm use ^<version^>' or 'nvm install ^<version^>'.
    exit /b 1
)
"%BIN%" %*
goto :eof

:resolve
set BIN=
if not "%CURRENT%"=="" (
    if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.exe" set BIN=%NVM_DIR%\%CURRENT%\bin\%CMD%.exe
    if not defined BIN if exist "%NVM_DIR%\%CURRENT%\bin\%CMD%.cmd" set BIN=%NVM_DIR%\%CURRENT%\bin\%CMD%.cmd
)
goto :eof
"@
    foreach ($cmd in @("node", "npm", "npx", "corepack")) {
        $shimFile = Join-Path $shimsDir "$cmd.cmd"
        Set-Content -Path $shimFile -Value $shimScript -Encoding ascii
    }
    Write-Success "Shim scripts created in $shimsDir"

    # Add to user PATH (shims dir + bin dir)
    $pathKey = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($pathKey -notlike "*$shimsDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$shimsDir;$pathKey;$InstallDir", "User")
        $env:Path = "$shimsDir;$env:Path;$InstallDir"
        Write-Success "Added to user PATH"
    } elseif ($pathKey -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$shimsDir;$pathKey;$InstallDir", "User")
        $env:Path = "$shimsDir;$env:Path;$InstallDir"
        Write-Success "Added to user PATH"
    } else {
        Write-Info "PATH already configured"
    }

    # Ensure PowerShell can load the profile (and thus our module).
    # Default Windows policy is Restricted, which silently blocks the
    # profile script this installer just wrote below, so `nvm` in a new
    # PowerShell window does nothing.
    $currentPolicy = Get-ExecutionPolicy
    if ($currentPolicy -eq 'Restricted') {
        Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser -Force
        Write-Success "PowerShell execution policy set to RemoteSigned"
    } else {
        Write-Info "PowerShell execution policy: $currentPolicy"
    }

    # Add to PowerShell profile
    $profilePath = $PROFILE
    $profileDir = Split-Path $profilePath -Parent

    if (-not (Test-Path $profileDir)) {
        New-Item -ItemType Directory -Path $profileDir -Force | Out-Null
    }

    if (Test-Path $profilePath) {
        $profileContent = Get-Content $profilePath -Raw -ErrorAction SilentlyContinue
        if ($profileContent -notlike "*nvm.psm1*") {
            Add-Content -Path $profilePath -Value "`n# nvm-rs`nImport-Module `"$shellDir\nvm.psm1`""
            Write-Success "Added PowerShell module to profile: $profilePath"
        } else {
            Write-Info "PowerShell module already in profile"
        }
    } else {
        Set-Content -Path $profilePath -Value "# nvm-rs`nImport-Module `"$shellDir\nvm.psm1`""
        Write-Success "Created PowerShell profile with nvm module"
    }

    # Auto-install tab-completion for PowerShell.
    $nvmExePath = Join-Path $InstallDir "nvm.exe"
    Install-Completion -NvmExePath $nvmExePath -NvmDir $nvmDir

    Write-Host ""
    Write-Success "nvm-rs $Version installed successfully!"
    Write-Host ""
    Write-Info "To activate now, run:"
    Write-Host "  Import-Module `"$shellDir\nvm.psm1`""
    Write-Host ""
    Write-Info "Or open a new PowerShell window to apply changes automatically."
    Write-Host ""
    Write-Info "Quick start:"
    Write-Host "  nvm install 20          # Install Node.js 20"
    Write-Host "  nvm use 20             # Switch to Node.js 20"
    Write-Host "  nvm ls                 # List installed versions"
    if (-not $offline) {
        Write-Host ""
        Write-Info "For China users, use mirror for faster downloads:"
        Write-Host "  `$env:GITHUB_MIRROR = 'ghproxy'"
        Write-Host "  irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex"
    }

    if ($tmpDir) {
        Remove-Item $tmpDir -Recurse -Force
    }
}

function Uninstall-Self {
    $nvmDir = Join-Path $env:USERPROFILE ".nvm.rust"

    Write-Warn "This will remove nvm itself (binary, nvm.sh, shims, shell config)."
    Write-Info "Node versions and config will be preserved at $nvmDir"
    $confirm = Read-Host "Continue? [y/N]"
    if ($confirm -ne "y" -and $confirm -ne "Y") {
        Write-Host "Cancelled."
        return
    }

    $binDir = Join-Path $nvmDir "bin"
    Remove-Item (Join-Path $binDir "nvm.exe") -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $binDir "nvm.sh") -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $nvmDir "shims") -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $nvmDir "current") -Force -ErrorAction SilentlyContinue
    Clean-ShellConfig

    Write-Success "nvm uninstalled. Node versions preserved at $nvmDir"
    Write-Info "Reinstall: irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex"
}

function Uninstall-All {
    $nvmDir = Join-Path $env:USERPROFILE ".nvm.rust"

    Write-Warn "This will remove nvm AND ALL installed Node versions."
    Write-Info "Everything in $nvmDir will be deleted."
    $confirm = Read-Host "Continue? [y/N]"
    if ($confirm -ne "y" -and $confirm -ne "Y") {
        Write-Host "Cancelled."
        return
    }

    Remove-Item $nvmDir -Recurse -Force -ErrorAction SilentlyContinue
    Clean-ShellConfig

    Write-Success "nvm and all Node versions uninstalled."
}

function Clean-ShellConfig {
    $profilePath = $PROFILE.CurrentUserCurrentHost
    if (-not (Test-Path $profilePath)) { return }
    Copy-Item $profilePath "$profilePath.bak" -Force -ErrorAction SilentlyContinue
    $content = Get-Content $profilePath
    $filtered = $content | Where-Object { $_ -notmatch "nvm.rust|nvm.sh|NVM_HOME" }
    $filtered | Set-Content $profilePath
    Write-Info "Shell config cleaned: $profilePath"
}

if ($Uninstall) {
    if ($Self) {
        Uninstall-Self
    } else {
        Uninstall-All
    }
    exit 0
}

Main
