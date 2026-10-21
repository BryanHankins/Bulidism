# buildsim — browser 3D construction planning engine (Rust/WASM)

Prototype v0.7 (Phase 1 in progress). See COMPLETION_STATUS.docx for done vs open.

## Layout
- crates/bim-core      IFC-aligned model (incl. levels/storeys: name + finished-floor elevation; elements and spaces may sit on one), element tasks (`depends_on` element predecessors + `duration_days`; validate() rejects unknown/self/cyclic deps), ProjectIndex (hashed element/opening lookups for whole-project passes), JSON I/O, validate(), Project::sample()
- crates/bim-geom      oriented wall boxes with door cut-outs, beams following start→end incl. slope, stepped stairs, ramp wedges, clash detection (BVH broadphase, checked against brute force on 2,000 random elements + exact rotated-footprint overlap)
- crates/bim-schedule  phase DAG (Kahn), CPM critical path, element tasks (start = max(phase start + offset, predecessor finishes), finish = start + duration; own DAG, cycles rejected)
- crates/bim-rules     data-driven advisory rules (rules/IN-2026.json: 10 draft rules, citations only)
- crates/bim-takeoff   quantities, cents-based costing, phase roll-up, CSV
- crates/bim-report    ONE pipeline entry point (validate → schedule → rules → clashes → takeoff → meshes) → Report JSON (per element start_day/finish_day; total_days = project finish incl. element overruns, phase_days = CPM); shared by CLI and wasm. Scale check: `cargo test --release -p bim-report --test scale -- --ignored --nocapture` (50,000 elements)
- crates/bim-ifc       IFC import (IFC2x3/IFC4 STEP): own STEP reader; walls, slabs (sloped/roof slabs at measured thickness), columns, beams, doors (on their host wall), stairs, ramps, spaces (occupancy from OccupancyType, measured floor area); geometry from extrusions, B-reps, triangulated/polygonal face sets, mapped items; everything else counted in import notes. Tested against buildingSMART sample files (walls match the exporter's own quantities)
- crates/bim-export    no-dependency exports: store-only ZIP writer (CRC-32), XLSX takeoff workbook (sheets Takeoff + By phase), BCF 2.1 clash topics with viewpoints (components = element UUIDs in IfcGuid form). Deterministic bytes
- crates/bim-wasm      wasm-bindgen exports: run(project_json, rules_json), validate, import_ifc, sample_project, new_project, schema_version, takeoff_xlsx, clashes_bcf (bytes)
- crates/bim-cli       headless pipeline: validate -> schedule -> advisories -> clashes -> takeoff; --html writes a self-contained three.js 4D report; --xlsx out.xlsx, --bcf out.bcfzip
- assets/sample_office.json  exported sample project
- assets/sample_report.html  generated 4D report (open in any browser; verified in headless Chromium)
- web/                 live editor app: select/property panel, click-to-place walls, columns and doors (doors snap onto the wall you click), drag-to-move, undo/redo (Ctrl+Z/Y), Delete key, phase Gantt, CSV + standalone-HTML report export, JSON tab, autosave to browser storage (restored on reload), ft-in or metric display (model stays metric; web/units.js), printable code-check report (Print → save as PDF: findings, every rule checked with its verification status, spaces with occupant load, clashes, schedule/cost, sign-off block; web/print.js). levels (Edit tab: add/rename/delete, set elevation (moves the level's elements with it), active level for new elements; level picker per element and space; a Level filter in the viewer and exported report isolates one storey), unit-cost editor (Edit tab: every cost code with its usage, rate per SF/CY/LF or m²/m³/m, unpriced codes flagged, CSV import/export; web/costs.js). JS tests: `node --test web/*.test.mjs` — index.html + app.js + viewer.js/css (shared with the static report) + vendored three.js; needs web/pkg from wasm-pack (see web/README.md)

## Verified (Rust 1.75, Ubuntu 24.04)
    cargo test --workspace            # all tests pass (bim-wasm exports are exercised natively)
    cargo clippy --workspace --all-targets -- -D warnings   # clean
    cargo run -p bim-cli -- assets/sample_office.json --csv takeoff.csv --html report.html
    cargo run -p bim-cli -- model.ifc --sample-out model.json   # import IFC, print notes, run the pipeline

## Web build
    wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg --release   # then serve web/ (web/README.md)
Verified: wasm-pack 0.15 on Windows builds this clean and the app runs the real engine in the browser.
Rebuild the wasm after any Rust change.

Dependencies use caret ranges; Cargo.lock holds the tested versions (also builds on Rust 1.97). wasm-bindgen stays
pinned exactly (=0.2.92) because it must match the wasm-bindgen-cli version CI installs.

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
