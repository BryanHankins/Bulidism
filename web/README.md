# buildsim web (live wasm app)

    rustup target add wasm32-unknown-unknown
    cargo install wasm-pack            # or wasm-bindgen-cli at the exact version in crates/bim-wasm/Cargo.toml
    wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg --release
    cp rules/IN-2026.json web/rules/   # keep in sync
    cd web && python3 -m http.server 8080   # any static server; ES modules need http://, not file://

Files: index.html (shell + editor), app.js (state, editor, wasm calls), viewer.js/viewer.css (shared with
bim-cli --html), three.r128.min.js (vendored, MIT), pkg/ (built output, git-ignored).

Deploy: see the Deploy section of the root README (Dockerfile + render.yaml + railway.json).
Convenience scripts: ./web/build.sh [--serve] or .\web\build.ps1 [-Serve].
