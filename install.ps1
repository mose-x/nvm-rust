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
        "AMD64"   { return "x86_64" }
        "x64"     { return "x86_64" }
        "ARM64"   { return "aarch64" }
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

function Main {
    Write-Info "Installing nvm-rs..."

    $os = Get-OS
    $arch = Get-Arch
    Write-Info "Detected OS: $os, Architecture: $arch"

    if (-not $Version) {
        Write-Info "Checking latest version..."
        $Version = Get-LatestVersion
        Write-Success "Latest version: $Version"
    } else {
        Write-Info "Using specified version: $Version"
    }

    $target = "x86_64-pc-windows-msvc"
    if ($arch -eq "aarch64") {
        $target = "aarch64-pc-windows-msvc"
    }

    $archive = "nvm-${target}.zip"
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

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $exeSource = Join-Path $tmpDir "nvm.exe"
    $exeDest = Join-Path $InstallDir "nvm.exe"
    Move-Item -Path $exeSource -Destination $exeDest -Force
    Write-Success "Installed to $exeDest"

    # Download shell integration scripts
    $nvmDir = Join-Path $env:USERPROFILE ".nvm.rust"
    $shellDir = Join-Path $nvmDir "shell"

    Write-Info "Downloading shell integration scripts..."

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
        Write-Success "Shell integration scripts installed"
    } catch {
        Write-Warn "Could not download shell scripts, but nvm binary is installed"
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

    Write-Host ""
    Write-Success "nvm-rs $Version installed successfully!"
    Write-Host ""
    Write-Info "Quick start:"
    Write-Host "  nvm install 20          # Install Node.js 20"
    Write-Host "  nvm use 20             # Switch to Node.js 20"
    Write-Host "  nvm ls                 # List installed versions"
    Write-Host ""
    Write-Info "Restart PowerShell or run:"
    Write-Host "  Import-Module `"$shellDir\nvm.psm1`""
    Write-Host ""
    Write-Info "For China users, use mirror for faster downloads:"
    Write-Host "  `$env:GITHUB_MIRROR = 'ghproxy'"
    Write-Host "  irm https://raw.githubusercontent.com/mose-x/nvm-rust/main/install.ps1 | iex"

    Remove-Item $tmpDir -Recurse -Force
}

Main
