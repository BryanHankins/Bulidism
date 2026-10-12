//! IFC (STEP; IFC2x3 and IFC4) -> `bim_core::Project`.
//!
//! The model is parametric and simpler than IFC, so geometry is *fitted*: every
//! element's body is reduced to world-space points (extrusions, B-reps,
//! triangulated/polygonal face sets, mapped items, clipping results), and the
//! matching parametric kind is fitted to them: walls and beams get the
//! minimum-area rectangle of their plan footprint (axis, length, thickness),
//! slabs, columns, stairs, ramps and spaces their plan bounding box. Doors are
//! placed on the wall they fill (IfcRelFillsElement/IfcRelVoidsElement).
//! Anything without a matching kind is counted and reported, never silently lost.
pub mod step;

use bim_core::*;
use glam::{DAffine3, DMat3, DVec2, DVec3, Vec3};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use step::{Entity, StepFile, Value};

#[derive(Debug, thiserror::Error)]
pub enum IfcError {
    #[error(transparent)]
    Step(#[from] step::StepError),
    #[error("no IfcProject in the file")]
    NoProject,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub project: Project,
    /// What was imported, skipped and assumed, for the user to read.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Slab,
    Column,
    Beam,
    Wall,
    Door,
    Stair,
    Ramp,
    Space,
}

fn kind_of(ty: &str) -> Option<Kind> {
    Some(match ty {
        "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCWALLELEMENTEDCASE" | "IFCCURTAINWALL" => Kind::Wall,
        "IFCSLAB" | "IFCSLABSTANDARDCASE" | "IFCSLABELEMENTEDCASE" => Kind::Slab,
        "IFCCOLUMN" | "IFCCOLUMNSTANDARDCASE" => Kind::Column,
        "IFCBEAM" | "IFCBEAMSTANDARDCASE" => Kind::Beam,
        "IFCDOOR" | "IFCDOORSTANDARDCASE" => Kind::Door,
        "IFCSTAIRFLIGHT" | "IFCSTAIR" => Kind::Stair,
        "IFCRAMPFLIGHT" | "IFCRAMP" => Kind::Ramp,
        "IFCSPACE" => Kind::Space,
        _ => return None,
    })
}

/// Spatial structure and helper objects: never reported as skipped.
const NOT_ELEMENTS: &[&str] = &[
    "IFCSITE",
    "IFCBUILDING",
    "IFCBUILDINGSTOREY",
    "IFCOPENINGELEMENT",
    "IFCOPENINGSTANDARDCASE",
    "IFCVIRTUALELEMENT",
    "IFCANNOTATION",
    "IFCGRID",
];

/// IFC compressed GlobalId (22 chars, base-64 with a custom alphabet) -> UUID.
pub fn guid_to_uuid(g: &str) -> Option<uuid::Uuid> {
    const B64: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";
    if g.len() != 22 {
        return None;
    }
    let mut n: u128 = 0;
    for (i, c) in g.bytes().enumerate() {
        let d = B64.iter().position(|&b| b == c)? as u128;
        if i == 0 && d > 3 {
            return None;
        }
        n = (n << 6) | d;
    }
    Some(uuid::Uuid::from_u128(n))
}

fn frame(o: DVec3, z: DVec3, xref: DVec3) -> DAffine3 {
    let z = z.try_normalize().unwrap_or(DVec3::Z);
    let x = (xref - z * xref.dot(z))
        .try_normalize()
        .unwrap_or_else(|| z.any_orthonormal_vector());
    DAffine3::from_mat3_translation(DMat3::from_cols(x, z.cross(x), z), o)
}

struct Reader<'a> {
    f: &'a StepFile,
    placements: HashMap<u64, DAffine3>,
    unsupported: BTreeMap<String, usize>,
}

