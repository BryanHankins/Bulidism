# Local build: compile the wasm into web/pkg, then serve.
#   .\web\build.ps1            build only
#   .\web\build.ps1 -Serve     build and serve on :8080
param([switch]$Serve)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg --release
Copy-Item rules/*.json web/rules/ -Force
Pop-Location
Write-Host "built web/pkg" -ForegroundColor Green
if ($Serve) { Push-Location "$root/web"; python -m http.server 8080; Pop-Location }
