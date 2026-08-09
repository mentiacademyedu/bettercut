# Fetch the pinned FFmpeg build into vendor/ffmpeg (section 88a).
#
#   powershell -ExecutionPolicy Bypass -File docs\fetch-ffmpeg.ps1
#
# Works in Windows PowerShell 5.1, which is what ships with Windows - `pwsh`
# (PowerShell 7) is a separate install and is not assumed here.
#
# Without this, the build fails with:
#   error: could not find native static library `avcodec`
#   error: could not compile `rusty_ffmpeg` (lib) due to 1 previous error
#
# See docs/ffmpeg.md for the version, its hash, and why this particular build
# was chosen (section 0.1's licensing constraints).

$ErrorActionPreference = 'Stop'

# Pinned to a DATED release, not to `latest`.
#
# `latest` is a rolling tag: upstream rebuilds nightly and replaces the asset in
# place, so any hash pinned against it goes stale within a day and every clone
# then fails the integrity check. That is exactly what happened - the pin held
# for one afternoon. A dated `autobuild-*` tag is immutable, so this hash stays
# correct until someone deliberately changes it.
#
# Upgrading: pick a newer autobuild tag, re-run the licence checklist in
# docs/ffmpeg.md (section 0.1) against its configure line, then update all four
# values below together.
$Version = 'n8.1.2-34-g9b6c8969e0'
$Tag     = 'autobuild-2026-08-09-13-03'
$Asset   = 'ffmpeg-n8.1.2-34-g9b6c8969e0-win64-lgpl-shared-8.1.zip'
$Sha256  = '2936E5449886641B4279CA3FC554B678C8E9A2D20DD0C0A34FE7208B254A0905'
$Url     = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$Tag/$Asset"

$repo   = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $repo 'vendor'
$target = Join-Path $vendor 'ffmpeg'

if (Test-Path (Join-Path $target 'lib\avcodec.lib')) {
    Write-Host "FFmpeg already present at $target"
    exit 0
}

New-Item -ItemType Directory -Force -Path $vendor | Out-Null
$zip = Join-Path $vendor $Asset

Write-Host "Downloading FFmpeg $Version (~120 MB) ..."
# `SilentlyContinue` is not cosmetic: drawing the progress bar makes
# Invoke-WebRequest an order of magnitude slower on a download this size.
$ProgressPreference = 'SilentlyContinue'
# TLS 1.2 and basic parsing are both needed by Windows PowerShell 5.1 - it
# defaults to older TLS on some builds, and without -UseBasicParsing it wants
# Internet Explorer's engine, which fails outright if IE was never configured.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

try {
    Invoke-WebRequest -Uri $Url -OutFile $zip -TimeoutSec 900 -UseBasicParsing
} catch {
    # Report what actually went wrong. A bare stack trace here reads as "the
    # script is broken" when the real cause is usually a proxy, a firewall, or
    # antivirus quarantining the download.
    Write-Host ""
    Write-Host "DOWNLOAD FAILED" -ForegroundColor Red
    Write-Host "  url:   $Url"
    Write-Host "  error: $($_.Exception.Message)"
    Write-Host ""
    Write-Host "Common causes: no internet, a corporate proxy, or antivirus"
    Write-Host "blocking the file. You can also download the URL above by hand"
    Write-Host "and save it as:"
    Write-Host "  $zip"
    Write-Host "then re-run this script - it will verify and extract it."
    exit 1
}

if (-not (Test-Path $zip)) {
    Write-Host "Download reported success but produced no file at $zip" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $zip).Length
if ($size -lt 10MB) {
    # An HTML error page saved as a .zip is the classic proxy failure.
    Write-Host "Downloaded file is only $size bytes - that is not the SDK." -ForegroundColor Red
    Write-Host "A proxy or captive portal has probably returned an error page."
    Remove-Item $zip -Force
    exit 1
}

$actual = (Get-FileHash $zip -Algorithm SHA256).Hash
if ($actual -ne $Sha256) {
    # Upstream `latest` is a rolling tag, so a mismatch is expected eventually.
    # It is still a stop: the licence checklist in docs/ffmpeg.md must be re-run
    # against the new build's configure line before the hash here is updated.
    Write-Host ""
    Write-Host "SHA-256 MISMATCH - refusing to use this download." -ForegroundColor Red
    Write-Host "  expected $Sha256"
    Write-Host "  actual   $actual"
    Write-Host ""
    Write-Host "This pins an immutable dated tag ($Tag),"
    Write-Host "so upstream cannot have changed the file. A mismatch means the"
    Write-Host "download was corrupted or intercepted - retry first."
    Write-Host ""
    Write-Host "Do NOT edit the hash to make this pass. It is the only check"
    Write-Host "standing between the build and an FFmpeg with GPL or non-free"
    Write-Host "components in it, which section 0.1 forbids."
    Remove-Item $zip -Force
    exit 1
}

Write-Host "Extracting ..."
Expand-Archive -Path $zip -DestinationPath $vendor -Force

$extracted = Get-ChildItem $vendor -Directory | Where-Object { $_.Name -like 'ffmpeg-n*' } | Select-Object -First 1
if (-not $extracted) { throw "Could not find the extracted FFmpeg directory in $vendor" }

if (Test-Path $target) { Remove-Item $target -Recurse -Force }
Rename-Item -Path $extracted.FullName -NewName 'ffmpeg'
Remove-Item $zip -Force

Write-Host "FFmpeg installed at $target"
Write-Host "Verifying the licence posture (section 0.1) ..."
& (Join-Path $target 'bin\ffmpeg.exe') -hide_banner -version 2>&1 |
    Select-String -Pattern 'configuration:' |
    ForEach-Object {
        foreach ($forbidden in @('--enable-gpl', '--enable-libx264', '--enable-libx265', '--enable-nonfree')) {
            if ($_ -match [regex]::Escape($forbidden)) {
                throw "This build contains $forbidden, which section 0.1 forbids. Do not use it."
            }
        }
        Write-Host "  OK: no GPL or non-free components."
    }
