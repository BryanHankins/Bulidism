use super::*;

const FIXTURE: &str = include_str!("../tests/data/fixture.ifc");

fn by_name<'a>(p: &'a Project, n: &str) -> &'a Element {
    p.elements
        .iter()
        .find(|e| e.name == n)
        .unwrap_or_else(|| panic!("no element {n}"))
}
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}
fn near3(a: Vec3, b: Vec3) -> bool {
    a.distance(b) < 1e-3
}

#[test]
fn guid_decodes_to_uuid() {
    let a = guid_to_uuid("0WallA000000000000000000").is_none(); // 24 chars
    assert!(a);
    assert!(guid_to_uuid("4WallA0000000000000000").is_none()); // first char > 3
    let u = guid_to_uuid("3$$$$$$$$$$$$$$$$$$$$$").unwrap();
    assert_eq!(u.as_u128(), u128::MAX);
    assert_eq!(guid_to_uuid("0000000000000000000001").unwrap().as_u128(), 1);
}

#[test]
fn min_rect_fits_rotated_rectangle() {
    let (s, c) = (
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    );
    let pts: Vec<DVec2> = [
        (-1.5, -0.1),
        (1.5, -0.1),
        (1.5, 0.1),
        (-1.5, 0.1),
        (0.0, 0.0),
    ]
    .iter()
    .map(|(u, v)| DVec2::new(10.0 + u * c - v * s, 5.0 + u * s + v * c))
    .collect();
    let (ctr, u, a, b) = min_rect(&pts).unwrap();
    assert!((a - 1.5).abs() < 1e-9 && (b - 0.1).abs() < 1e-9, "{a} {b}");
    assert!(ctr.distance(DVec2::new(10.0, 5.0)) < 1e-9);
    assert!((u.x - c).abs() < 1e-9 && (u.y - s).abs() < 1e-9, "{u}");
    // collinear points: zero width, so callers can skip them
    let (_, _, a, b) = min_rect(&[DVec2::ZERO, DVec2::X, DVec2::X * 2.0]).unwrap();
    assert!((a - 1.0).abs() < 1e-9 && b.abs() < 1e-9);
    assert!(min_rect(&[DVec2::ONE, DVec2::ONE]).is_none());
}

