# Fetch the pinned FFmpeg build into vendor/ffmpeg (§88a).
#
#   pwsh docs/fetch-ffmpeg.ps1
#
# See docs/ffmpeg.md for the version, its hash, and why this particular build
# was chosen (§0.1's licensing constraints).

$ErrorActionPreference = 'Stop'

$Version = 'n8.1'
$Asset   = 'ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip'
$Sha256  = '96326847B2CDCE6A97C2703B1F487C3A5ED5E56C9D19180A080276B095BE95D3'
$Url     = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/$Asset"

$repo   = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $repo 'vendor'
$target = Join-Path $vendor 'ffmpeg'

if (Test-Path (Join-Path $target 'lib\avcodec.lib')) {
    Write-Host "FFmpeg already present at $target"
    exit 0
}

New-Item -ItemType Directory -Force -Path $vendor | Out-Null
$zip = Join-Path $vendor $Asset

Write-Host "Downloading FFmpeg $Version ..."
$ProgressPreference = 'SilentlyContinue'
Invoke-WebRequest -Uri $Url -OutFile $zip -TimeoutSec 900

$actual = (Get-FileHash $zip -Algorithm SHA256).Hash
if ($actual -ne $Sha256) {
    # Upstream `latest` is a rolling tag, so a mismatch is expected eventually.
    # It is still a stop: the licence checklist in docs/ffmpeg.md must be re-run
    # against the new build's configure line before the hash here is updated.
    Remove-Item $zip -Force
    throw "SHA-256 mismatch.`n  expected $Sha256`n  actual   $actual`nSee the 'Upgrading' section of docs/ffmpeg.md."
}

Write-Host "Extracting ..."
Expand-Archive -Path $zip -DestinationPath $vendor -Force

$extracted = Get-ChildItem $vendor -Directory | Where-Object { $_.Name -like 'ffmpeg-n*' } | Select-Object -First 1
if (-not $extracted) { throw "Could not find the extracted FFmpeg directory in $vendor" }

if (Test-Path $target) { Remove-Item $target -Recurse -Force }
Rename-Item -Path $extracted.FullName -NewName 'ffmpeg'
Remove-Item $zip -Force

Write-Host "FFmpeg installed at $target"
Write-Host "Verifying the licence posture (§0.1) ..."
& (Join-Path $target 'bin\ffmpeg.exe') -hide_banner -version 2>&1 |
    Select-String -Pattern 'configuration:' |
    ForEach-Object {
        foreach ($forbidden in @('--enable-gpl', '--enable-libx264', '--enable-libx265', '--enable-nonfree')) {
            if ($_ -match [regex]::Escape($forbidden)) {
                throw "This build contains $forbidden, which §0.1 forbids. Do not use it."
            }
        }
        Write-Host "  OK: no GPL or non-free components."
    }