impl<'a> Reader<'a> {
    fn ent(&self, v: &Value) -> Option<&'a Entity> {
        v.id().and_then(|i| self.f.entities.get(&i))
    }
    fn nums(v: &Value) -> Vec<f64> {
        v.list().iter().filter_map(Value::num).collect()
    }
    fn point(&self, v: &Value) -> Option<DVec3> {
        let e = self.ent(v).filter(|e| e.ty == "IFCCARTESIANPOINT")?;
        let c = Self::nums(e.arg(0));
        Some(DVec3::new(
            *c.first()?,
            *c.get(1).unwrap_or(&0.0),
            *c.get(2).unwrap_or(&0.0),
        ))
    }
    fn dir(&self, v: &Value) -> Option<DVec3> {
        let e = self.ent(v).filter(|e| e.ty == "IFCDIRECTION")?;
        let c = Self::nums(e.arg(0));
        DVec3::new(
            *c.first()?,
            *c.get(1).unwrap_or(&0.0),
            *c.get(2).unwrap_or(&0.0),
        )
        .try_normalize()
    }
    fn axis_placement(&self, v: &Value) -> DAffine3 {
        let Some(e) = self.ent(v) else {
            return DAffine3::IDENTITY;
        };
        let o = self.point(e.arg(0)).unwrap_or(DVec3::ZERO);
        match e.ty.as_str() {
            "IFCAXIS2PLACEMENT3D" => frame(
                o,
                self.dir(e.arg(1)).unwrap_or(DVec3::Z),
                self.dir(e.arg(2)).unwrap_or(DVec3::X),
            ),
            "IFCAXIS2PLACEMENT2D" => frame(o, DVec3::Z, self.dir(e.arg(1)).unwrap_or(DVec3::X)),
            _ => DAffine3::IDENTITY,
        }
    }
    /// IfcLocalPlacement chain -> world transform (memoised; cycles cut off).
    fn object_placement(&mut self, v: &Value, depth: u32) -> DAffine3 {
        let Some(id) = v.id() else {
            return DAffine3::IDENTITY;
        };
        if let Some(t) = self.placements.get(&id) {
            return *t;
        }
        let t = match self.ent(v) {
            Some(e) if e.ty == "IFCLOCALPLACEMENT" && depth < 64 => {
                let parent = self.object_placement(e.arg(0), depth + 1);
                parent * self.axis_placement(e.arg(1))
            }
            Some(e) if e.ty != "IFCLOCALPLACEMENT" => {
                *self.unsupported.entry(e.ty.clone()).or_default() += 1;
                DAffine3::IDENTITY
            }
            _ => DAffine3::IDENTITY,
        };
        self.placements.insert(id, t);
        t
    }
    /// IfcCartesianTransformationOperator(2D|3D)(NonUniform).
    fn cart_op(&self, v: &Value) -> DAffine3 {
        let Some(e) = self.ent(v) else {
            return DAffine3::IDENTITY;
        };
        let three = e.ty.contains("3D");
        let o = self.point(e.arg(2)).unwrap_or(DVec3::ZERO);
        let x = self.dir(e.arg(0)).unwrap_or(DVec3::X);
        let z = if three {
            self.dir(e.arg(4)).unwrap_or(DVec3::Z)
        } else {
            DVec3::Z
        };
        let s = e.arg(3).num().unwrap_or(1.0);
        let (s2, s3) = if e.ty.contains("NONUNIFORM") {
            let i = if three { 5 } else { 4 };
            (e.arg(i).num().unwrap_or(s), e.arg(i + 1).num().unwrap_or(s))
        } else {
            (s, s)
        };
        let f = frame(o, z, x);
        DAffine3::from_mat3_translation(
            DMat3::from_cols(
                f.matrix3.x_axis * s,
                f.matrix3.y_axis * s2,
                f.matrix3.z_axis * s3,
            ),
            o,
        )
    }
    fn skip(&mut self, ty: &str) {
        *self.unsupported.entry(ty.to_string()).or_default() += 1;
    }

    /// Outline points of a profile in its own 2D plane.
    fn profile(&mut self, v: &Value) -> Vec<DVec2> {
        let Some(e) = self.ent(v) else {
            return vec![];
        };
        let rect = |r: &Self, w: Option<f64>, h: Option<f64>| -> Vec<DVec2> {
            let (Some(w), Some(h)) = (w, h) else {
                return vec![];
            };
            let p = r.axis_placement(e.arg(2));
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|(sx, sy)| {
                    p.transform_point3(DVec3::new(sx * w / 2.0, sy * h / 2.0, 0.0))
                        .truncate()
                })
                .collect()
        };
        match e.ty.as_str() {
            "IFCRECTANGLEPROFILEDEF"
            | "IFCROUNDEDRECTANGLEPROFILEDEF"
            | "IFCRECTANGLEHOLLOWPROFILEDEF"
            | "IFCISHAPEPROFILEDEF"
            | "IFCASYMMETRICISHAPEPROFILEDEF" => rect(self, e.arg(3).num(), e.arg(4).num()),
            // Depth first, then flange width
            "IFCLSHAPEPROFILEDEF"
            | "IFCUSHAPEPROFILEDEF"
            | "IFCCSHAPEPROFILEDEF"
            | "IFCTSHAPEPROFILEDEF"
            | "IFCZSHAPEPROFILEDEF" => {
                rect(self, e.arg(4).num().or(e.arg(3).num()), e.arg(3).num())
            }
            "IFCCIRCLEPROFILEDEF" | "IFCCIRCLEHOLLOWPROFILEDEF" => {
                let d = e.arg(3).num().map(|r| 2.0 * r);
                rect(self, d, d)
            }
            "IFCELLIPSEPROFILEDEF" => rect(
                self,
                e.arg(3).num().map(|a| 2.0 * a),
                e.arg(4).num().map(|b| 2.0 * b),
            ),
            "IFCARBITRARYCLOSEDPROFILEDEF" | "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                self.curve(e.arg(2))
            }
            "IFCDERIVEDPROFILEDEF" => {
                let op = self.cart_op(e.arg(3));
                self.profile(e.arg(2))
                    .into_iter()
                    .map(|p| op.transform_point3(p.extend(0.0)).truncate())
                    .collect()
            }
            t => {
                self.skip(t);
                vec![]
            }
        }
    }

    /// Points along a 2D curve (polylines exactly; arcs by their extreme points).
    fn curve(&mut self, v: &Value) -> Vec<DVec2> {
        let Some(e) = self.ent(v) else {
            return vec![];
        };
        match e.ty.as_str() {
            "IFCPOLYLINE" => e
                .arg(0)
                .list()
                .iter()
                .filter_map(|p| self.point(p))
                .map(|p| p.truncate())
                .collect(),
            "IFCINDEXEDPOLYCURVE" => self
                .ent(e.arg(0))
                .map(|pl| {
                    pl.arg(0)
                        .list()
                        .iter()
                        .filter_map(|c| {
                            let c = Self::nums(c);
                            Some(DVec2::new(*c.first()?, *c.get(1)?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            "IFCCOMPOSITECURVE" => {
                let segs: Vec<&Entity> =
                    e.arg(0).list().iter().filter_map(|s| self.ent(s)).collect();
                segs.into_iter()
                    .flat_map(|s| self.curve(s.arg(2)))
                    .collect()
            }
            "IFCTRIMMEDCURVE" => {
                let mut pts = self.curve(e.arg(0));
                for t in [e.arg(1), e.arg(2)] {
                    pts.extend(
                        t.list()
                            .iter()
                            .filter_map(|p| self.point(p))
                            .map(|p| p.truncate()),
                    );
                }
                pts
            }
            "IFCCIRCLE" | "IFCELLIPSE" => {
                let p = self.axis_placement(e.arg(0));
                let a = e.arg(1).num().unwrap_or(0.0);
                let b = e.arg(2).num().unwrap_or(a);
                [(a, 0.0), (0.0, b), (-a, 0.0), (0.0, -b)]
                    .iter()
                    .map(|(x, y)| p.transform_point3(DVec3::new(*x, *y, 0.0)).truncate())
                    .collect()
            }
            "IFCLINE" => vec![],
            t => {
                self.skip(t);
                vec![]
            }
        }
    }

    /// World-space points (IFC axes and units) of one representation item.
    fn item(&mut self, v: &Value, xf: DAffine3, out: &mut Vec<DVec3>, depth: u32) {
        let Some(e) = self.ent(v) else {
            return;
        };
        if depth > 32 {
            return;
        }
        match e.ty.as_str() {
            "IFCEXTRUDEDAREASOLID" | "IFCEXTRUDEDAREASOLIDTAPERED" => {
                let prof = self.profile(e.arg(0));
                let pos = xf * self.axis_placement(e.arg(1));
                let d = self.dir(e.arg(2)).unwrap_or(DVec3::Z) * e.arg(3).num().unwrap_or(0.0);
                for p in prof {
                    let b = p.extend(0.0);
                    out.push(pos.transform_point3(b));
                    out.push(pos.transform_point3(b + d));
                }
            }
            "IFCBOOLEANCLIPPINGRESULT" | "IFCBOOLEANRESULT" => {
                self.item(e.arg(1), xf, out, depth + 1);
                if e.arg(0).enumv() == Some("UNION") {
                    self.item(e.arg(2), xf, out, depth + 1);
                }
            }
            "IFCFACETEDBREP" | "IFCFACETEDBREPWITHVOIDS" | "IFCADVANCEDBREP" | "IFCCSGSOLID" => {
                self.item(e.arg(0), xf, out, depth + 1)
            }
            "IFCCLOSEDSHELL" | "IFCOPENSHELL" | "IFCCONNECTEDFACESET" => {
                for face in e.arg(0).list().iter().filter_map(|f| self.ent(f)) {
                    for bound in face.arg(0).list().iter().filter_map(|b| self.ent(b)) {
                        if let Some(lp) = self.ent(bound.arg(0)).filter(|l| l.ty == "IFCPOLYLOOP") {
                            out.extend(
                                lp.arg(0)
                                    .list()
                                    .iter()
                                    .filter_map(|p| self.point(p))
                                    .map(|p| xf.transform_point3(p)),
                            );
                        }
                    }
                }
            }
            "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => {
                for s in e.arg(0).list() {
                    self.item(s, xf, out, depth + 1);
                }
            }
            "IFCTRIANGULATEDFACESET"
            | "IFCPOLYGONALFACESET"
            | "IFCTRIANGULATEDIRREGULARNETWORK" => {
                if let Some(pl) = self.ent(e.arg(0)) {
                    out.extend(pl.arg(0).list().iter().filter_map(|c| {
                        let c = Self::nums(c);
                        Some(xf.transform_point3(DVec3::new(*c.first()?, *c.get(1)?, *c.get(2)?)))
                    }));
                }
            }
            "IFCMAPPEDITEM" => {
                let Some(src) = self.ent(e.arg(0)) else {
                    return;
                };
                let t = xf * self.cart_op(e.arg(1)) * self.axis_placement(src.arg(0));
                if let Some(rep) = self.ent(src.arg(1)) {
                    for it in rep.arg(3).list() {
                        self.item(it, t, out, depth + 1);
                    }
                }
            }
            "IFCBOUNDINGBOX" | "IFCBLOCK" => {
                let (o, i) = if e.ty == "IFCBLOCK" {
                    (self.axis_placement(e.arg(0)), 1)
                } else {
                    (
                        DAffine3::from_translation(self.point(e.arg(0)).unwrap_or(DVec3::ZERO)),
                        1,
                    )
                };
                let s = DVec3::new(
                    e.arg(i).num().unwrap_or(0.0),
                    e.arg(i + 1).num().unwrap_or(0.0),
                    e.arg(i + 2).num().unwrap_or(0.0),
                );
                for c in 0..8 {
                    let p =
                        DVec3::new((c & 1) as f64, ((c >> 1) & 1) as f64, ((c >> 2) & 1) as f64)
                            * s;
                    out.push(xf.transform_point3(o.transform_point3(p)));
                }
            }
            t => self.skip(t),
        }
    }

    /// World points of a product's body representation.
    fn product_points(&mut self, e: &Entity) -> Vec<DVec3> {
        let xf = self.object_placement(e.arg(5), 0);
        let mut out = Vec::new();
        let Some(pds) = self.ent(e.arg(6)) else {
            return out;
        };
        let reps: Vec<&Entity> = pds
            .arg(2)
            .list()
            .iter()
            .filter_map(|r| self.ent(r))
            .filter(|r| r.ty == "IFCSHAPEREPRESENTATION")
            .collect();
        let body = reps
            .iter()
            .find(|r| r.arg(1).str() == Some("Body"))
            .or_else(|| {
                reps.iter().find(|r| {
                    !matches!(
                        r.arg(1).str(),
                        Some("Axis" | "FootPrint" | "Annotation" | "Profile" | "Clearance")
                    )
                })
            });
        if let Some(body) = body {
            for it in body.arg(3).list() {
                self.item(it, xf, &mut out, 0);
            }
        }
        out
    }
}

/// Minimum-area rectangle of plan points: centre, unit long axis, half length, half width.
fn min_rect(pts: &[DVec2]) -> Option<(DVec2, DVec2, f64, f64)> {
    let mut p: Vec<DVec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup_by(|a, b| a.distance(*b) < 1e-9);
    if p.len() < 2 {
        return None;
    }
    // Andrew's monotone chain (collinear input collapses to its two end points)
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut h: Vec<DVec2> = Vec::with_capacity(2 * p.len());
    for &q in &p {
        while h.len() >= 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
            h.pop();
        }
        h.push(q);
    }
    let lower = h.len() + 1;
    for &q in p.iter().rev().skip(1) {
        while h.len() >= lower && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
            h.pop();
        }
        h.push(q);
    }
    h.pop();
    let mut best: Option<(f64, DVec2, DVec2, f64, f64)> = None;
    for i in 0..h.len() {
        let Some(u) = (h[(i + 1) % h.len()] - h[i]).try_normalize() else {
            continue;
        };
        let v = u.perp();
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for q in &h {
            let (a, b) = (q.dot(u), q.dot(v));
            (u0, u1, v0, v1) = (u0.min(a), u1.max(a), v0.min(b), v1.max(b));
        }
        let area = (u1 - u0) * (v1 - v0);
        if !matches!(best, Some(b) if area >= b.0 - 1e-12) {
            let c = u * (u0 + u1) / 2.0 + v * (v0 + v1) / 2.0;
            best = Some((area, c, u, (u1 - u0) / 2.0, (v1 - v0) / 2.0));
        }
    }
    let (_, c, mut u, mut a, mut b) = best?;
    if a < b {
        u = u.perp();
        std::mem::swap(&mut a, &mut b);
    }
    if u.x < -1e-9 || (u.x.abs() <= 1e-9 && u.y < 0.0) {
        u = -u;
    }
    Some((c, u, a, b))
}

struct Bounds {
    min: Vec3,
    max: Vec3,
}
fn bounds(p: &[Vec3]) -> Bounds {
    let mut b = Bounds {
        min: Vec3::splat(f32::MAX),
        max: Vec3::splat(f32::MIN),
    };
    for q in p {
        b.min = b.min.min(*q);
        b.max = b.max.max(*q);
    }
    b
}

fn occupancy_from(s: &str) -> Option<Occupancy> {
    let t = s.trim().to_ascii_uppercase();
    let first = t.chars().next()?;
    if !(t.len() == 1 || t.as_bytes().get(1) == Some(&b'-')) {
        return None;
    }
    Some(match first {
        'A' => Occupancy::Assembly,
        'B' => Occupancy::Business,
        'E' => Occupancy::Educational,
        'M' => Occupancy::Mercantile,
        'R' => Occupancy::Residential,
        'S' => Occupancy::Storage,
        'I' => Occupancy::Institutional,
        _ => return None,
    })
}

const STRUCTURE: PhaseId = PhaseId(0);
const ENVELOPE: PhaseId = PhaseId(1);
const FIT_OUT: PhaseId = PhaseId(2);

/// Import an IFC file. `fallback_name` names the project if IfcProject has no name.
pub fn import(text: &str, fallback_name: &str) -> Result<ImportResult, IfcError> {
    let f = step::parse(text)?;
    let mut r = Reader {
        f: &f,
        placements: HashMap::new(),
        unsupported: BTreeMap::new(),
    };
    let mut ids: Vec<u64> = f.entities.keys().copied().collect();
    ids.sort_unstable();
    let proj = ids
        .iter()
        .map(|i| &f.entities[i])
        .find(|e| e.ty == "IFCPROJECT")
        .ok_or(IfcError::NoProject)?;

    // ---- units: length scale to metres, area scale to m²
    let (mut len_scale, mut area_scale) = (1.0, 1.0);
    let prefix = |p: Option<&str>| -> f64 {
        match p {
            Some("MILLI") => 1e-3,
            Some("CENTI") => 1e-2,
            Some("DECI") => 1e-1,
            Some("KILO") => 1e3,
            _ => 1.0,
        }
    };
    if let Some(ua) = r.ent(proj.arg(8)) {
        for u in ua.arg(0).list().iter().filter_map(|u| r.ent(u)) {
            let kind = u.arg(1).enumv();
            match (u.ty.as_str(), kind) {
                ("IFCSIUNIT", Some("LENGTHUNIT")) => len_scale = prefix(u.arg(2).enumv()),
                ("IFCSIUNIT", Some("AREAUNIT")) => area_scale = prefix(u.arg(2).enumv()).powi(2),
                ("IFCCONVERSIONBASEDUNIT", Some(k @ ("LENGTHUNIT" | "AREAUNIT"))) => {
                    let named = match u.arg(2).str().map(|s| s.to_ascii_uppercase()).as_deref() {
                        Some("FOOT") => Some(0.3048),
                        Some("INCH") => Some(0.0254),
                        Some("SQUARE FOOT") => Some(0.09290304),
                        _ => None,
                    };
                    let measured = r.ent(u.arg(3)).and_then(|m| {
                        let v = m.arg(0).num()?;
                        let si = r
                            .ent(m.arg(1))
                            .map(|s| prefix(s.arg(2).enumv()))
                            .unwrap_or(1.0);
                        Some(v * if k == "AREAUNIT" { si * si } else { si })
                    });
                    let s = named.or(measured).unwrap_or(1.0);
                    if k == "LENGTHUNIT" {
                        len_scale = s
                    } else {
                        area_scale = s
                    }
                }
                _ => {}
            }
        }
    }
    // IFC is Z-up; the model is Y-up: (x, y, z) -> (x, z, -y), in metres
    let to_model = |p: DVec3| {
        Vec3::new(
            (p.x * len_scale) as f32,
            (p.z * len_scale) as f32,
            (-p.y * len_scale) as f32,
        )
    };

    // ---- relationships
    let mut void_host: HashMap<u64, u64> = HashMap::new(); // opening -> host element
    let mut fill_opening: HashMap<u64, u64> = HashMap::new(); // door -> opening
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut props: HashMap<u64, Vec<(String, Value)>> = HashMap::new();
    let mut quants: HashMap<u64, Vec<(String, f64)>> = HashMap::new();
    let mut bounded_by: HashMap<u64, Vec<u64>> = HashMap::new(); // space -> elements
    for &id in &ids {
        let e = &f.entities[&id];
        match e.ty.as_str() {
            "IFCRELVOIDSELEMENT" => {
                if let (Some(h), Some(o)) = (e.arg(4).id(), e.arg(5).id()) {
                    void_host.insert(o, h);
                }
            }
            "IFCRELFILLSELEMENT" => {
                if let (Some(o), Some(d)) = (e.arg(4).id(), e.arg(5).id()) {
                    fill_opening.insert(d, o);
                }
            }
            "IFCRELAGGREGATES" | "IFCRELNESTS" => {
                if let Some(p) = e.arg(4).id() {
                    children
                        .entry(p)
                        .or_default()
                        .extend(e.arg(5).list().iter().filter_map(Value::id));
                }
            }
            "IFCRELSPACEBOUNDARY"
            | "IFCRELSPACEBOUNDARY1STLEVEL"
            | "IFCRELSPACEBOUNDARY2NDLEVEL" => {
                if let (Some(s), Some(el)) = (e.arg(4).id(), e.arg(5).id()) {
                    bounded_by.entry(s).or_default().push(el);
                }
            }
            "IFCRELDEFINESBYPROPERTIES" => {
                let Some(def) = r.ent(e.arg(5)) else { continue };
                let objs: Vec<u64> = e.arg(4).list().iter().filter_map(Value::id).collect();
                match def.ty.as_str() {
                    "IFCPROPERTYSET" => {
                        for p in def
                            .arg(4)
                            .list()
                            .iter()
                            .filter_map(|p| r.ent(p))
                            .filter(|p| p.ty == "IFCPROPERTYSINGLEVALUE")
                        {
                            if let Some(n) = p.arg(0).str() {
                                for o in &objs {
                                    props
                                        .entry(*o)
                                        .or_default()
                                        .push((n.to_string(), p.arg(2).clone()));
                                }
                            }
                        }
                    }
                    "IFCELEMENTQUANTITY" => {
                        for q in def.arg(5).list().iter().filter_map(|q| r.ent(q)) {
                            let scale = match q.ty.as_str() {
                                "IFCQUANTITYAREA" => area_scale,
                                "IFCQUANTITYLENGTH" => len_scale,
                                _ => continue,
                            };
                            if let (Some(n), Some(v)) = (q.arg(0).str(), q.arg(3).num()) {
                                for o in &objs {
                                    quants
                                        .entry(*o)
                                        .or_default()
                                        .push((n.to_string(), v * scale));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let prop = |id: u64, name: &str| {
        props.get(&id).and_then(|v| {
            v.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })
    };
    let quant = |id: u64, name: &str| {
        quants.get(&id).and_then(|v| {
            v.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| *v)
        })
    };

    // ---- gather products with geometry
    let mut found: Vec<(Kind, u64, Vec<Vec3>)> = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_geometry: BTreeMap<String, usize> = BTreeMap::new();
    for &id in &ids {
        let e = &f.entities[&id];
        let Some(kind) = kind_of(&e.ty) else {
            let product = e.arg(0).str().is_some_and(|g| g.len() == 22)
                && r.ent(e.arg(5)).is_some_and(|p| p.ty.ends_with("PLACEMENT"));
            if product && r.ent(e.arg(6)).is_some() && !NOT_ELEMENTS.contains(&e.ty.as_str()) {
                *skipped.entry(e.ty.clone()).or_default() += 1;
            }
            continue;
        };
        // containers (stair, ramp) whose flights are imported themselves
        let kids = children.get(&id).map(Vec::as_slice).unwrap_or(&[]);
        if matches!(e.ty.as_str(), "IFCSTAIR" | "IFCRAMP")
            && kids.iter().any(|k| {
                matches!(
                    f.entities.get(k).map(|c| c.ty.as_str()),
                    Some("IFCSTAIRFLIGHT" | "IFCRAMPFLIGHT")
                )
            })
        {
            continue;
        }
        let mut pts = r.product_points(e);
        if pts.is_empty() {
            // assemblies (curtain walls, stairs) carry geometry on their parts
            for k in kids {
                if let Some(c) = f.entities.get(k) {
                    pts.extend(r.product_points(c));
                }
            }
        }
        if pts.is_empty() {
            *no_geometry.entry(e.ty.clone()).or_default() += 1;
            continue;
        }
        found.push((kind, id, pts.into_iter().map(to_model).collect()));
    }

    // ---- re-centre: plan origin at the model's corner; keep elevations unless far from 0
    let all = bounds(
        &found
            .iter()
            .flat_map(|(_, _, p)| p.iter().copied())
            .collect::<Vec<_>>(),
    );
    let mut notes = Vec::new();
    let mut shift = Vec3::new(-all.min.x, 0.0, -all.min.z);
    if found.is_empty() {
        shift = Vec3::ZERO;
    } else if all.min.y.abs() > 50.0 {
        shift.y = -all.min.y;
        notes.push(format!(
            "Elevations shifted by {:.2} m so the model starts at 0.",
            -all.min.y
        ));
    }
    found.sort_by_key(|(k, id, _)| (*k, *id));

    // ---- build the project
    let name = proj
        .arg(2)
        .str()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_name)
        .to_string();
    let mut p = Project::new(name, "IN-2026");
    p.phases = vec![
        Phase {
            id: STRUCTURE,
            name: "Structure".into(),
            duration_days: 20,
            depends_on: vec![],
        },
        Phase {
            id: ENVELOPE,
            name: "Envelope".into(),
            duration_days: 15,
            depends_on: vec![STRUCTURE],
        },
        Phase {
            id: FIT_OUT,
            name: "Fit-out".into(),
            duration_days: 20,
            depends_on: vec![ENVELOPE],
        },
    ];
    let mut used_ids: HashSet<ElementId> = HashSet::new();
    let mut walls: HashMap<u64, (ElementId, Vec3, Vec3, f32, f32)> = HashMap::new(); // start, end, height, thickness
    let mut doors: HashMap<u64, (ElementId, bool)> = HashMap::new();
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    let mut space_src: Vec<u64> = Vec::new(); // IFC id of each p.spaces entry
    let (mut defaulted_occupancy, mut stairs, mut degenerate, mut sloped) = (0, 0, 0, 0);

    for (kind, id, pts) in &found {
        let e = &f.entities[id];
        let pts: Vec<Vec3> = pts.iter().map(|q| *q + shift).collect();
        let b = bounds(&pts);
        let plan: Vec<DVec2> = pts
            .iter()
            .map(|q| DVec2::new(q.x as f64, q.z as f64))
            .collect();
        let label = e
            .arg(2)
            .str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} #{id}", e.ty));
        let height = b.max.y - b.min.y;
        let (k, disc, phase, code) = match kind {
            Kind::Wall | Kind::Beam => {
                let Some((c, u, a, hw)) = min_rect(&plan) else {
                    degenerate += 1;
                    continue;
                };
                let (s, t) = (c - u * a, c + u * a);
                let y = b.min.y;
                let (start, end) = (
                    Vec3::new(s.x as f32, y, s.y as f32),
                    Vec3::new(t.x as f32, y, t.y as f32),
                );
                if a <= 1e-4 || hw <= 1e-4 || height <= 1e-4 {
                    degenerate += 1;
                    continue;
                }
                if *kind == Kind::Wall {
                    (
                        ElementKind::Wall {
                            start,
                            end,
                            height,
                            thickness: 2.0 * hw as f32,
                        },
                        Discipline::Architectural,
                        ENVELOPE,
                        "B2010",
                    )
                } else {
                    (
                        ElementKind::Beam {
                            start,
                            end,
                            width: 2.0 * hw as f32,
                            depth: height,
                        },
                        Discipline::Structural,
                        STRUCTURE,
                        "B1010",
                    )
                }
            }
            Kind::Column | Kind::Slab | Kind::Stair | Kind::Ramp => {
                let s = b.max - b.min;
                if s.x <= 1e-4 || s.z <= 1e-4 || (s.y <= 1e-4 && *kind != Kind::Ramp) {
                    degenerate += 1;
                    continue;
                }
                match kind {
                    Kind::Column => (
                        ElementKind::Column {
                            base: Vec3::new(
                                (b.min.x + b.max.x) / 2.0,
                                b.min.y,
                                (b.min.z + b.max.z) / 2.0,
                            ),
                            width: s.x,
                            depth: s.z,
                            height: s.y,
                        },
                        Discipline::Structural,
                        STRUCTURE,
                        "B1010",
                    ),
                    Kind::Slab => {
                        // a sloped (roof) slab's box is far taller than the slab is thick: use the
                        // measured depth so volume and cost stay right
                        let depth = quant(*id, "Depth")
                            .or_else(|| quant(*id, "Width"))
                            .map(|d| d as f32)
                            .filter(|d| *d > 0.0);
                        let (thickness, is_sloped) = match depth {
                            Some(d) if s.y > 1.5 * d => {
                                sloped += 1;
                                (d, true)
                            }
                            _ => (s.y, false),
                        };
                        // roof slabs belong to the envelope, priced as roofing
                        let roof = is_sloped
                            || e.arg(8).enumv() == Some("ROOF")
                            || children.iter().any(|(par, ks)| {
                                ks.contains(id)
                                    && f.entities.get(par).is_some_and(|x| x.ty == "IFCROOF")
                            });
                        let slab = ElementKind::Slab {
                            origin: b.min,
                            size_x: s.x,
                            size_z: s.z,
                            thickness,
                        };
                        if roof {
                            (slab, Discipline::Architectural, ENVELOPE, "B3010")
                        } else {
                            (slab, Discipline::Structural, STRUCTURE, "A1010")
                        }
                    }
                    Kind::Stair => {
                        stairs += 1;
                        let risers = e
                            .arg(8)
                            .num()
                            .filter(|n| *n >= 1.0)
                            .map(|n| n as u32)
                            .unwrap_or(((s.y / 0.178).ceil() as u32).max(1));
                        (
                            ElementKind::Stair {
                                base: b.min,
                                run_length: s.x,
                                total_rise: s.y,
                                width: s.z,
                                riser_count: risers,
                            },
                            Discipline::Architectural,
                            FIT_OUT,
                            "C2010",
                        )
                    }
                    _ => (
                        ElementKind::Ramp {
                            base: b.min,
                            run_length: s.x,
                            rise: s.y,
                            width: s.z,
                        },
                        Discipline::Architectural,
                        FIT_OUT,
                        "G2030",
                    ),
                }
            }
            Kind::Door => continue, // after walls
            Kind::Space => {
                let s = b.max - b.min;
                if s.x <= 1e-4 || s.z <= 1e-4 {
                    degenerate += 1;
                    continue;
                }
                let long = e.arg(7).str().filter(|s| !s.is_empty());
                let occ = prop(*id, "OccupancyType")
                    .and_then(|v| v.str())
                    .and_then(occupancy_from);
                if occ.is_none() {
                    defaulted_occupancy += 1;
                }
                let mut sp = Space::new(
                    long.map(str::to_string).unwrap_or(label.clone()),
                    occ.unwrap_or(Occupancy::Business),
                    s.x * s.z,
                );
                sp.bounds = Some([b.min.x, b.min.z, s.x, s.z]);
                // a measured floor area beats the bounding box for non-rectangular rooms
                if let Some(q) = quant(*id, "NetFloorArea")
                    .or_else(|| quant(*id, "GrossFloorArea"))
                    .filter(|q| *q > 0.0)
                {
                    if ((q as f32) - s.x * s.z).abs() > 0.1 * s.x * s.z {
                        sp.bounds = None;
                    }
                    sp.floor_area_m2 = q as f32;
                }
                p.spaces.push(sp);
                space_src.push(*id);
                *count.entry("spaces").or_default() += 1;
                continue;
            }
        };
        let mut el = Element::new(label, k, disc, phase);
        el.cost_code = code.into();
        el.id = unique_id(e, &mut used_ids);
        if let ElementKind::Wall {
            start,
            end,
            height,
            thickness,
        } = el.kind
        {
            walls.insert(*id, (el.id, start, end, height, thickness));
        }
        *count.entry(plural(kind)).or_default() += 1;
        p.add(el);
    }

    // doors: on the wall they fill, else the nearest wall they sit in
    let mut unhosted = 0;
    let mut placed: HashMap<ElementId, Vec<(f32, f32)>> = HashMap::new();
    let mut dropped_overlap = 0;
    for (_, id, pts) in found.iter().filter(|(k, _, _)| *k == Kind::Door) {
        let e = &f.entities[id];
        let pts: Vec<Vec3> = pts.iter().map(|q| *q + shift).collect();
        let b = bounds(&pts);
        let centre = (b.min + b.max) / 2.0;
        let host = fill_opening
            .get(id)
            .and_then(|o| void_host.get(o))
            .and_then(|w| walls.get(w))
            .or_else(|| {
                walls
                    .values()
                    .filter(|(_, s, t, _, th)| dist_to_segment(centre, *s, *t) <= th / 2.0 + 0.3)
                    .min_by(|a, c| {
                        dist_to_segment(centre, a.1, a.2)
                            .total_cmp(&dist_to_segment(centre, c.1, c.2))
                    })
            });
        let Some(&(wall_id, s, t, wall_h, _)) = host else {
            unhosted += 1;
            continue;
        };
        let len = s.distance(t);
        let dir = (t - s) / len;
        let (mut t0, mut t1) = (f32::MAX, f32::MIN);
        for q in &pts {
            let a = (*q - s).dot(dir);
            (t0, t1) = (t0.min(a), t1.max(a));
        }
        let mut width = e
            .arg(9)
            .num()
            .map(|w| (w * len_scale) as f32)
            .filter(|w| *w > 0.0)
            .unwrap_or(t1 - t0)
            .min(len);
        let height = e
            .arg(8)
            .num()
            .map(|h| (h * len_scale) as f32)
            .filter(|h| *h > 0.0)
            .unwrap_or(b.max.y - b.min.y)
            .min(wall_h);
        let offset = ((t0 + t1) / 2.0 - width / 2.0).clamp(0.0, (len - width).max(0.0));
        if width <= 1e-4 || height <= 1e-4 {
            degenerate += 1;
            continue;
        }
        let taken = placed.entry(wall_id).or_default();
        if taken
            .iter()
            .any(|(o, w)| offset < o + w - 1e-4 && *o < offset + width - 1e-4)
        {
            dropped_overlap += 1;
            continue;
        }
        width = width.min(len - offset);
        taken.push((offset, width));
        let egress = matches!(
            prop(*id, "FireExit").and_then(|v| v.enumv()),
            Some("T" | "TRUE")
        );
        let label = e
            .arg(2)
            .str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} #{id}", e.ty));
        let mut el = Element::new(
            label,
            ElementKind::Door {
                host: wall_id,
                offset,
                width,
                height,
                is_egress: egress,
            },
            Discipline::Architectural,
            FIT_OUT,
        );
        el.cost_code = "C1020".into();
        el.id = unique_id(e, &mut used_ids);
        doors.insert(*id, (el.id, egress));
        *count.entry("doors").or_default() += 1;
        p.add(el);
    }
    // egress doors on each space's boundary
    for (sp, id) in p.spaces.iter_mut().zip(space_src) {
        if let Some(els) = bounded_by.get(&id) {
            let mut ex: Vec<ElementId> = els
                .iter()
                .filter_map(|d| doors.get(d))
                .filter(|(_, eg)| *eg)
                .map(|(i, _)| *i)
                .collect();
            ex.sort_by_key(|i| i.0);
            ex.dedup();
            sp.egress_doors = ex;
        }
    }

    // ---- notes
    let summary: Vec<String> = count.iter().map(|(k, n)| format!("{n} {k}")).collect();
    notes.insert(
        0,
        format!(
            "Imported {} from {} ({}).",
            if summary.is_empty() {
                "nothing".to_string()
            } else {
                summary.join(", ")
            },
            if f.schema.is_empty() {
                "IFC"
            } else {
                f.schema.as_str()
            },
            if (len_scale - 1e-3).abs() < 1e-12 {
                "millimetres"
            } else if (len_scale - 1.0).abs() < 1e-12 {
                "metres"
            } else if (len_scale - 0.3048).abs() < 1e-9 {
                "feet"
            } else {
                "converted units"
            }
        ),
    );
    if !skipped.is_empty() {
        notes.push(format!(
            "Not imported (no matching element type yet): {}.",
            skipped
                .iter()
                .map(|(t, n)| format!("{n} {}", pretty(t)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !no_geometry.is_empty() {
        notes.push(format!(
            "Skipped without usable geometry: {}.",
            no_geometry
                .iter()
                .map(|(t, n)| format!("{n} {}", pretty(t)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !r.unsupported.is_empty() {
        notes.push(format!(
            "Unsupported geometry parts ignored: {}.",
            r.unsupported
                .iter()
                .map(|(t, n)| format!("{n} {}", pretty(t)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if degenerate > 0 {
        notes.push(format!(
            "{degenerate} element(s) skipped: zero length, width or height after import."
        ));
    }
    if unhosted > 0 {
        notes.push(format!("{unhosted} door(s) skipped: no host wall found."));
    }
    if dropped_overlap > 0 {
        notes.push(format!(
            "{dropped_overlap} door(s) skipped: they overlap another door in the same wall."
        ));
    }
    if count.contains_key("doors") {
        let eg = doors.values().filter(|(_, e)| *e).count();
        notes.push(format!("{eg} door(s) marked egress from their FireExit property; check egress doors per space in Edit → Spaces."));
    }
    if defaulted_occupancy > 0 {
        notes.push(format!("{defaulted_occupancy} space(s) had no IBC occupancy (OccupancyType) and were set to Business; review in Edit → Spaces."));
    }
    if sloped > 0 {
        notes.push(format!(
            "{sloped} sloped slab(s) imported flat at their measured thickness."
        ));
    }
    if stairs > 0 {
        notes.push(
            "Stairs and ramps are approximated by their bounding box and run along +X.".into(),
        );
    }
    notes.push("Elements are fitted to simple boxes (walls/beams by their plan footprint, others by bounding box); slab and column rotation is not kept. Phases and cost codes are defaults; add unit costs in Edit → Unit costs.".into());
    for err in p.validate().iter().take(10) {
        notes.push(format!("Check: {err}"));
    }
    Ok(ImportResult { project: p, notes })
}

fn unique_id(e: &Entity, used: &mut HashSet<ElementId>) -> ElementId {
    let id = e
        .arg(0)
        .str()
        .and_then(guid_to_uuid)
        .map(ElementId)
        .filter(|i| !used.contains(i))
        .unwrap_or_default();
    used.insert(id);
    id
}

fn dist_to_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let (p, a, b) = (
        glam::vec2(p.x, p.z),
        glam::vec2(a.x, a.z),
        glam::vec2(b.x, b.z),
    );
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn plural(k: &Kind) -> &'static str {
    match k {
        Kind::Wall => "walls",
        Kind::Slab => "slabs",
        Kind::Column => "columns",
        Kind::Beam => "beams",
        Kind::Door => "doors",
        Kind::Stair => "stairs",
        Kind::Ramp => "ramps",
        Kind::Space => "spaces",
    }
}

/// IFCBUILDINGELEMENTPROXY -> IfcBuildingElementProxy (best effort: IFC + title-cased rest).
fn pretty(t: &str) -> String {
    const WORDS: &[&str] = &[
        "BUILDING",
        "ELEMENT",
        "PROXY",
        "WINDOW",
        "ROOF",
        "RAILING",
        "COVERING",
        "FURNISHING",
        "FURNITURE",
        "MEMBER",
        "PLATE",
        "FLOW",
        "SEGMENT",
        "TERMINAL",
        "FITTING",
        "CONTROLLER",
        "DISTRIBUTION",
        "STANDARD",
        "CASE",
        "FOOTING",
        "PILE",
        "CHIMNEY",
        "SHADING",
        "DEVICE",
        "SYSTEM",
        "TRANSPORT",
        "GEOGRAPHIC",
        "CIVIL",
        "ASSEMBLY",
        "PART",
        "FASTENER",
        "DISCRETE",
        "ACCESSORY",
        "OPENING",
        "SPATIAL",
        "ZONE",
        "SPACE",
        "CURTAIN",
        "WALL",
        "SLAB",
        "DOOR",
        "STAIR",
        "RAMP",
        "COLUMN",
        "BEAM",
        "SITE",
        "STOREY",
    ];
    let Some(mut rest) = t.strip_prefix("IFC") else {
        return t.to_string();
    };
    let mut out = String::from("Ifc");
    'outer: while !rest.is_empty() {
        for w in WORDS {
            if let Some(r) = rest.strip_prefix(w) {
                out.push_str(&w[..1]);
                out.push_str(&w[1..].to_ascii_lowercase());
                rest = r;
                continue 'outer;
            }
        }
        out.push_str(&rest[..1]);
        out.push_str(&rest[1..].to_ascii_lowercase());
        break;
    }
    out
}

#[cfg(test)]
mod tests;
