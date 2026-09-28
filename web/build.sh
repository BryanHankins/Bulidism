#!/usr/bin/env bash
# Local build: compile the wasm into web/pkg, then optionally serve.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg --release
cp rules/*.json web/rules/
echo "built web/pkg"
[ "${1:-}" = "--serve" ] && (cd web && python3 -m http.server 8080)
