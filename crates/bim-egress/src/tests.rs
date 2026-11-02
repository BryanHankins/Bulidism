use super::*;
use glam::vec3;

/// Walls along `pts` (closed or open polyline at y = 0, 0.2 m thick); returns their ids.
fn walls(p: &mut Project, pts: &[[f32; 2]]) -> Vec<ElementId> {
    pts.windows(2)
        .map(|w| {
            p.add(Element::new(
                "wall",
                ElementKind::Wall {
                    start: vec3(w[0][0], 0.0, w[0][1]),
                    end: vec3(w[1][0], 0.0, w[1][1]),
                    height: 3.0,
                    thickness: 0.2,
                },
                Discipline::Architectural,
                PhaseId(0),
            ))
        })
        .collect()
}
fn door(p: &mut Project, name: &str, host: ElementId, offset: f32, egress: bool) -> ElementId {
    p.add(Element::new(
        name,
        ElementKind::Door {
            host,
            offset,
            width: 1.0,
            height: 2.1,
            is_egress: egress,
        },
        Discipline::Architectural,
        PhaseId(0),
    ))
}
fn room(p: &mut Project, name: &str, outline: &[[f32; 2]]) {
    let mut s = Space::new(name, Occupancy::Business, 0.0);
    s.outline = Some(outline.to_vec());
    p.spaces.push(s);
}
fn project() -> Project {
    let mut p = Project::new("t", "IN-2026");
    p.phases.push(Phase {
        id: PhaseId(0),
        name: "p".into(),
        duration_days: 1,
        depends_on: vec![],
    });
    p
}
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.02
}

#[test]
fn rectangle_room_one_exit() {
    let mut p = project();
    let w = walls(
        &mut p,
        &[[0., 0.], [10., 0.], [10., 6.], [0., 6.], [0., 0.]],
    );
    door(&mut p, "Exit", w[0], 4.5, true); // centre (5, 0)
    room(&mut p, "Room", &[[0., 0.], [10., 0.], [10., 6.], [0., 6.]]);
    let a = analyze(&p);
    assert_eq!(a.doors.len(), 1);
    assert!(a.doors[0].exit && a.doors[0].sides.contains(&None));
    let s = &a.spaces[0];
    // far corner (0 or 10, 6) to the door's inside point (5, 0.12), then 0.12 m through it
    let expect = (25.0f32 + 5.88 * 5.88).sqrt() + 0.12;
    assert!(
        near(s.travel_m.unwrap(), expect),
        "{:?} vs {expect}",
        s.travel_m
    );
    // one exit: the whole route is common path
    assert!(near(s.common_path_m.unwrap(), expect));
    assert_eq!(s.path.len(), 3, "corner, door point, exit: {:?}", s.path);
    assert_eq!(s.exit_door, Some(p.elements[4].id));
}

#[test]
fn l_room_walks_around_its_inside_corner() {
    let mut p = project();
    let l = [[0., 0.], [6., 0.], [6., 2.], [2., 2.], [2., 5.], [0., 5.]];
    let mut ring = l.to_vec();
    ring.push(l[0]);
    let w = walls(&mut p, &ring);
    door(&mut p, "Exit", w[0], 4.5, true); // centre (5, 0)
    room(&mut p, "L", &l);
    let s = &analyze(&p).spaces[0];
    // (0, 5) -> reflex corner (2, 2) -> (5, 0.12) -> through the door
    let expect = (4.0f32 + 9.0).sqrt() + (9.0f32 + 1.88 * 1.88).sqrt() + 0.12;
    assert!(
        near(s.travel_m.unwrap(), expect),
        "{:?} vs {expect}",
        s.travel_m
    );
    assert!(
        s.path.iter().any(|q| near(q[0], 2.0) && near(q[1], 2.0)),
        "bends at the corner: {:?}",
        s.path
    );
}

