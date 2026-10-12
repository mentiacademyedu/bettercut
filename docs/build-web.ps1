# Build bettercut for the web: the app compiled to WebAssembly, its
# JavaScript glue, and the page that hosts it, ready to serve as static files.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File docs\build-web.ps1
#   ... -Dev        a debug build: quicker to make, much slower to run
#   ... -Serve      then serve it at http://localhost:8080 to try
#
# Needs, once:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <the wasm-bindgen in Cargo.lock>
#
# Output goes to dist\web\ (gitignored). Any static host serves it; it needs
# no server code.
#
# ASCII only: this runs in consoles still on legacy code pages.

param([switch]$Dev, [switch]$Serve)

$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
Set-Location $Repo

$Build = if ($Dev) { 'debug' } else { 'release' }
$CargoArgs = @('build', '-p', 'bettercut-desktop', '--target', 'wasm32-unknown-unknown')
if (-not $Dev) { $CargoArgs += '--release' }
Write-Host "Building bettercut for the web ($Build)"
& cargo @CargoArgs
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# The CLI must be the same version as the library the app linked, or the
# glue it writes does not fit.
$Wanted = (Select-String -Path "$Repo\Cargo.lock" -Pattern '^name = "wasm-bindgen"$' -Context 0,1 |
    Select-Object -First 1).Context.PostContext[0] -replace '.*"(.+)".*', '$1'
$Have = (& wasm-bindgen --version) -replace '^wasm-bindgen\s+', ''
if ($Have -ne $Wanted) {
    throw "wasm-bindgen $Have is installed; this build needs $Wanted (cargo install wasm-bindgen-cli --version $Wanted)"
}

$Out = "$Repo\dist\web"
# Emptied rather than deleted, so a server already serving it keeps going.
New-Item -ItemType Directory -Force $Out | Out-Null
Get-ChildItem $Out | Remove-Item -Recurse -Force
& wasm-bindgen --target web --no-typescript --out-dir $Out `
    "$Repo\target\wasm32-unknown-unknown\$Build\bettercut.wasm"
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }

Copy-Item "$Repo\apps\desktop\web\index.html" $Out

$Size = (Get-Item "$Out\bettercut_bg.wasm").Length / 1MB
Write-Host ("Built {0}  (bettercut_bg.wasm {1:N1} MB)" -f $Out, $Size)

if ($Serve) {
    Write-Host "Serving at http://localhost:8080 (Ctrl+C to stop)"
    & python -m http.server 8080 --directory $Out
}