#[test]
fn fixture_imports_every_supported_kind() {
    let r = import(FIXTURE, "fallback").unwrap();
    let p = &r.project;
    assert_eq!(p.name, "Fixture Clinic");
    assert!(p.validate().is_empty(), "{:?}", p.validate());
    assert_eq!(
        r.notes[0],
        "Imported 1 beams, 1 columns, 1 doors, 1 slabs, 1 spaces, 1 stairs, 4 walls from IFC4 (millimetres)."
    );
    // two storeys, lowest first, at their world elevations (3 m and 6 m)
    let lv: Vec<(&str, f32)> = p
        .levels
        .iter()
        .map(|l| (l.name.as_str(), l.elevation_m))
        .collect();
    assert_eq!(lv, vec![("Level 1", 3.0), ("Level 2", 6.0)]);
    assert!(
        r.notes[1].starts_with("2 storey(s): Level 1 at 3.00 m, Level 2 at 6.00 m"),
        "{:?}",
        r.notes
    );
    assert_eq!(by_name(p, "Wall D upper").level, Some(LevelId(1)));
    for e in p.elements.iter().filter(|e| e.name != "Wall D upper") {
        assert_eq!(e.level, Some(LevelId(0)), "{}", e.name);
    }
    assert_eq!(p.spaces[0].level, Some(LevelId(0))); // aggregated under the storey
                                                     // model axes: x = IFC x, y = IFC z (up), z = -IFC y; re-centred so the plan starts at 0
                                                     // (IFC y max 4000 mm -> model z 0). Storey at +3000 mm.
    let ElementKind::Wall {
        start,
        end,
        height,
        thickness,
    } = by_name(p, "Wall A").kind
    else {
        panic!()
    };
    assert!(
        near3(start, Vec3::new(0.0, 3.0, 4.0)) && near3(end, Vec3::new(6.0, 3.0, 4.0)),
        "{start} {end}"
    );
    assert!(near(height, 3.0) && near(thickness, 0.2));
    let ElementKind::Wall { start, end, .. } = by_name(p, "Wall B").kind else {
        panic!()
    };
    assert!(
        near3(start, Vec3::new(6.0, 3.0, 0.0)) && near3(end, Vec3::new(6.0, 3.0, 4.0)),
        "{start} {end}"
    );
    let ElementKind::Wall {
        start,
        end,
        thickness,
        ..
    } = by_name(p, "Wall C diagonal").kind
    else {
        panic!()
    };
    assert!(
        near(start.distance(end), 3.0) && near(thickness, 0.2),
        "{start} {end} {thickness}"
    );
    assert!(
        near((end.x - start.x).abs(), (end.z - start.z).abs()),
        "45 degrees"
    );

    let wall_a = by_name(p, "Wall A").id;
    let ElementKind::Door {
        host,
        offset,
        width,
        height,
        is_egress,
    } = by_name(p, "Front door").kind
    else {
        panic!()
    };
    assert_eq!(host, wall_a);
    assert!(
        near(offset, 2.0) && near(width, 0.9) && near(height, 2.1) && is_egress,
        "{offset} {width} {height}"
    );

    let ElementKind::Slab {
        origin,
        size_x,
        size_z,
        thickness,
    } = by_name(p, "Floor slab").kind
    else {
        panic!()
    };
    assert!(
        near3(origin, Vec3::new(0.0, 2.8, 0.0))
            && near(size_x, 6.0)
            && near(size_z, 4.0)
            && near(thickness, 0.2)
    );
    let ElementKind::Column {
        base,
        width,
        depth,
        height,
    } = by_name(p, "Column C1").kind
    else {
        panic!()
    };
    assert!(
        near3(base, Vec3::new(5.0, 3.0, 1.0))
            && near(width, 0.4)
            && near(depth, 0.4)
            && near(height, 3.0),
        "{base}"
    );
    let ElementKind::Beam {
        start,
        end,
        width,
        depth,
    } = by_name(p, "Beam B1").kind
    else {
        panic!()
    };
    assert!(
        near3(start, Vec3::new(0.0, 5.6, 2.0)) && near3(end, Vec3::new(6.0, 5.6, 2.0)),
        "{start} {end}"
    );
    assert!(near(width, 0.3) && near(depth, 0.4));
    let ElementKind::Stair {
        base,
        run_length,
        total_rise,
        width,
        riser_count,
    } = by_name(p, "Stair flight").kind
    else {
        panic!()
    };
    assert!(
        near3(base, Vec3::new(1.0, 3.0, 1.3))
            && near(run_length, 3.0)
            && near(total_rise, 1.5)
            && near(width, 1.2),
        "{base}"
    );
    assert_eq!(riser_count, 9);

    let sp = &p.spaces[0];
    assert_eq!(sp.name, "Waiting Room");
    assert_eq!(sp.occupancy, Occupancy::Assembly);
    assert!(near(sp.floor_area_m2, 23.5));
    assert_eq!(sp.bounds, Some([0.0, 0.0, 6.0, 4.0])); // measured area within 10% of the box: outline kept
    assert_eq!(sp.egress_doors, vec![by_name(p, "Front door").id]);

    assert_eq!(
        by_name(p, "Wall A").id.0,
        guid_to_uuid("0WallA0000000000000000").unwrap()
    );
    assert!(
        r.notes.iter().any(|n| n.contains("1 IfcWindow")),
        "{:?}",
        r.notes
    );
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("1 door(s) marked egress")),
        "{:?}",
        r.notes
    );
}

#[test]
fn fixture_runs_through_the_pipeline() {
    let p = import(FIXTURE, "x").unwrap().project;
    let rs = bim_rules::RuleSet::from_json(include_str!("../../../rules/IN-2026.json")).unwrap();
    let r = bim_report::build(&p, Some(&rs)).unwrap();
    assert_eq!(r.elements.len(), p.elements.len());
    // no unit costs in an IFC: every code is reported unpriced
    assert!(r
        .advisories
        .iter()
        .any(|a| a.rule_id == "cost.missing_rate"));
}

#[test]
fn feet_and_rejects() {
    let ft = FIXTURE.replace(
        "#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
        "#5=IFCCONVERSIONBASEDUNIT(#8,.LENGTHUNIT.,'FOOT',#9);\n#8=IFCDIMENSIONALEXPONENTS(1,0,0,0,0,0,0);\n#9=IFCMEASUREWITHUNIT(IFCLENGTHMEASURE(0.3048),#10);\n#10=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);",
    );
    let p = import(&ft, "x").unwrap().project;
    let ElementKind::Wall { thickness, .. } = by_name(&p, "Wall A").kind else {
        panic!()
    };
    assert!(near(thickness, 200.0 * 0.3048), "{thickness}"); // the numbers are now feet
    assert!(matches!(import("hello", "x"), Err(IfcError::Step(_))));
    assert!(matches!(
        import("ISO-10303-21;\nDATA;\n#1=IFCWALL('a');\nENDSEC;", "x"),
        Err(IfcError::NoProject)
    ));
}
