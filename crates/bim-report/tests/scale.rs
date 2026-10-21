//! Scale check: a synthetic 50,000-element building through the whole pipeline.
//! Ignored by default (slow in debug). Run with timings:
//!     cargo test --release -p bim-report --test scale -- --ignored --nocapture
use bim_core::*;
use glam::vec3;
use std::time::Instant;

/// `levels` storeys of `nx` x `nz` 6 m bays; 10 elements and one space per bay:
/// slab, column, two walls, a door, a beam (every 7th sloped), a duct through the wall
/// (Mechanical vs Architectural clash), a riser through the slab (Plumbing vs Structural
/// clash), a fixture mesh and a stair. Element tasks chain slab -> column -> beam.
fn synthetic(levels: u32, nx: u32, nz: u32) -> Project {
    let mut p = Project::new("Synthetic tower", "IN-2026");
    for l in 0..levels {
        p.phases.push(Phase {
            id: PhaseId(l),
            name: format!("Level {} build", l + 1),
            duration_days: 10,
            depends_on: if l == 0 { vec![] } else { vec![PhaseId(l - 1)] },
        });
        p.levels.push(Level {
            id: LevelId(l),
            name: format!("Level {}", l + 1),
            elevation_m: l as f32 * 3.6,
        });
    }
    for (code, c) in [("A1010", 18_000), ("B1010", 95_000), ("B2010", 22_000)] {
        p.unit_costs.insert(code.into(), Cents(c));
    }
    p.unit_costs.insert("C1020".into(), Cents(85_000));
    for l in 0..levels {
        let y = l as f32 * 3.6;
        for i in 0..nx {
            for k in 0..nz {
                let (x, z) = (i as f32 * 6.0, k as f32 * 6.0);
                let ph = PhaseId(l);
                let add = |p: &mut Project, name: &str, kind, d, code: &str, dur, deps| {
                    let mut e = Element::new(format!("{name} L{l} {i}-{k}"), kind, d, ph);
                    e.cost_code = code.into();
                    e.level = Some(LevelId(l));
                    e.duration_days = dur;
                    e.depends_on = deps;
                    p.add(e)
                };
                use Discipline::*;
                let slab = add(
                    &mut p,
                    "Slab",
                    ElementKind::Slab {
                        origin: vec3(x, y, z),
                        size_x: 6.0,
                        size_z: 6.0,
                        thickness: 0.2,
                    },
                    Structural,
                    "A1010",
                    Some(2),
                    vec![],
                );
                let col = add(
                    &mut p,
                    "Column",
                    ElementKind::Column {
                        base: vec3(x, y + 0.2, z),
                        width: 0.4,
                        depth: 0.4,
                        height: 3.2,
                    },
                    Structural,
                    "B1010",
                    Some(1),
                    vec![slab],
                );
                let wall = add(
                    &mut p,
                    "Wall X",
                    ElementKind::Wall {
                        start: vec3(x + 0.3, y + 0.2, z),
                        end: vec3(x + 5.7, y + 0.2, z),
                        height: 3.0,
                        thickness: 0.2,
                    },
                    Architectural,
                    "B2010",
                    None,
                    vec![],
                );
                add(
                    &mut p,
                    "Wall Z",
                    ElementKind::Wall {
                        start: vec3(x, y + 0.2, z + 0.3),
                        end: vec3(x, y + 0.2, z + 5.7),
                        height: 3.0,
                        thickness: 0.2,
                    },
                    Architectural,
                    "B2010",
                    None,
                    vec![],
                );
                let door = add(
                    &mut p,
                    "Door",
                    ElementKind::Door {
                        host: wall,
                        offset: 2.0,
                        width: 0.9,
                        height: 2.1,
                        is_egress: true,
                    },
                    Architectural,
                    "C1020",
                    None,
                    vec![],
                );
                let rise = if (i + k) % 7 == 0 { 0.3 } else { 0.0 };
                add(
                    &mut p,
                    "Beam",
                    ElementKind::Beam {
                        start: vec3(x + 0.3, y + 3.0, z + 3.0),
                        end: vec3(x + 5.7, y + 3.0 + rise, z + 3.0),
                        width: 0.3,
                        depth: 0.4,
                    },
                    Structural,
                    "B1010",
                    Some(1),
                    vec![col],
                );
                add(
                    &mut p,
                    "Duct",
                    ElementKind::Beam {
                        start: vec3(x - 1.0, y + 2.6, z + 4.0),
                        end: vec3(x + 1.0, y + 2.6, z + 4.0),
                        width: 0.4,
                        depth: 0.3,
                    },
                    Mechanical,
                    "D3040",
                    None,
                    vec![],
                );
                add(
                    &mut p,
                    "Riser",
                    ElementKind::Column {
                        base: vec3(x + 4.0, y - 0.1, z + 4.0),
                        width: 0.1,
                        depth: 0.1,
                        height: 0.5,
                    },
                    Plumbing,
                    "D2010",
                    None,
                    vec![],
                );
                add(
                    &mut p,
                    "Fixture",
                    ElementKind::Mesh {
                        asset: "light.glb".into(),
                        origin: vec3(x + 3.0, y + 2.9, z + 3.0),
                    },
                    Electrical,
                    "D5020",
                    None,
                    vec![],
                );
                add(
                    &mut p,
                    "Stair",
                    ElementKind::Stair {
                        base: vec3(x + 1.0, y + 0.2, z + 1.0),
                        run_length: 3.0,
                        total_rise: 1.5,
                        width: 1.1,
                        riser_count: 9,
                    },
                    Architectural,
                    "C2010",
                    None,
                    vec![],
                );
                let mut s = Space::new(format!("Bay L{l} {i}-{k}"), Occupancy::Business, 36.0);
                s.egress_doors = vec![door];
                s.level = Some(LevelId(l));
                p.spaces.push(s);
            }
        }
    }
    p
}

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    println!("{label:<28} {:>9.1} ms", t.elapsed().as_secs_f64() * 1e3);
    r
}

