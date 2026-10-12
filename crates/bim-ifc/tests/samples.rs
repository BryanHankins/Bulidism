//! Real exporter output: buildingSMART International sample files (CC BY 4.0, see NOTICE).
use bim_core::*;
use bim_ifc::{import, step};
use std::collections::HashMap;

const ARCH: &str = include_str!("data/bsi-building-architecture-ifc2x3.ifc");
const WALL_WINDOW: &str = include_str!("data/bsi-wall-with-opening-and-window-ifc4.ifc");

/// Wall name -> (Length, Width) in metres, from the file's own Qto_WallBaseQuantities (mm).
fn wall_quantities(text: &str) -> HashMap<String, (f32, f32)> {
    let f = step::parse(text).unwrap();
    let mut out = HashMap::new();
    for rel in f
        .entities
        .values()
        .filter(|e| e.ty == "IFCRELDEFINESBYPROPERTIES")
    {
        let Some(q) = rel
            .arg(5)
            .id()
            .and_then(|i| f.entities.get(&i))
            .filter(|q| q.ty == "IFCELEMENTQUANTITY")
        else {
            continue;
        };
        let get = |name: &str| {
            q.arg(5)
                .list()
                .iter()
                .filter_map(|v| f.entities.get(&v.id()?))
                .find(|x| x.arg(0).str() == Some(name))
                .and_then(|x| x.arg(3).num())
        };
        let (Some(l), Some(w)) = (get("Length"), get("Width")) else {
            continue;
        };
        for o in rel
            .arg(4)
            .list()
            .iter()
            .filter_map(|v| f.entities.get(&v.id()?))
            .filter(|o| o.ty == "IFCWALL")
        {
            out.insert(
                o.arg(2).str().unwrap().to_string(),
                (l as f32 / 1000.0, w as f32 / 1000.0),
            );
        }
    }
    out
}

#[test]
fn walls_match_the_exporters_own_quantities() {
    let r = import(ARCH, "x").unwrap();
    let q = wall_quantities(ARCH);
    assert_eq!(q.len(), 4);
    for (name, (len, width)) in &q {
        let e = r
            .project
            .elements
            .iter()
            .find(|e| &e.name == name)
            .unwrap_or_else(|| panic!("{name} not imported"));
        let ElementKind::Wall {
            start,
            end,
            thickness,
            ..
        } = e.kind
        else {
            panic!("{name} is not a wall")
        };
        assert!(
            (start.distance(end) - len).abs() < 1e-3,
            "{name}: length {} vs {len}",
            start.distance(end)
        );
        assert!(
            (thickness - width).abs() < 1e-3,
            "{name}: thickness {thickness} vs {width}"
        );
    }
}

#[test]
fn architecture_sample_imports_and_runs() {
    let r = import(ARCH, "x").unwrap();
    let p = &r.project;
    assert_eq!(p.name, "ifc silly sample scene - project");
    let n = |f: fn(&ElementKind) -> bool| p.elements.iter().filter(|e| f(&e.kind)).count();
    assert_eq!(n(|k| matches!(k, ElementKind::Wall { .. })), 4);
    assert_eq!(n(|k| matches!(k, ElementKind::Slab { .. })), 3);
    assert_eq!(p.spaces.len(), 2);
    // one storey; every element and space is on it: walls/slabs by containment, spaces by
    // aggregation, and the two roof slabs (whose IfcRoof sits in the site) by elevation
    assert_eq!(p.levels.len(), 1);
    assert_eq!(p.levels[0].name, "00 groundfloor");
    assert!(
        p.elements.iter().all(|e| e.level == Some(LevelId(0))),
        "{:?}",
        p.elements
            .iter()
            .map(|e| (&e.name, e.level))
            .collect::<Vec<_>>()
    );
    assert!(p.spaces.iter().all(|s| s.level == Some(LevelId(0))));
    assert!(
        r.notes[1]
            .ends_with("2 item(s) not contained in a storey were placed on one by elevation."),
        "{:?}",
        r.notes
    );
    assert!(p.validate().is_empty(), "{:?}", p.validate());
    // the two roof slabs are sloped: imported at their measured 300 mm, not their 2.8-4.2 m box height
    for e in p.elements.iter().filter(|e| e.name.contains("roof")) {
        let ElementKind::Slab { thickness, .. } = e.kind else {
            panic!()
        };
        assert!((thickness - 0.3).abs() < 1e-3, "{}: {thickness}", e.name);
        // part of the IfcRoof: envelope phase, roofing cost code
        assert_eq!(
            (e.phase, e.cost_code.as_str()),
            (PhaseId(1), "B3010"),
            "{}",
            e.name
        );
    }
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("IfcBuildingElementProxy")),
        "{:?}",
        r.notes
    );
    let rs = bim_rules::RuleSet::from_json(include_str!("../../../rules/IN-2026.json")).unwrap();
    let rep = bim_report::build(p, Some(&rs)).unwrap();
    assert_eq!(rep.elements.len(), 7);
    // one missing-rate advisory per cost code, not per element
    assert_eq!(
        rep.advisories
            .iter()
            .filter(|a| a.rule_id == "cost.missing_rate")
            .count(),
        3
    );
}

#[test]
fn ifc4_wall_with_window() {
    let r = import(WALL_WINDOW, "x").unwrap();
    let ElementKind::Wall {
        start,
        end,
        thickness,
        height,
    } = r.project.elements[0].kind
    else {
        panic!()
    };
    assert!(
        (start.distance(end) - 3.0).abs() < 1e-3
            && (thickness - 0.3).abs() < 1e-3
            && (height - 2.0).abs() < 1e-3
    );
    assert!(
        r.notes.iter().any(|n| n.contains("1 IfcWindow")),
        "{:?}",
        r.notes
    );
}
