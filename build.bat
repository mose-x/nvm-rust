@echo off
setlocal enabledelayedexpansion
REM build.bat — build nvm from source and auto-copy to .nvm.rust\bin\ or Program Files\nvm-rust\
REM
REM Use this if you downloaded the source and want to build locally.
REM If you just want to install (no Rust toolchain needed), use: install.ps1
REM
REM Usage: build.bat
REM
REM Simple admin check: if admin, copies to Program Files\nvm-rust\ (no probe).
REM Falls back to user path if not admin.
cargo build
if not exist "%USERPROFILE%\.nvm.rust\bin" mkdir "%USERPROFILE%\.nvm.rust\bin"
if exist target\debug\nvm.exe (
    REM Simple admin check: if admin, install to system path (no probe).
    net session >nul 2>&1
    if !ERRORLEVEL! equ 0 (
        if not exist "%ProgramFiles%\nvm-rust" mkdir "%ProgramFiles%\nvm-rust" 2>nul
        copy /Y target\debug\nvm.exe "%ProgramFiles%\nvm-rust\nvm.exe" >nul
        REM Also sync to user dir (Windows can't symlink, so dual-copy)
        copy /Y target\debug\nvm.exe "%USERPROFILE%\.nvm.rust\bin\nvm.exe" >nul
        echo ✓ Copied to %ProgramFiles%\nvm-rust\nvm.exe (system path) + %USERPROFILE%\.nvm.rust\bin\nvm.exe (sync)
    ) else (
        copy /Y target\debug\nvm.exe "%USERPROFILE%\.nvm.rust\bin\nvm.exe" >nul
        echo ✓ Copied to %USERPROFILE%\.nvm.rust\bin\nvm.exe (user path — run as admin for EDR-safe)
    )
) else (
    echo ⚠ Binary not found in target\debug\
    exit /b 1
)
