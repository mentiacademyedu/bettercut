# Preflight check for a fresh clone.
#
#   powershell -ExecutionPolicy Bypass -File docs\doctor.ps1
#
# Reports every prerequisite the build needs and says which one is missing.
# Paste the whole output into a bug report - it is designed to be the only
# thing anyone needs to diagnose "it will not build on my machine".

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$problems = @()

function Report($label, $ok, $detail) {
    $mark = if ($ok) { '  OK  ' } else { ' FAIL ' }
    Write-Host ("[{0}] {1,-22} {2}" -f $mark, $label, $detail)
}

Write-Host "bettercut doctor"
Write-Host "repo: $repo"
Write-Host ("-" * 72)

# --- toolchain ------------------------------------------------------------
$rustc = (Get-Command rustc -ErrorAction SilentlyContinue)
if ($rustc) {
    $v = (& rustc --version)
    # Edition 2024 needs 1.85; let-chains in the UI crate need 1.88.
    $num = [version](($v -split ' ')[1] -replace '-.*$', '')
    $ok = $num -ge [version]'1.88.0'
    Report 'rustc' $ok $v
    if (-not $ok) { $problems += "rustc $num is too old; run: rustup update stable" }
} else {
    Report 'rustc' $false 'not found on PATH'
    $problems += 'Rust is not installed. See https://rustup.rs'
}

# --- MSVC linker ----------------------------------------------------------
# Rust on Windows links with MSVC; without the Build Tools the failure is a
# confusing "linker `link.exe` not found" late in the build.
$host_triple = if ($rustc) { (& rustc -vV | Select-String '^host:').Line -replace 'host: ', '' } else { '' }
if ($host_triple -like '*msvc*') {
    $link = Get-Command link.exe -ErrorAction SilentlyContinue
    $vs = Test-Path 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
    $ok = ($null -ne $link) -or $vs
    Report 'MSVC build tools' $ok $(if ($ok) { 'present' } else { 'link.exe not found' })
    if (-not $ok) {
        $problems += 'Install "Desktop development with C++" from the Visual Studio Build Tools.'
    }
} else {
    Report 'host triple' $true $host_triple
}

# --- the committed binding ------------------------------------------------
$binding = Join-Path $repo 'vendor\ffmpeg-binding.rs'
if (Test-Path $binding) {
    $len = (Get-Item $binding).Length
    $ok = $len -gt 1000000
    Report 'ffmpeg-binding.rs' $ok "$([math]::Round($len/1MB,2)) MB"
    if (-not $ok) { $problems += "vendor/ffmpeg-binding.rs is only $len bytes; the clone is incomplete." }
} else {
    Report 'ffmpeg-binding.rs' $false 'missing'
    $problems += 'vendor/ffmpeg-binding.rs is missing. It is committed, so re-clone or run: git checkout -- vendor/ffmpeg-binding.rs'
}

# --- the FFmpeg SDK -------------------------------------------------------
# This is the step people skip. It is not in the repository.
$sdk = Join-Path $repo 'vendor\ffmpeg'
$lib = Join-Path $sdk 'lib\avcodec.lib'
$inc = Join-Path $sdk 'include\libavcodec\avcodec.h'
$bin = Join-Path $sdk 'bin\avcodec-62.dll'
foreach ($pair in @(@('ffmpeg lib', $lib), @('ffmpeg include', $inc), @('ffmpeg bin', $bin))) {
    $present = Test-Path $pair[1]
    Report $pair[0] $present $(if ($present) { $pair[1].Replace($repo, '.') } else { "missing: $($pair[1].Replace($repo,'.'))" })
    if (-not $present) {
        $problems += 'The FFmpeg SDK is not vendored. Run:  powershell -ExecutionPolicy Bypass -File docs\fetch-ffmpeg.ps1'
    }
}

# --- cargo config ---------------------------------------------------------
$cfg = Join-Path $repo '.cargo\config.toml'
Report 'cargo config' (Test-Path $cfg) $(if (Test-Path $cfg) { 'present' } else { 'missing .cargo/config.toml' })
if (-not (Test-Path $cfg)) { $problems += '.cargo/config.toml is missing; the build cannot find FFmpeg.' }

# A path with characters the FFmpeg build script mishandles is worth flagging.
if ($repo -match '[^\x20-\x7E]' -or $repo -match ' ') {
    Report 'repo path' $false "contains spaces or non-ASCII: $repo"
    $problems += 'Move the clone to a path with no spaces or non-ASCII characters.'
} else {
    Report 'repo path' $true 'plain ASCII, no spaces'
}

# --- summary --------------------------------------------------------------
Write-Host ("-" * 72)
if ($problems.Count -eq 0) {
    Write-Host "No problems found. Build with:"
    Write-Host "    cargo run -p bettercut-desktop"
    Write-Host ""
    Write-Host "If it still fails, capture the FULL error with:"
    Write-Host "    cargo build -p bettercut-desktop 2>&1 | Tee-Object build-log.txt"
} else {
    Write-Host "Problems found, in the order worth fixing:"
    $i = 1
    foreach ($p in ($problems | Select-Object -Unique)) {
        # ${i} braces are required: PowerShell parses "$i." as the start of a
        # property access and swallows the number, which printed a blank bullet.
        Write-Host "  ${i}. $p"
        $i++
    }
}
