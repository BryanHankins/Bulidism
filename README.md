# buildsim — browser 3D construction planning engine (Rust/WASM)

Prototype v0.7 (Phase 1 in progress). See COMPLETION_STATUS.docx for done vs open.

## Layout
- crates/bim-core      IFC-aligned model (incl. levels/storeys: name + finished-floor elevation; elements and spaces may sit on one), JSON I/O, validate(), Project::sample()
- crates/bim-geom      oriented wall/beam boxes with door cut-outs, stepped stairs, ramp wedges, clash detection (AABB sweep + exact rotated-footprint overlap)
- crates/bim-schedule  phase DAG (Kahn), CPM critical path, per-element 4D start days
- crates/bim-rules     data-driven advisory rules (rules/IN-2026.json: 10 draft rules, citations only)
- crates/bim-takeoff   quantities, cents-based costing, phase roll-up, CSV
- crates/bim-report    ONE pipeline entry point (validate → schedule → rules → clashes → takeoff → meshes) → Report JSON; shared by CLI and wasm
- crates/bim-ifc       IFC import (IFC2x3/IFC4 STEP): own STEP reader; walls, slabs (sloped/roof slabs at measured thickness), columns, beams, doors (on their host wall), stairs, ramps, spaces (occupancy from OccupancyType, measured floor area); geometry from extrusions, B-reps, triangulated/polygonal face sets, mapped items; everything else counted in import notes. Tested against buildingSMART sample files (walls match the exporter's own quantities)
- crates/bim-wasm      wasm-bindgen exports: run(project_json, rules_json), validate, import_ifc, sample_project, new_project, schema_version
- crates/bim-cli       headless pipeline: validate -> schedule -> advisories -> clashes -> takeoff; --html writes a self-contained three.js 4D report
- assets/sample_office.json  exported sample project
- assets/sample_report.html  generated 4D report (open in any browser; verified in headless Chromium)
- web/                 live editor app: select/property panel, click-to-place walls, columns and doors (doors snap onto the wall you click), drag-to-move, undo/redo (Ctrl+Z/Y), Delete key, phase Gantt, CSV + standalone-HTML report export, JSON tab, autosave to browser storage (restored on reload), ft-in or metric display (model stays metric; web/units.js), code analysis sheet ("code data block": occupant load by space/story/building, exits and egress door/stair capacity per story, plumbing fixtures, building facts set in Edit → Building; in the report pane and the printed report; story-level failures become findings; parameters in the rule set's code_summary, all marked with their verification), BCF 2.1 issue export (BCF button: one topic per error/warning finding and clash, element selected by IFC GlobalId, camera framed on it in the source IFC coordinates, snapshot; opens in Revit/Solibri/BIMcollab; web/bcf.js), printable code-check report (Print → save as PDF: findings with measured vs required, every rule checked with its verification status, spaces with occupant load, clashes, schedule/cost, sign-off block; web/print.js). levels (Edit tab: add/rename/delete, set elevation (moves the level's elements with it), active level for new elements; level picker per element and space; a Level filter in the viewer and exported report isolates one storey), unit-cost editor (Edit tab: every cost code with its usage, rate per SF/CY/LF or m²/m³/m, unpriced codes flagged, CSV import/export; web/costs.js). JS tests: `node --test web/*.test.mjs` — index.html + app.js + viewer.js/css (shared with the static report) + vendored three.js; needs web/pkg from wasm-pack (see web/README.md)

## Verified (Rust 1.75, Ubuntu 24.04)
    cargo test --workspace            # all tests pass (bim-wasm exports are exercised natively)
    cargo clippy --workspace --all-targets -- -D warnings   # clean
    cargo run -p bim-cli -- assets/sample_office.json --csv takeoff.csv --html report.html
    cargo run -p bim-cli -- model.ifc --sample-out model.json   # import IFC, print notes, run the pipeline

## Web build
    wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg --release   # then serve web/ (web/README.md)
Verified: wasm-pack 0.15 on Windows builds this clean and the app runs the real engine in the browser.
Rebuild the wasm after any Rust change.

Dependency versions are pinned exactly in Cargo.toml because the authoring toolchain was cargo 1.75
(newer transitive crates require edition 2024). On a current stable toolchain you may relax the pins.

Rule parameters in rules/IN-2026.json carry per-rule provenance (`verified`). 10 rules; two parameters are checked against the ICC 2024 IBC text, the rest are secondary-source or unverified; Indiana amendments are not yet checked. Advisory only; never present as compliance.

Third-party: three.js r128 (MIT) is vendored in crates/bim-cli/vendor for the HTML report.

## Deploy
Docker image builds the wasm from source and serves it with nginx:

    docker build -f web/Dockerfile -t buildsim .   # from the repo root
    docker run -p 8080:80 buildsim

- **Render**: New → Blueprint, pointed at this repo (`render.yaml`).
- **Railway**: New Project → Deploy from repo; `railway.json` selects the Dockerfile.
- **Own server**: `./web/build.sh` then copy `web/` behind nginx (`web/nginx.conf` has the wasm MIME type and gzip).

No backend in v0 — it's a static site.

## Local build

    ./web/build.sh --serve        # macOS/Linux
    .\web\build.ps1 -Serve       # Windows
