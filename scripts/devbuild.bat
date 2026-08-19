@echo off
REM devbuild.bat — build debug binary and auto-copy to %%USERPROFILE%%\.nvm.rust\bin\
REM Usage: scripts\devbuild.bat
cargo build
if not exist "%USERPROFILE%\.nvm.rust\bin" mkdir "%USERPROFILE%\.nvm.rust\bin"
if exist target\debug\nvm.exe (
    copy /Y target\debug\nvm.exe "%USERPROFILE%\.nvm.rust\bin\nvm.exe" >nul
    echo ✓ Copied to %USERPROFILE%\.nvm.rust\bin\nvm.exe
) else (
    echo ⚠ Binary not found in target\debug\
    exit /b 1
)
