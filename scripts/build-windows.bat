@echo off
REM ==========================================================================
REM nvm-rust Windows build script (native cmd, no Git Bash needed)
REM
REM Usage:
REM   scripts\build-windows.bat              - debug build
REM   scripts\build-windows.bat release       - release build (optimized + LTO + static CRT)
REM   scripts\build-windows.bat check         - fmt + clippy + test (full verification)
REM   scripts\build-windows.bat check quick    - fmt + clippy only
REM
REM Prerequisites:
REM   - Rust toolchain (rustup): https://rustup.rs/
REM   - Visual Studio Build Tools (C++ workload): https://aka.ms/vs/17/release/vs_BuildTools.exe
REM
REM This is a LOCAL build script. CI uses .github/workflows/ci.yml and
REM release.yml which have their own toolchain setup via dtolnay/rust-toolchain
REM — NOT replaced by this script.
REM ==========================================================================
setlocal enabledelayedexpansion

cd /d "%~dp0\.."

REM === Step 0: Detect and load MSVC Build Tools ===
where cl.exe >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [INFO] Detecting MSVC Build Tools...
    for %%V in ("2022" "2019") do (
        for %%E in (BuildTools Enterprise Professional Community) do (
            for %%A in ("C:\Program Files (x86)\Microsoft Visual Studio\%%~V\%%E\VC\Auxiliary\Build\vcvars64.bat" "C:\Program Files\Microsoft Visual Studio\%%~V\%%E\VC\Auxiliary\Build\vcvars64.bat") do (
                if exist "%%~A" (
                    echo [INFO] Found: %%~A
                    call "%%~A" >nul 2>&1
                    goto :msvc_found
                )
            )
        )
    )
    echo [ERROR] MSVC Build Tools not found.
    echo        Install: https://aka.ms/vs/17/release/vs_BuildTools.exe
    echo        Select: "Desktop development with C++"
    exit /b 1
)
:msvc_found

REM === Step 1: Detect Rust toolchain ===
where cargo >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [INFO] Cargo not on PATH, trying .svc Rust...
    for /d %%R in ("%USERPROFILE%\.svc\rust\*") do (
        if exist "%%~R\cargo\bin\cargo.exe" (
            echo [INFO] Found .svc Rust: %%~R
            set "PATH=%%~R\cargo\bin;%%~R\rustc\bin;%%~R\rustfmt-preview\bin;%%~R\clippy-preview\bin;%PATH%"
            goto :rust_found
        )
    )
    echo [ERROR] Rust not found. Install: https://rustup.rs/
    exit /b 1
)
:rust_found

REM === Step 2: Detect Node.js (needed for corepack test) ===
where node >nul 2>&1
if %ERRORLEVEL% neq 0 (
    for /d %%N in ("%USERPROFILE%\.svc\nodejs\*") do (
        if exist "%%~N\node.exe" (
            set "PATH=%%~N;%PATH%"
            goto :node_found
        )
    )
    echo [WARN] Node.js not found. Some tests may fail.
)
:node_found

REM === Step 3: Set NVM_DIR to temp ===
set "NVM_DIR=%TEMP%\nvm-test-env"
rd /s /q "%NVM_DIR%" 2>nul
mkdir "%NVM_DIR%"

REM === Step 4: Dispatch by mode ===
set "MODE=%1"
if "%MODE%"=="" set "MODE=build"

if /i "%MODE%"=="check" goto :check
if /i "%MODE%"=="release" goto :release
if /i "%MODE%"=="build" goto :build
goto :usage

:check
echo [1/3] Formatting...
cargo fmt
cargo fmt --check
if %ERRORLEVEL% neq 0 (
    echo [FAIL] cargo fmt --check failed. Run 'cargo fmt' to fix.
    exit /b 1
)
echo [OK] Formatting clean.

echo [2/3] Clippy...
cargo clippy --all-targets -- -D warnings
if %ERRORLEVEL% neq 0 (
    echo [FAIL] clippy found warnings. Fix before committing.
    exit /b 1
)
echo [OK] Clippy clean.

if /i "%2"=="quick" (
    echo [SKIP] Tests skipped ^(quick mode^).
    goto :check_done
)

echo [3/3] Tests...
cargo test --all
if %ERRORLEVEL% neq 0 (
    echo [FAIL] Tests failed. Fix before pushing.
    exit /b 1
)
echo [OK] All tests passed.

:check_done
echo.
echo ====================================
echo  All checks passed. Ready to commit.
echo ====================================
goto :eof

:release
echo [INFO] Building release ^(optimized + LTO + static CRT^)...
cargo build --release
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Build failed.
    exit /b 1
)
echo [OK] Release build: target\release\nvm.exe
goto :eof

:build
echo [INFO] Building debug...
cargo build
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Build failed.
    exit /b 1
)
echo [OK] Debug build: target\debug\nvm.exe
goto :eof

:usage
echo Usage: scripts\build-windows.bat [build^|release^|check [quick]]
echo.
echo   build      - Debug build (default)
echo   release    - Release build (optimized + LTO + static CRT)
echo   check      - fmt + clippy + test
echo   check quick - fmt + clippy only
exit /b 1
