# nvm-rs PowerShell installer
# Usage: irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex

param(
    [string]$Version = ""
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

    # Add to user PATH
    $pathKey = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($pathKey -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$pathKey;$InstallDir", "User")
        $env:Path = "$env:Path;$InstallDir"
        Write-Success "Added to user PATH"
    } else {
        Write-Info "PATH already configured"
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

Main
