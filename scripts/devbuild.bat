@echo off
setlocal enabledelayedexpansion
REM devbuild.bat — build debug binary and auto-copy to system path or .nvm.rust\bin\
REM Usage: scripts\devbuild.bat
REM
REM Tries system path (%ProgramFiles%\nvm-rust\) first if admin (EDR-safe).
REM Falls back to user path if not admin.
cargo build
if not exist "%USERPROFILE%\.nvm.rust\bin" mkdir "%USERPROFILE%\.nvm.rust\bin"
if exist target\debug\nvm.exe (
    REM Try system path (EDR-safe) — requires admin
    net session >nul 2>&1
    if !ERRORLEVEL! equ 0 (
        if not exist "%ProgramFiles%\nvm-rust" mkdir "%ProgramFiles%\nvm-rust" 2>nul
        copy /Y target\debug\nvm.exe "%ProgramFiles%\nvm-rust\nvm.exe" >nul
        echo ✓ Copied to %ProgramFiles%\nvm-rust\nvm.exe (system path, EDR-safe)
    ) else (
        copy /Y target\debug\nvm.exe "%USERPROFILE%\.nvm.rust\bin\nvm.exe" >nul
        echo ✓ Copied to %USERPROFILE%\.nvm.rust\bin\nvm.exe (user path)
    )
) else (
    echo ⚠ Binary not found in target\debug\
    exit /b 1
)
