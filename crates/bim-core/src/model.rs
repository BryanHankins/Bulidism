use glam::Vec3;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementId(pub Uuid);
impl ElementId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ElementId {
    fn default() -> Self {
        Self::new()
    }
}

/// Construction phase index; lower phases are built first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PhaseId(pub u32);

/// Money as integer cents. Never f64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct Cents(pub i64);
impl Cents {
    pub fn dollars(&self) -> f64 {
        self.0 as f64 / 100.0
    }
}
impl std::ops::Add for Cents {
    type Output = Cents;
    fn add(self, o: Cents) -> Cents {
        Cents(self.0 + o.0)
    }
}
impl std::iter::Sum for Cents {
    fn sum<I: Iterator<Item = Cents>>(i: I) -> Cents {
        i.fold(Cents(0), |a, b| a + b)
    }
}

/// BIMForum Level of Development.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lod {
    Lod100,
    Lod200,
    Lod300,
    Lod350,
    Lod400,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Discipline {
    Architectural,
    Structural,
    Mechanical,
    Electrical,
    Plumbing,
    Site,
}

/// Parametric geometry. All units metres. Y is up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ElementKind {
    /// IfcWall: from start to end (plan view), extruded up.
    Wall {
        start: Vec3,
        end: Vec3,
        height: f32,
        thickness: f32,
    },
    /// IfcSlab: axis-aligned footprint at elevation origin.y.
    Slab {
        origin: Vec3,
        size_x: f32,
        size_z: f32,
        thickness: f32,
    },
    /// IfcColumn: rectangular section at base point.
    Column {
        base: Vec3,
        width: f32,
        depth: f32,
        height: f32,
    },
    /// IfcBeam: rectangular section between two points.
    Beam {
        start: Vec3,
        end: Vec3,
        width: f32,
        depth: f32,
    },
    /// IfcDoor: opening in a wall. host must be a Wall.
    Door {
        host: ElementId,
        offset: f32,
        width: f32,
        height: f32,
        is_egress: bool,
    },
    /// IfcStair: straight run along +X from base; risers are equal height.
    Stair {
        base: Vec3,
        run_length: f32,
        total_rise: f32,
        width: f32,
        riser_count: u32,
    },
    /// IfcRamp: straight run along +X from base.
    Ramp {
        base: Vec3,
        run_length: f32,
        rise: f32,
        width: f32,
    },
    /// Pre-built mesh (glTF node): sequenced/costed but not parametric.
    Mesh { asset: String, origin: Vec3 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    pub name: String,
    pub kind: ElementKind,
    pub discipline: Discipline,
    pub phase: PhaseId,
    /// Days after the phase start before this element appears (4D staggering within a phase).
    #[serde(default)]
    pub start_offset_days: u32,
    pub lod: Lod,
    /// Uniformat II-style cost code, e.g. "B2010".
    pub cost_code: String,
    pub material: String,
}

impl Element {
    pub fn new(
        name: impl Into<String>,
        kind: ElementKind,
        discipline: Discipline,
        phase: PhaseId,
    ) -> Self {
        Self {
            id: ElementId::new(),
            name: name.into(),
            kind,
            discipline,
            phase,
            start_offset_days: 0,
            lod: Lod::Lod200,
            cost_code: String::new(),
            material: String::new(),
        }
    }
}

/// Occupancy per IBC Chapter 3 (subset). Drives occupant-load rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Occupancy {
    Assembly,
    Business,
    Educational,
    Residential,
    Mercantile,
    Storage,
    Institutional,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub name: String,
    pub occupancy: Occupancy,
    pub floor_area_m2: f32,
    pub egress_doors: Vec<ElementId>,
    /// Optional plan rectangle [x, z, size_x, size_z]; when set, `floor_area_m2`
    /// is recomputed from it so the area cannot drift from the drawn extent.
    #[serde(default)]
    pub bounds: Option<[f32; 4]>,
}

impl Space {
    pub fn new(name: impl Into<String>, occupancy: Occupancy, floor_area_m2: f32) -> Self {
        Self {
            name: name.into(),
            occupancy,
            floor_area_m2,
            egress_doors: vec![],
            bounds: None,
        }
    }
    /// Area implied by `bounds` when present, otherwise the stated area.
    pub fn area_m2(&self) -> f32 {
        self.bounds
            .map(|[_, _, sx, sz]| sx * sz)
            .unwrap_or(self.floor_area_m2)
    }
}