/// Two offices off a corridor with an exit at each end.
fn corridor_plan() -> Project {
    let mut p = project();
    // corridor x 0..12, z 0..2; offices A (x 0..6) and B (x 6..12), z 2..8
    let outer = walls(
        &mut p,
        &[[0., 0.], [12., 0.], [12., 8.], [0., 8.], [0., 0.]],
    );
    let mid = walls(&mut p, &[[0., 2.], [12., 2.]]);
    walls(&mut p, &[[6., 2.], [6., 8.]]);
    door(&mut p, "West exit", outer[3], 6.5, true); // wall (0,8)->(0,0): centre (0, 1)
    door(&mut p, "East exit", outer[1], 0.5, true); // wall (12,0)->(12,8): centre (12, 1)
    door(&mut p, "A door", mid[0], 2.5, false); // centre (3, 2)
    door(&mut p, "B door", mid[0], 8.5, false); // centre (9, 2)
    room(
        &mut p,
        "Corridor",
        &[[0., 0.], [12., 0.], [12., 2.], [0., 2.]],
    );
    room(
        &mut p,
        "Office A",
        &[[0., 2.], [6., 2.], [6., 8.], [0., 8.]],
    );
    room(
        &mut p,
        "Office B",
        &[[6., 2.], [12., 2.], [12., 8.], [6., 8.]],
    );
    p
}

#[test]
fn offices_off_a_corridor_with_two_exits() {
    let p = corridor_plan();
    assert!(p.validate().is_empty(), "{:?}", p.validate());
    let a = analyze(&p);
    let link = |n: &str| a.doors.iter().find(|d| d.name == n).unwrap();
    assert!(link("West exit").exit && link("East exit").exit);
    assert!(!link("A door").exit);
    let mut sides = link("A door").sides;
    sides.sort();
    assert_eq!(sides, [Some(0), Some(1)], "corridor and office A");
    // corridor: two exits, so no common path
    assert_eq!(a.spaces[0].common_path_m, Some(0.0));
    // office A: far corner (0 or 6, 8) to its door (3, 2.12), through the door (0.24 m), along
    // the corridor from (3, 1.88) to the west exit's inside point (0.12, 1), out (0.12 m)
    let to_door = (9.0f32 + 5.88 * 5.88).sqrt();
    let along = (2.88f32 * 2.88 + 0.88 * 0.88).sqrt();
    let off_a = &a.spaces[1];
    assert!(
        near(off_a.travel_m.unwrap(), to_door + 0.24 + along + 0.12),
        "{:?}",
        off_a.travel_m
    );
    // common path ends on entering the corridor, where the two exits are separate routes
    assert!(
        near(off_a.common_path_m.unwrap(), to_door + 0.24),
        "{:?}",
        off_a.common_path_m
    );
    // symmetric plan: office B the same, toward the east exit
    assert!(near(a.spaces[2].travel_m.unwrap(), off_a.travel_m.unwrap()));
    assert_eq!(
        a.spaces[2].exit_door,
        Some(
            p.elements
                .iter()
                .find(|e| e.name == "East exit")
                .unwrap()
                .id
        )
    );
    // the path runs corner -> A door (both sides) -> corridor -> exit
    assert_eq!(off_a.path.len(), 5, "{:?}", off_a.path);
}

#[test]
fn no_exit_and_no_shape() {
    let mut p = corridor_plan();
    for e in &mut p.elements {
        if let ElementKind::Door { is_egress, .. } = &mut e.kind {
            *is_egress = false;
        }
    }
    p.spaces
        .push(Space::new("Area only", Occupancy::Business, 20.0));
    let a = analyze(&p);
    assert!(a.spaces[..3]
        .iter()
        .all(|s| s.analyzed && s.travel_m.is_none()));
    assert!(!a.spaces[3].analyzed);
}

