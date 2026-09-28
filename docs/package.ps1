# Package bettercut for testers: a release build, the FFmpeg DLLs it links
# against, and the licences, as a portable zip - and as an installer too when
# Inno Setup is installed.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File docs\package.ps1
#   ... -SkipBuild     reuse target\release\bettercut.exe as it is
#
# Output goes to dist\ (gitignored): dist\bettercut-<version>\ (the staged
# files), dist\bettercut-<version>-windows.zip, and, with Inno Setup,
# dist\bettercut-<version>-setup.exe.
#
# ASCII only: this runs in consoles still on legacy code pages.

param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
Set-Location $Repo

# The version, from the workspace manifest: the same number the app shows.
$Version = (Select-String -Path "$Repo\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $Version) { throw "Could not read the version from Cargo.toml" }
Write-Host "Packaging bettercut $Version"

$FfmpegBin = "$Repo\vendor\ffmpeg\bin"
if (-not (Test-Path "$FfmpegBin\avcodec-62.dll")) {
    throw "FFmpeg is not in vendor\ffmpeg. Run setup.cmd first."
}

if (-not $SkipBuild) {
    Write-Host "Building the release (this takes a while)..."
    # The C runtime linked into the exe: otherwise it needs VCRUNTIME140.dll,
    # which a fresh Windows install may not have, and fails to open there
    # with a missing-DLL error. Set here only, so everyday builds are
    # untouched; the Windows CRT (ucrt) itself ships with Windows 10 and up.
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    # Two jobs: a release build with LTO is heavy, and this machine has run
    # out of memory on wider builds before.
    cargo build --release -p bettercut-desktop -j 2
    if ($LASTEXITCODE -ne 0) { throw "The release build failed" }
}
$Exe = "$Repo\target\release\bettercut.exe"
if (-not (Test-Path $Exe)) { throw "No release build at $Exe" }
# Refuse to ship an exe that still needs the Visual C++ runtime DLL.
$ExeText = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($Exe))
if ($ExeText.IndexOf('VCRUNTIME140.dll', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw "bettercut.exe still needs VCRUNTIME140.dll: build without -SkipBuild so the C runtime is linked in"
}

# Stage: the program, the seven FFmpeg libraries beside it (Windows loads
# DLLs from the program's own folder first), and the licences.
$Dist = "$Repo\dist"
$Stage = "$Dist\bettercut-$Version"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item $Exe $Stage
Get-ChildItem "$FfmpegBin\*.dll" | Copy-Item -Destination $Stage
$Dlls = (Get-ChildItem "$Stage\*.dll").Count
if ($Dlls -ne 7) { throw "Expected 7 FFmpeg DLLs, staged $Dlls" }

New-Item -ItemType Directory -Force "$Stage\licences" | Out-Null
Copy-Item "$Repo\vendor\ffmpeg\LICENSE.txt" "$Stage\licences\FFmpeg-LICENSE.txt"
Copy-Item "$Repo\LICENSE-MIT", "$Repo\LICENSE-APACHE" "$Stage\licences"
# The notices every Rust library bettercut is built from asks to be shipped
# with it. Generated fresh, so a new dependency is never left out; a build
# without them is not one to hand out.
$Notices = "$Stage\licences\THIRD-PARTY-NOTICES.txt"
Push-Location $Repo
cargo about generate -c docs\about.toml docs\about.hbs -o $Notices
$AboutExit = $LASTEXITCODE
Pop-Location
if ($AboutExit -ne 0 -or -not (Test-Path $Notices)) {
    throw "Could not generate the third-party notices: cargo install cargo-about --locked --features cli"
}
@"
bettercut $Version

bettercut is licensed MIT OR Apache-2.0.

It uses FFmpeg under the LGPL v3, linked dynamically: the avcodec, avdevice,
avfilter, avformat, avutil, swresample and swscale DLLs beside bettercut.exe
can be replaced with your own build. The build shipped here is BtbN's
FFmpeg-Builds, release tag autobuild-2026-08-09-13-03, asset
ffmpeg-n8.1.2-34-g9b6c8969e0-win64-lgpl-shared-8.1.zip; its source is at
https://github.com/FFmpeg/FFmpeg and the build scripts at
https://github.com/BtbN/FFmpeg-Builds. FFmpeg's licence is in
FFmpeg-LICENSE.txt.
"@ | Set-Content -Encoding ascii "$Stage\licences\NOTICE.txt"

@"
bettercut $Version - public beta

Run bettercut.exe. Keep the .dll files in the same folder: the program needs
them to read and write video.

This is a beta and will have bugs. Report them at
https://github.com/mentiacademyedu/bettercut/issues - in bettercut, Ctrl+K
then "Report a Bug on GitHub" fills in your version and system. If bettercut
crashes, it shows the saved report the next time it starts.

Licences: see the licences folder.
"@ | Set-Content -Encoding ascii "$Stage\README.txt"

# The portable zip: works everywhere, nothing to install.
$Zip = "$Dist\bettercut-$Version-windows.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path "$Stage\*" -DestinationPath $Zip
Write-Host "Portable zip: $Zip"

# The installer, when Inno Setup is there to make it.
$Iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($Iscc) {
    & $Iscc "/DAppVersion=$Version" "/DStageDir=$Stage" "/DOutputDir=$Dist" "$Repo\docs\installer.iss"
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
    Write-Host "Installer: $Dist\bettercut-$Version-setup.exe"
} else {
    Write-Host "No installer made: Inno Setup is not installed."
    Write-Host "  To make one: winget install JRSoftware.InnoSetup, then run this again."
}
