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

/// Building-wide facts the code analysis needs and the model cannot infer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BuildingInfo {
    /// IBC construction type, e.g. "IIB" (recorded and reported; allowable area/height not evaluated).
    #[serde(default)]
    pub construction_type: Option<String>,
    /// Automatic sprinkler system throughout (NFPA 13 / 13R).
    #[serde(default)]
    pub sprinklered: bool,
    /// Emergency voice/alarm communication system.
    #[serde(default)]
    pub voice_alarm: bool,
}

/// Building storey (IfcBuildingStorey).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LevelId(pub u32);

/// A storey: name and finished-floor elevation (metres, Y up).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub id: LevelId,
    pub name: String,
    pub elevation_m: f32,
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
        /// Door leaves: 1 (default), 2 for a pair, 0 for an opening with no door in it (a
        /// cased opening). `width` is the whole opening.
        #[serde(default = "one", skip_serializing_if = "is_one")]
        leaves: u32,
    },
    /// IfcWindow: opening in a wall, from `sill` above the wall's base up `height`. host must
    /// be a Wall.
    Window {
        host: ElementId,
        offset: f32,
        width: f32,
        height: f32,
        sill: f32,
        /// Whether it opens: Some(false) for fixed glazing (cannot be an escape opening),
        /// None when not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operable: Option<bool>,
    },
    /// IfcStair: straight run along +X from base (along +Z when `along_z`); risers are equal
    /// height. `base` is the low corner of its footprint either way.
    Stair {
        base: Vec3,
        run_length: f32,
        total_rise: f32,
        width: f32,
        riser_count: u32,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        along_z: bool,
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

impl ElementKind {
    /// For a door or window: its host wall and (offset, width, height, sill).
    pub fn opening(&self) -> Option<(ElementId, (f32, f32, f32, f32))> {
        match *self {
            ElementKind::Door {
                host,
                offset,
                width,
                height,
                ..
            } => Some((host, (offset, width, height, 0.0))),
            ElementKind::Window {
                host,
                offset,
                width,
                height,
                sill,
                ..
            } => Some((host, (offset, width, height, sill))),
            _ => None,
        }
    }
}

fn one() -> u32 {
    1
}
fn is_one(n: &u32) -> bool {
    *n == 1
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
    /// Storey the element belongs to (None: site / unassigned).
    #[serde(default)]
    pub level: Option<LevelId>,
    /// Element-level predecessors: each must finish before this element starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<ElementId>,
    /// Task duration in days; None = appears at its start with no duration of its own (legacy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_days: Option<u32>,
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
            level: None,
            depends_on: vec![],
            duration_days: None,
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
    /// Not occupied and not walked through: roofs, voids ("open to below"), shafts. No
    /// occupant load; left out of the egress analysis.
    Unoccupied,
}

/// What a room is used for, as the residential code sorts rooms: habitable rooms (sleeping,
/// living, dining, kitchen) get light, size and height requirements; sleeping rooms need an
/// emergency escape opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoomUse {
    Sleeping,
    Living,
    Dining,
    Kitchen,
    Bathroom,
    Circulation,
    Storage,
    Utility,
    Garage,
    Other,
}

impl RoomUse {
    /// Living, sleeping, eating or cooking.
    pub fn habitable(self) -> bool {
        matches!(
            self,
            RoomUse::Sleeping | RoomUse::Living | RoomUse::Dining | RoomUse::Kitchen
        )
    }
    /// A guess from a room name ("Bedroom 2", "Kitchen", "Hall"); Other when nothing fits.
    pub fn from_name(name: &str) -> RoomUse {
        let n = format!(" {} ", name.to_lowercase());
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        if has(&["bed", "sleep", "nursery", "guest room"]) {
            RoomUse::Sleeping
        } else if has(&["kitchen"]) {
            RoomUse::Kitchen
        } else if has(&["dining"]) {
            RoomUse::Dining
        } else if has(&["bath", " wc", "toilet", "shower", "powder", "lavatory"]) {
            RoomUse::Bathroom
        } else if has(&["garage", "carport"]) {
            RoomUse::Garage
        } else if has(&[
            "closet", "storage", " store", "pantry", "cupboard", "wardrobe", "linen",
        ]) {
            RoomUse::Storage
        } else if has(&[
            "laundry",
            "utility",
            "mech",
            "boiler",
            "furnace",
            "electrical",
        ]) {
            RoomUse::Utility
        } else if has(&[
            "hall",
            "corridor",
            "foyer",
            "entry",
            "vestibule",
            "landing",
            "stair",
            "lobby",
            "passage",
        ]) {
            RoomUse::Circulation
        } else if has(&[
            "living",
            "family",
            "lounge",
            "den ",
            "study",
            "office",
            "great room",
            "sitting",
            "library",
            "rec room",
        ]) {
            RoomUse::Living
        } else {
            RoomUse::Other
        }
    }
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
    #[serde(default)]
    pub level: Option<LevelId>,
    /// Room outline in plan, [x, z] corners in order (counter-clockwise preferred). When set it
    /// is the room's shape: area, travel distance and door connections use it, and `bounds`
    /// is only its bounding box. Rectangles drawn in the editor use `bounds` alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<[f32; 2]>>,
    /// The dwelling unit (apartment) this space belongs to, by name: spaces with the same
    /// name form one unit. Doors and stairs entirely within a unit get the code's dwelling-unit
    /// exceptions (Residential only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dwelling_unit: Option<String>,
    /// What the room is used for; None = guessed from its name (`Space::room_use`).
    #[serde(default, rename = "room_use", skip_serializing_if = "Option::is_none")]
    pub use_: Option<RoomUse>,
    /// Clear height, floor to ceiling, when known (e.g. from an IFC space's geometry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f32>,
}

impl Space {
    pub fn new(name: impl Into<String>, occupancy: Occupancy, floor_area_m2: f32) -> Self {
        Self {
            name: name.into(),
            occupancy,
            floor_area_m2,
            egress_doors: vec![],
            bounds: None,
            level: None,
            outline: None,
            dwelling_unit: None,
            use_: None,
            height_m: None,
        }
    }
    /// The room's use: as set, else guessed from its name.
    pub fn room_use(&self) -> RoomUse {
        self.use_.unwrap_or_else(|| RoomUse::from_name(&self.name))
    }
    /// Area of the outline, else of `bounds`, else the stated area.
    pub fn area_m2(&self) -> f32 {
        if let Some(o) = self.outline.as_deref().filter(|o| o.len() >= 3) {
            return crate::plan::signed_area(o).abs();
        }
        self.bounds
            .map(|[_, _, sx, sz]| sx * sz)
            .unwrap_or(self.floor_area_m2)
    }
    /// The room's plan shape: the outline, or the `bounds` rectangle; None when the space
    /// has only an area.
    pub fn polygon(&self) -> Option<Vec<[f32; 2]>> {
        if let Some(o) = self.outline.as_ref().filter(|o| o.len() >= 3) {
            return Some(o.clone());
        }
        self.bounds
            .map(|[x, z, sx, sz]| vec![[x, z], [x + sx, z], [x + sx, z + sz], [x, z + sz]])
    }
}
