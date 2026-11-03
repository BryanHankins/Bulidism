# Real-file corpus

Models exported by real authoring tools, imported and checked end to end on every CI run
(`crates/bim-report/tests/corpus.rs`), so changes to the importer, the egress analysis or the
rules show up against real files, not only hand-made ones.

    bash corpus/fetch.sh                                         # download, checksummed
    cargo test --release -p bim-report --test corpus -- --nocapture
    UPDATE_CORPUS=1 cargo test --release -p bim-report --test corpus   # rewrite expectations

`manifest.json` lists each model (pinned URL, SHA-256, license), optionally with a `setup`:
what a user would set after importing (occupancy, dwelling units), applied before the check. The files are downloaded into
`corpus/cache/` (git-ignored), not committed. `expected/<id>.json` holds the reviewed answers:
element counts, rooms with no route to an exit, the longest walk, and every error/warning
finding with a verdict:

| verdict | meaning |
|---|---|
| `valid` | correct for this model under the rule as written |
| `rule_limit` | correct under our rule, but the code has an exception or condition we do not model (the note says which) |
| `model_gap` | the model itself lacks something (e.g. no door into a room); correct relative to the model |
| `false_positive` | wrong: a defect to fix |

A finding that appears after a change is marked `unreviewed` and fails the test until someone
reviews it and sets its verdict; a finding that disappears or a changed count also fails, so
every change in results is looked at.

## Current results

| model | findings | valid | rule limit | model gap | false positive |
|---|---|---|---|---|---|
| Duplex Apartment (Revit 2011), as imported | 10 | 2 | 8 | 0 | 0 |
| Duplex, spaces set Residential and grouped into dwelling units | 8 | 8 | 0 | 0 | 0 |
| Duplex checked against the IRC 2021 (its proper code) | 2 | 2 | 0 | 0 | 0 |
| Medical-Dental Clinic (Revit) | 16 | 6 | 6 | 4 | 0 |

Before this corpus existed the Clinic produced 187 findings, most of them false: 124 rooms
"without a route to an exit" and dozens of door and stair findings. The fixes it drove:
exterior doors (`IsExternal`) as exits; wall openings with no door as passages; open-plan rooms
and corridors split into several spaces connected along shared edges with no wall; bulkhead
walls above head height not blocking the floor; stairs linking storeys; stair risers and treads
from the file's properties (Revit 2011 writes the flight attributes in feet) and along the
longer side of the footprint; double doors from the door style; curved walls at their layer
thickness, not their arc's depth; roofs and voids as unoccupied; door rules only for doors on a
route; the 36 in stair width for occupant loads under 50; the lowest occupied story as the
lowest story.

Not yet covered: ArchiCAD and other exporters (the openly licensed ArchiCAD sample,
Schependomlaan, is licensed for scientific and academic use only, so it is not used here).

## Attribution

- Duplex Apartment: BSI (2020) "Duplex Apartment Test Files," buildingSMART International,
  https://github.com/buildingsmart-community/Community-Sample-Test-Files — CC BY 4.0.
- Medical-Dental Clinic: BSI (2020) "Medical-Dental Test Files," buildingSMART International,
  https://github.com/buildingsmart-community/Community-Sample-Test-Files — CC BY 4.0.

The files are used unchanged.
