# buildsim — browser 3D construction planning engine (Rust/WASM)

Prototype v0.11 (Phase 1 code complete; Phase 2 backend, monitoring and backups done in code, not yet deployed). See COMPLETION_STATUS.docx for done vs open.

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
- crates/bim-server    Axum + Postgres API: accounts, project storage, sharing (see Server below; not a dependency of anything)
- assets/sample_office.json  exported sample project
- assets/sample_report.html  generated 4D report (open in any browser; verified in headless Chromium)
- web/                 live editor app: select/property panel, click-to-place walls, columns and doors (doors snap onto the wall you click), drag-to-move, undo/redo (Ctrl+Z/Y), Delete key, phase Gantt, CSV + standalone-HTML report export, JSON tab, autosave to browser storage (restored on reload), ft-in or metric display (model stays metric; web/units.js), printable code-check report (Print → save as PDF: findings, every rule checked with its verification status, spaces with occupant load, clashes, schedule/cost, sign-off block; web/print.js). levels (Edit tab: add/rename/delete, set elevation (moves the level's elements with it), active level for new elements; level picker per element and space; a Level filter in the viewer and exported report isolates one storey), unit-cost editor (Edit tab: every cost code with its usage, rate per SF/CY/LF or m²/m³/m, unpriced codes flagged, CSV import/export; web/costs.js). Multi-select (Shift/Ctrl+click in the view or element list; Delete, drag-move and phase/discipline/level/cost-code edits apply to all, one undo step each), copy/paste/duplicate (Ctrl+C/V/D: new ids, +1 m in X and Z, doors follow a copied wall or stay on theirs; web/clipboard.js), draw a space by dragging a rectangle, rule-set selector (rules/index.json), code citations link to the ICC / Indiana code text (web/cites.js), per-element duration + predecessors with element bars in the Gantt (when the engine reports finish_day), XLSX/BCF export buttons when the wasm has takeoff_xlsx/clashes_bcf, optional Cloud tab (sign in, open/save/share projects; shown only when /api/health answers; web/cloud.js). Models over 2,000 elements draw as one merged mesh per discipline (50k elements: 7 draw calls instead of 50,001). JS tests: `node --test web/*.test.mjs` — index.html + app.js + viewer.js/css (shared with the static report) + vendored three.js; needs web/pkg from wasm-pack (see web/README.md)

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

Rule parameters in rules/IN-2026.json carry per-rule provenance (`verified`). rules/IBC-2024.json is the unamended model code: the same rules and parameters (IN-2026 adopts the 2024 IBC) without the Indiana-amendment caveat; rules/index.json lists the rule sets offered in the app. 10 rules; two parameters are checked against the ICC 2024 IBC text, the rest are secondary-source or unverified; Indiana amendments are not yet checked. Advisory only; never present as compliance.

Third-party: three.js r128 (MIT) is vendored in crates/bim-cli/vendor for the HTML report.

## Deploy
Docker image builds the wasm from source and serves it with nginx:

    docker build -f web/Dockerfile -t buildsim .   # from the repo root
    docker run -p 8080:80 buildsim

- **Render**: New → Blueprint, pointed at this repo (`render.yaml`): free web service running bim-server (app at /, API at /api) + free Postgres. Free services sleep when idle and free databases expire; upgrade both plans before relying on saved projects.
- **Railway**: New Project → Deploy from repo; `railway.json` selects the Dockerfile.
- **Own server**: `./web/build.sh` then copy `web/` behind nginx (`web/nginx.conf` has the wasm MIME type and gzip).

The web app works as a static site. Cloud projects are optional: serve the API at /api on the same origin (or set `window.BUILDSIM_API` to its root) and the Cloud tab appears; without it nothing changes.

The nginx image above is static only; for accounts and saved projects use the server below.

## Server (Phase 2)
crates/bim-server is an Axum + Postgres JSON API under `/api`: register/login/logout (argon2 passwords,
random bearer tokens stored as sha256, 30-day sessions), `/api/me`, project CRUD (project JSON in JSONB,
checked with `bim_core::Project::from_json`, 422 if invalid; `updated_at` on PUT gives 409 on conflict),
and per-project sharing (owner / editor / viewer; no access is a 404). Migrations are embedded and run at startup.

- Rate limits (in-process, per instance; 429 + Retry-After): login 20 per 5 min per IP and 10 failed attempts per
  15 min per account; register 10 per hour per IP; reset requests 10 per hour per IP and 3 per hour per address.
  Set `TRUST_PROXY=true` behind a proxy that appends `X-Forwarded-For` (render.yaml does).
- Password reset: `POST /api/auth/password-reset {email}` always answers 202 and, if the account exists, emails
  a single-use link `<PUBLIC_URL>/#reset=<token>` (60 minutes; only its sha256 is stored). `POST
  /api/auth/password-reset/confirm {token, password}` sets the password and signs the account out everywhere.
  Email goes over SMTP (`SMTP_URL` + `MAIL_FROM`, any provider); without them reset answers 503. The Cloud tab
  has "Forgot password?" and opens the link's set-new-password form.
- Email verification: with email configured, sign-up mails a 48-hour link (`#verify=<token>`, `POST /api/auth/verify`,
  `POST /api/auth/verify/resend`). Accounts work before verifying, but nobody can share a project with an unverified
  address (409), so registering someone else's email gains nothing; a completed password reset also verifies.
  Accounts that existed before migration 0003 count as verified. Without email nothing is enforced.
- Your data: `GET /api/me/export` downloads everything stored about the account (owned projects with data and shares,
  projects shared with you; never password hashes or tokens). `DELETE /api/me {password}` deletes the account and
  every project it owns. Both are in the Cloud tab under Account.
- Monitoring: `SENTRY_DSN` sends `error!` logs and panics to Sentry or a self-hosted GlitchTip (no personal data;
  release = deployed commit). `/api/health` also checks the database (503 when it is down).
  `.github/workflows/uptime.yml` pings it hourly once the `HEALTH_URL` repository variable is set.
- Backups: `.github/workflows/backup.yml` runs a daily `pg_dump` (Postgres 17 client; render.yaml pins the database
  to 17), checks it, encrypts it with AES-256 and keeps it as a 30-day artifact. It needs the `BACKUP_DATABASE_URL`
  and `BACKUP_PASSPHRASE` repository secrets. Restore with `BACKUP_PASSPHRASE=... scripts/restore-db.sh <file> <db-url>`.

    docker compose up --build            # Postgres 16 + server; app and API on http://localhost:8081
    DATABASE_URL=postgres://... cargo run -p bim-server   # env: BIND_ADDR (else 0.0.0.0:$PORT, else 0.0.0.0:8081), CORS_ORIGIN, WEB_DIR, TRUST_PROXY, PUBLIC_URL (else RENDER_EXTERNAL_URL), SMTP_URL, MAIL_FROM, SENTRY_DSN
    DATABASE_URL=postgres://... cargo test -p bim-server  # DB tests; skipped when DATABASE_URL is unset

The Docker images build with Rust 1.90 (`rust-version` in crates/bim-server/Cargo.toml); CI uses current stable.

## Local build

    ./web/build.sh --serve        # macOS/Linux
    .\web\build.ps1 -Serve       # Windows