#[test]
fn a_stair_room_is_an_exit() {
    let mut p = corridor_plan();
    // west exit no longer egress; office A instead gets a stair: its door becomes an exit
    for e in &mut p.elements {
        if let ElementKind::Door { is_egress, .. } = &mut e.kind {
            if e.name == "West exit" {
                *is_egress = false;
            }
        }
    }
    p.add(Element::new(
        "Stair",
        ElementKind::Stair {
            base: vec3(1.0, 0.0, 4.0),
            run_length: 3.0,
            total_rise: 3.0,
            width: 1.2,
            riser_count: 17,
        },
        Discipline::Architectural,
        PhaseId(0),
    ));
    // an open stair in a 36 m2 office is not an exit enclosure ...
    assert!(!analyze(&p).spaces[1].stair);
    // ... a room named as the stair is
    p.spaces[1].name = "Stair A".into();
    let a = analyze(&p);
    assert!(a.spaces[1].stair);
    assert!(a.doors.iter().find(|d| d.name == "A door").unwrap().exit);
    // corridor: the stair door and the east exit are two separate routes
    assert_eq!(a.spaces[0].common_path_m, Some(0.0));
    // office B's nearest exit from its west half is the stair door
    assert!(a.spaces[2].travel_m.is_some());
}

/// `n` offices (6 x 6 m) on each side of a long corridor with an exit at each end.
fn floor_of_offices(n: usize) -> Project {
    let mut p = project();
    let len = 6.0 * n as f32;
    let outer = walls(
        &mut p,
        &[[0., -6.], [len, -6.], [len, 8.], [0., 8.], [0., -6.]],
    );
    let south = walls(&mut p, &[[0., 0.], [len, 0.]])[0];
    let north = walls(&mut p, &[[0., 2.], [len, 2.]])[0];
    door(&mut p, "West exit", outer[3], 12.5, true); // wall (0,8)->(0,-6): centre (0, 1)
    door(&mut p, "East exit", outer[1], 6.5, true); // wall (len,-6)->(len,8): centre (len, 1)
    room(
        &mut p,
        "Corridor",
        &[[0., 0.], [len, 0.], [len, 2.], [0., 2.]],
    );
    for i in 0..n {
        let x = 6.0 * i as f32;
        walls(&mut p, &[[x + 6.0, -6.0], [x + 6.0, 0.0]]);
        walls(&mut p, &[[x + 6.0, 2.0], [x + 6.0, 8.0]]);
        door(&mut p, &format!("S{i}"), south, x + 2.5, false);
        door(&mut p, &format!("N{i}"), north, x + 2.5, false);
        room(
            &mut p,
            &format!("South {i}"),
            &[[x, -6.], [x + 6., -6.], [x + 6., 0.], [x, 0.]],
        );
        room(
            &mut p,
            &format!("North {i}"),
            &[[x, 2.], [x + 6., 2.], [x + 6., 8.], [x, 8.]],
        );
    }
    p
}

#[test]
fn forty_offices_on_a_corridor() {
    let p = floor_of_offices(20);
    let t0 = std::time::Instant::now();
    let a = analyze(&p);
    let ms = t0.elapsed().as_millis();
    assert!(a.spaces.iter().all(|s| s.travel_m.is_some()));
    // the middle offices are farthest from both exits: about half the corridor (60 m) plus the room
    let mid = &a.spaces[1 + 2 * 10];
    assert!(
        (60.0..70.0).contains(&mid.travel_m.unwrap()),
        "{:?}",
        mid.travel_m
    );
    // every office's common path ends at its door into the corridor (about 6.5 m)
    assert!(a.spaces[1..].iter().all(|s| s.common_path_m.unwrap() < 7.0));
    assert!(ms < 5_000, "{ms} ms for 41 rooms");
}

#[test]
#[ignore = "scale benchmark; run in release with --ignored --nocapture"]
fn four_hundred_offices() {
    let p = floor_of_offices(200);
    let t0 = std::time::Instant::now();
    let a = analyze(&p);
    println!(
        "{} rooms, {} doors: {:?}",
        p.spaces.len(),
        a.doors.len(),
        t0.elapsed()
    );
    assert!(a.spaces.iter().all(|s| s.travel_m.is_some()));
}