#[test]
#[ignore = "scale benchmark; run in release with --ignored --nocapture"]
fn pipeline_50k_elements() {
    let rules = bim_rules::RuleSet::from_json(include_str!("../../../rules/IN-2026.json")).unwrap();
    let p = timed("generate", || synthetic(10, 25, 20));
    assert_eq!(p.elements.len(), 50_000);
    let total = Instant::now();
    timed("validate", || assert!(p.validate().is_empty()));
    let r = timed("build (full pipeline)", || {
        bim_report::build(&p, Some(&rules)).unwrap()
    });
    let json = timed("report -> JSON", || serde_json::to_string(&r).unwrap());
    let clashes = timed("clashes alone", || bim_geom::clashes(&p, 0.001));
    timed("rules alone", || rules.evaluate(&p).len());
    timed("egress exits alone", || {
        let ix = ProjectIndex::new(&p);
        p.spaces
            .iter()
            .map(|s| bim_rules::egress_exits(&ix, s))
            .sum::<usize>()
    });
    timed("takeoff alone", || bim_takeoff::takeoff(&p).len());
    timed("meshes alone", || {
        let ix = ProjectIndex::new(&p);
        p.elements
            .iter()
            .filter_map(|e| bim_geom::mesh_for(&ix, e).ok())
            .count()
    });
    let x = timed("xlsx", || bim_export::xlsx(&p));
    let b = timed("bcf", || bim_export::bcf(&p, &clashes));
    println!(
        "{:<28} {:>9.1} ms  ({} clashes, report {:.1} MB, xlsx {:.1} MB, bcf {:.1} MB)",
        "total after generate",
        total.elapsed().as_secs_f64() * 1e3,
        r.clashes.len(),
        json.len() as f64 / 1e6,
        x.len() as f64 / 1e6,
        b.len() as f64 / 1e6
    );
    // two designed clashes per bay (duct x wall, riser x slab)
    assert_eq!(r.clashes.len(), 2 * 5000);
    assert_eq!(r.elements.len(), 50_000);
    // per level: slab 2 days -> column 1 -> beam 1, inside the 10-day phase
    assert_eq!(r.total_days, r.phase_days);
}
