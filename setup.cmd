@echo off
REM One-step setup for a fresh clone.
REM
REM   setup.cmd          fetch the FFmpeg SDK, then check the build prerequisites
REM   setup.cmd check    only run the checks
REM
REM A .cmd wrapper because getting a PowerShell invocation right is its own
REM obstacle: `pwsh` is not installed on stock Windows, execution policy blocks
REM unsigned scripts by default, and a relative script path fails from the wrong
REM directory. This resolves its own location, so it works from anywhere and can
REM also just be double-clicked.

setlocal
set "REPO=%~dp0"
set "PS=powershell -NoProfile -ExecutionPolicy Bypass -File"

if /I "%~1"=="check" goto :check

echo === Fetching the FFmpeg SDK ===========================================
%PS% "%REPO%docs\fetch-ffmpeg.ps1"
if errorlevel 1 (
    echo.
    echo Setup FAILED while fetching FFmpeg. The error above is the real cause.
    echo Nothing else will build until this succeeds.
    exit /b 1
)
echo.

:check
echo === Checking build prerequisites ======================================
%PS% "%REPO%docs\doctor.ps1"
echo.
echo Next:  cargo run -p bettercut-desktop
endlocal
