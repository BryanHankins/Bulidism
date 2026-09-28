use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("unsupported schema version {0}, expected {SCHEMA_VERSION}")]
    Schema(u32),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("element {0:?} not found")]
    MissingElement(ElementId),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phase {
    pub id: PhaseId,
    pub name: String,
    pub duration_days: u32,
    pub depends_on: Vec<PhaseId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub name: String,
    /// Jurisdiction rule set id, e.g. "IN-2026" or "IBC-2024".
    pub rule_set: String,
    pub phases: Vec<Phase>,
    pub elements: Vec<Element>,
    pub spaces: Vec<Space>,
    /// Unit cost in cents per quantity unit, keyed by cost code.
    pub unit_costs: HashMap<String, Cents>,
}

impl Project {
    pub fn new(name: impl Into<String>, rule_set: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            name: name.into(),
            rule_set: rule_set.into(),
            phases: vec![],
            elements: vec![],
            spaces: vec![],
            unit_costs: HashMap::new(),
        }
    }
    pub fn to_json(&self) -> Result<String, ProjectError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(s: &str) -> Result<Self, ProjectError> {
        let p: Project = serde_json::from_str(s)?;
        if p.schema_version != SCHEMA_VERSION {
            return Err(ProjectError::Schema(p.schema_version));
        }
        Ok(p)
    }
    pub fn element(&self, id: ElementId) -> Result<&Element, ProjectError> {
        self.elements
            .iter()
            .find(|e| e.id == id)
            .ok_or(ProjectError::MissingElement(id))
    }
    pub fn add(&mut self, e: Element) -> ElementId {
        let id = e.id;
        self.elements.push(e);
        id
    }

    pub fn phase(&self, id: PhaseId) -> Option<&Phase> {
        self.phases.iter().find(|p| p.id == id)
    }

    /// Doors hosted by `wall`, as (offset, width, height) sorted by offset.
    pub fn openings_in(&self, wall: ElementId) -> Vec<(f32, f32, f32)> {
        let mut v: Vec<(f32, f32, f32)> = self
            .elements
            .iter()
            .filter_map(|e| match e.kind {
                ElementKind::Door {
                    host,
                    offset,
                    width,
                    height,
                    ..
                } if host == wall => Some((offset, width, height)),
                _ => None,
            })
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    }

    /// Structural integrity checks a UI must run before rendering or scheduling.
    pub fn validate(&self) -> Vec<ProjectError> {
        let mut errs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for ph in &self.phases {
            if !seen.insert(ph.id) {
                errs.push(ProjectError::Invalid(format!(
                    "duplicate phase {:?}",
                    ph.id
                )));
            }
        }
        for e in &self.elements {
            if self.phase(e.phase).is_none() {
                errs.push(ProjectError::Invalid(format!(
                    "element '{}' references unknown phase {:?}",
                    e.name, e.phase
                )));
            }
            let bad_dim = match &e.kind {
                ElementKind::Wall {
                    height,
                    thickness,
                    start,
                    end,
                } => *height <= 0.0 || *thickness <= 0.0 || start.distance(*end) <= 0.0,
                ElementKind::Slab {
                    size_x,
                    size_z,
                    thickness,
                    ..
                } => *size_x <= 0.0 || *size_z <= 0.0 || *thickness <= 0.0,
                ElementKind::Column {
                    width,
                    depth,
                    height,
                    ..
                } => *width <= 0.0 || *depth <= 0.0 || *height <= 0.0,
                ElementKind::Beam {
                    width,
                    depth,
                    start,
                    end,
                } => *width <= 0.0 || *depth <= 0.0 || start.distance(*end) <= 0.0,
                ElementKind::Door { width, height, .. } => *width <= 0.0 || *height <= 0.0,
                ElementKind::Stair {
                    run_length,
                    total_rise,
                    width,
                    riser_count,
                    ..
                } => *run_length <= 0.0 || *total_rise <= 0.0 || *width <= 0.0 || *riser_count == 0,
                ElementKind::Ramp {
                    run_length,
                    rise,
                    width,
                    ..
                } => *run_length <= 0.0 || *rise < 0.0 || *width <= 0.0,
                ElementKind::Mesh { asset, .. } => asset.is_empty(),
            };
            if bad_dim {
                errs.push(ProjectError::Invalid(format!(
                    "element '{}' has non-positive dimensions",
                    e.name
                )));
            }
            if let ElementKind::Door {
                host,
                offset,
                width,
                height,
                ..
            } = &e.kind
            {
                match self.element(*host) {
                    Ok(Element {
                        kind:
                            ElementKind::Wall {
                                start,
                                end,
                                height: wh,
                                ..
                            },
                        ..
                    }) => {
                        if offset + width > start.distance(*end) + 1e-4 {
                            errs.push(ProjectError::Invalid(format!(
                                "door '{}' extends past its host wall",
                                e.name
                            )));
                        }
                        if height > wh {
                            errs.push(ProjectError::Invalid(format!(
                                "door '{}' taller than its host wall",
                                e.name
                            )));
                        }
                    }
                    Ok(_) => errs.push(ProjectError::Invalid(format!(
                        "door '{}' host is not a wall",
                        e.name
                    ))),
                    Err(err) => errs.push(err),
                }
            }
        }
        for s in &self.spaces {
            if s.area_m2() <= 0.0 {
                errs.push(ProjectError::Invalid(format!(
                    "space '{}' has non-positive floor area",
                    s.name
                )));
            }
            for d in &s.egress_doors {
                match self.elements.iter().find(|e| e.id == *d) {
                    None => errs.push(ProjectError::Invalid(format!(
                        "space '{}' references unknown egress door",
                        s.name
                    ))),
                    Some(e) if !matches!(e.kind, ElementKind::Door { .. }) => {
                        errs.push(ProjectError::Invalid(format!(
                            "space '{}' egress reference '{}' is not a door",
                            s.name, e.name
                        )))
                    }
                    Some(_) => {}
                }
            }
        }
        errs
    }

    /// Small demo used by the viewer and tests. Entry door is deliberately
    /// narrower than the egress minimum so the rule engine has something to flag.
    /// Element ids are fixed so exported assets and golden files stay stable.
    pub fn sample() -> Self {
        use glam::vec3;
        let mut p = Project::new("Sample Office", "IN-2026");
        p.phases = vec![
            Phase {
                id: PhaseId(0),
                name: "Foundation".into(),
                duration_days: 10,
                depends_on: vec![],
            },
            Phase {
                id: PhaseId(1),
                name: "Structure".into(),
                duration_days: 15,
                depends_on: vec![PhaseId(0)],
            },
            Phase {
                id: PhaseId(2),
                name: "Envelope".into(),
                duration_days: 12,
                depends_on: vec![PhaseId(1)],
            },
            Phase {
                id: PhaseId(3),
                name: "Fit-out".into(),
                duration_days: 20,
                depends_on: vec![PhaseId(2)],
            },
        ];
        p.unit_costs.insert("A1010".into(), Cents(18_000)); // slab per m3
        p.unit_costs.insert("B1010".into(), Cents(95_000)); // columns per m3
        p.unit_costs.insert("B2010".into(), Cents(22_000)); // ext walls per m2
        p.unit_costs.insert("C1020".into(), Cents(85_000)); // doors each
        let mut e = Element::new(
            "Ground slab",
            ElementKind::Slab {
                origin: vec3(0.0, 0.0, 0.0),
                size_x: 12.0,
                size_z: 8.0,
                thickness: 0.2,
            },
            Discipline::Structural,
            PhaseId(0),
        );
        e.cost_code = "A1010".into();
        p.add(e);
        for (i, (x, z)) in [(0.0, 0.0), (12.0, 0.0), (0.0, 8.0), (12.0, 8.0)]
            .iter()
            .enumerate()
        {
            let mut c = Element::new(
                format!("Column {}", i + 1),
                ElementKind::Column {
                    base: vec3(*x, 0.2, *z),
                    width: 0.4,
                    depth: 0.4,
                    height: 3.2,
                },
                Discipline::Structural,
                PhaseId(1),
            );
            c.cost_code = "B1010".into();
            p.add(c);
        }
        let mut w = Element::new(
            "South wall",
            ElementKind::Wall {
                start: vec3(0.0, 0.2, 0.0),
                end: vec3(12.0, 0.2, 0.0),
                height: 3.0,
                thickness: 0.25,
            },
            Discipline::Architectural,
            PhaseId(2),
        );
        w.cost_code = "B2010".into();
        let wall_id = p.add(w);
        let mut d = Element::new(
            "Main entry",
            ElementKind::Door {
                host: wall_id,
                offset: 5.0,
                width: 0.75,
                height: 2.1,
                is_egress: true,
            },
            Discipline::Architectural,
            PhaseId(3),
        );
        d.cost_code = "C1020".into();
        let door_id = p.add(d);
        let mut w2 = Element::new(
            "North wall",
            ElementKind::Wall {
                start: vec3(0.0, 0.2, 8.0),
                end: vec3(12.0, 0.2, 8.0),
                height: 3.0,
                thickness: 0.25,
            },
            Discipline::Architectural,
            PhaseId(2),
        );
        w2.cost_code = "B2010".into();
        w2.start_offset_days = 4;
        let w2_id = p.add(w2);
        let mut d2 = Element::new(
            "Rear exit",
            ElementKind::Door {
                host: w2_id,
                offset: 9.0,
                width: 0.9,
                height: 2.1,
                is_egress: true,
            },
            Discipline::Architectural,
            PhaseId(3),
        );
        d2.cost_code = "C1020".into();
        let door2_id = p.add(d2);
        let mut b = Element::new(
            "Roof beam",
            ElementKind::Beam {
                start: vec3(0.0, 3.4, 0.0),
                end: vec3(12.0, 3.4, 8.0),
                width: 0.3,
                depth: 0.5,
            },
            Discipline::Structural,
            PhaseId(1),
        );
        b.cost_code = "B1010".into();
        b.start_offset_days = 8;
        p.add(b);
        let mut st = Element::new(
            "Stair to mezzanine",
            ElementKind::Stair {
                base: vec3(1.0, 0.2, 3.0),
                run_length: 3.5,
                total_rise: 3.0,
                width: 1.2,
                riser_count: 17,
            },
            Discipline::Architectural,
            PhaseId(3),
        );
        st.cost_code = "C2010".into();
        p.add(st);
        let mut r = Element::new(
            "Entry ramp",
            ElementKind::Ramp {
                base: vec3(6.0, 0.0, -3.0),
                run_length: 2.4,
                rise: 0.2,
                width: 1.2,
            },
            Discipline::Site,
            PhaseId(3),
        );
        r.cost_code = "G2030".into();
        p.add(r);
        p.unit_costs.insert("C2010".into(), Cents(120_000)); // stairs per m run
        p.unit_costs.insert("G2030".into(), Cents(35_000)); // ramp per m2
        p.spaces.push(Space {
            name: "Open office".into(),
            occupancy: Occupancy::Business,
            floor_area_m2: 96.0,
            egress_doors: vec![door_id, door2_id],
            bounds: Some([0.0, 0.0, 12.0, 8.0]),
        });
        p.assign_fixed_ids(0x5a3b_0000_0000_4000_8000_0000_0000_0000);
        p
    }

    /// Replace every element id with a deterministic one derived from `seed` + index,
    /// rewriting door hosts and space egress references. Used for stable fixtures.
    pub fn assign_fixed_ids(&mut self, seed: u128) {
        let map: HashMap<ElementId, ElementId> = self
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, ElementId(uuid::Uuid::from_u128(seed + i as u128 + 1))))
            .collect();
        for e in &mut self.elements {
            e.id = map[&e.id];
            if let ElementKind::Door { host, .. } = &mut e.kind {
                if let Some(n) = map.get(host) {
                    *host = *n;
                }
            }
        }
        for s in &mut self.spaces {
            for d in &mut s.egress_doors {
                if let Some(n) = map.get(d) {
                    *d = *n;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_round_trip() {
        let p = Project::sample();
        let s = p.to_json().unwrap();
        assert_eq!(Project::from_json(&s).unwrap(), p);
    }
    #[test]
    fn rejects_bad_schema() {
        let s = r#"{"schema_version":99,"name":"x","rule_set":"x","phases":[],"elements":[],"spaces":[],"unit_costs":{}}"#;
        assert!(matches!(
            Project::from_json(s),
            Err(ProjectError::Schema(99))
        ));
    }
    #[test]
    fn missing_element() {
        assert!(Project::sample().element(ElementId::new()).is_err());
    }
    #[test]
    fn sample_validates() {
        let e = Project::sample().validate();
        assert!(e.is_empty(), "{e:?}");
    }
    #[test]
    fn validate_catches_bad_door() {
        let mut p = Project::sample();
        let wall = p
            .elements
            .iter()
            .find(|e| e.name == "South wall")
            .unwrap()
            .id;
        p.add(Element::new(
            "bad",
            ElementKind::Door {
                host: wall,
                offset: 11.8,
                width: 1.0,
                height: 2.0,
                is_egress: false,
            },
            Discipline::Architectural,
            PhaseId(9),
        ));
        let errs = p.validate();
        assert_eq!(errs.len(), 2, "{errs:?}");
    }
    #[test]
    fn space_area_follows_bounds() {
        let mut p = Project::sample();
        assert_eq!(p.spaces[0].area_m2(), 96.0);
        p.spaces[0].bounds = Some([0.0, 0.0, 10.0, 5.0]);
        assert_eq!(p.spaces[0].area_m2(), 50.0);
        p.spaces[0].bounds = None;
        assert_eq!(p.spaces[0].area_m2(), 96.0);
    }
    #[test]
    fn validate_rejects_non_door_egress_and_zero_area() {
        let mut p = Project::sample();
        let wall = p
            .elements
            .iter()
            .find(|e| e.name == "South wall")
            .unwrap()
            .id;
        p.spaces[0].egress_doors = vec![wall];
        p.spaces.push(Space::new("empty", Occupancy::Storage, 0.0));
        let errs = p.validate();
        assert_eq!(errs.len(), 2, "{errs:?}");
    }
    #[test]
    fn sample_is_deterministic() {
        let a = Project::sample();
        let b = Project::sample();
        assert_eq!(a, b);
        assert!(a.validate().is_empty());
    }
    #[test]
    fn openings_sorted() {
        let p = Project::sample();
        let wall = p
            .elements
            .iter()
            .find(|e| e.name == "North wall")
            .unwrap()
            .id;
        assert_eq!(p.openings_in(wall).len(), 1);
    }
}
