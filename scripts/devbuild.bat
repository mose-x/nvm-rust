@echo off
setlocal enabledelayedexpansion
REM devbuild.bat — build debug binary and auto-copy to system path or .nvm.rust\bin\
REM Usage: scripts\devbuild.bat
REM
REM EDR probe-first: if admin, probes Program Files\nvm-rust\ by copying
REM and executing --version. Falls back to user path if probe fails or
REM not admin.
cargo build
if not exist "%USERPROFILE%\.nvm.rust\bin" mkdir "%USERPROFILE%\.nvm.rust\bin"
if exist target\debug\nvm.exe (
    REM EDR probe-first: if admin, probe system path before installing.
    net session >nul 2>&1
    if !ERRORLEVEL! equ 0 (
        if not exist "%ProgramFiles%\nvm-rust" mkdir "%ProgramFiles%\nvm-rust" 2>nul
        copy /Y target\debug\nvm.exe "%ProgramFiles%\nvm-rust\.nvm_probe.exe" >nul
        "%ProgramFiles%\nvm-rust\.nvm_probe.exe" --version >nul 2>&1
        if !ERRORLEVEL! equ 0 (
            del "%ProgramFiles%\nvm-rust\.nvm_probe.exe" >nul 2>&1
            copy /Y target\debug\nvm.exe "%ProgramFiles%\nvm-rust\nvm.exe" >nul
            echo ✓ Copied to %ProgramFiles%\nvm-rust\nvm.exe (system path, EDR-safe)
        ) else (
            del "%ProgramFiles%\nvm-rust\.nvm_probe.exe" >nul 2>&1
            copy /Y target\debug\nvm.exe "%USERPROFILE%\.nvm.rust\bin\nvm.exe" >nul
            echo ✓ EDR blocked system path. Copied to %USERPROFILE%\.nvm.rust\bin\nvm.exe
        )
    ) else (
        copy /Y target\debug\nvm.exe "%USERPROFILE%\.nvm.rust\bin\nvm.exe" >nul
        echo ✓ Copied to %USERPROFILE%\.nvm.rust\bin\nvm.exe (user path)
    )
) else (
    echo ⚠ Binary not found in target\debug\
    exit /b 1
)
