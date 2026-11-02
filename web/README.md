# buildsim web (live wasm app)

    rustup target add wasm32-unknown-unknown
    cargo install wasm-pack            # or wasm-bindgen-cli at the exact version in crates/bim-wasm/Cargo.toml
    wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg --release
    mkdir -p web/rules && cp rules/*.json web/rules/   # rule sets + index.json; generated copy, rules/ is the source (git-ignored)
    cd web && python3 -m http.server 8080   # any static server; ES modules need http://, not file://

Files: index.html (shell + editor), app.js (state, editor, wasm calls), viewer.js/viewer.css (shared with
bim-cli --html), three.r128.min.js (vendored, MIT), pkg/ (built output, git-ignored). Pure, node-tested modules:
units.js, costs.js, levels.js, print.js, clipboard.js (multi-select delete/move, copy/paste, element dependencies,
drawn spaces), cites.js (citation -> code-text URL; also a classic script for viewer.js and the exported report),
cloud.js (cloud projects API client). Run them with `node --test web/*.test.mjs`.

Browser end-to-end tests (web/e2e/: the real app, wasm engine and viewer in headless Chromium; also run in CI after the wasm build): build web/pkg and copy rules to web/rules as above, then

    (cd web/e2e && npm install && npx playwright install chromium)
    node --test web/e2e/*.e2e.mjs

With Playwright installed elsewhere, point `PLAYWRIGHT_MODULE` at its index.js instead of `npm install`.

Keys: Shift/Ctrl+click toggles selection, Delete, Ctrl+C / Ctrl+V / Ctrl+D (duplicate), Ctrl+Z / Ctrl+Y, Esc.

Engine features are detected, not assumed: the XLSX/BCF buttons appear only when the wasm exports
takeoff_xlsx / clashes_bcf, and Gantt element bars only when the report has finish_day.

Cloud: the Cloud tab probes `GET <api>/health` at startup (api = `window.BUILDSIM_API`, default same-origin
`/api`) and stays hidden if it fails, so a static host shows nothing new (the browser console logs that one 404).
The sign-in token is kept in localStorage (`buildsim.cloud.token`).

Large models (over 2,000 elements) render as one merged mesh per discipline colour; the 4D day is a draw range,
level filters and selection rebuild the index buffer, and picking maps the hit back to the element.

Deploy: see the Deploy section of the root README (Dockerfile + render.yaml + railway.json).
Convenience scripts: ./web/build.sh [--serve] or .\web\build.ps1 [-Serve].
