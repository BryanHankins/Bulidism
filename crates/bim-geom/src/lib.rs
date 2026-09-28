//! Parametric element -> triangle mesh, plus AABB clash detection.
//! Walls are oriented boxes split around door openings (no CSG needed);
//! stairs are stepped; ramps are wedges. Units: metres, Y up.
use bim_core::*;
use glam::{vec3, Mat3, Vec3};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn append(&mut self, other: Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend(other.positions);
        self.normals.extend(other.normals);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
    pub fn aabb(&self) -> Option<Aabb> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold(
            Aabb {
                min: first,
                max: first,
            },
            |a, p| Aabb {
                min: a.min.min(*p),
                max: a.max.max(*p),
            },
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}
impl Aabb {
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min.x < o.max.x
            && self.max.x > o.min.x
            && self.min.y < o.max.y
            && self.max.y > o.min.y
            && self.min.z < o.max.z
            && self.max.z > o.min.z
    }
    pub fn volume(&self) -> f32 {
        let d = (self.max - self.min).max(Vec3::ZERO);
        d.x * d.y * d.z
    }
    pub fn intersection(&self, o: &Aabb) -> Aabb {
        Aabb {
            min: self.min.max(o.min),
            max: self.max.min(o.max),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GeomError {
    #[error("element {0:?} has no intrinsic geometry (external mesh asset)")]
    NoGeometry(ElementId),
    #[error("door {0:?} host is not a wall")]
    DoorHostNotWall(ElementId),
    #[error(transparent)]
    Project(#[from] ProjectError),
}

/// Axis-aligned box mesh (12 triangles, flat normals, CCW).
pub fn box_mesh(b: &Aabb) -> Mesh {
    let (n, x) = (b.min, b.max);
    let faces: [(Vec3, [Vec3; 4]); 6] = [
        (
            Vec3::X,
            [
                vec3(x.x, n.y, n.z),
                vec3(x.x, x.y, n.z),
                vec3(x.x, x.y, x.z),
                vec3(x.x, n.y, x.z),
            ],
        ),
        (
            Vec3::NEG_X,
            [
                vec3(n.x, n.y, x.z),
                vec3(n.x, x.y, x.z),
                vec3(n.x, x.y, n.z),
                vec3(n.x, n.y, n.z),
            ],
        ),
        (
            Vec3::Y,
            [
                vec3(n.x, x.y, n.z),
                vec3(n.x, x.y, x.z),
                vec3(x.x, x.y, x.z),
                vec3(x.x, x.y, n.z),
            ],
        ),
        (
            Vec3::NEG_Y,
            [
                vec3(n.x, n.y, x.z),
                vec3(n.x, n.y, n.z),
                vec3(x.x, n.y, n.z),
                vec3(x.x, n.y, x.z),
            ],
        ),
        (
            Vec3::Z,
            [
                vec3(x.x, n.y, x.z),
                vec3(x.x, x.y, x.z),
                vec3(n.x, x.y, x.z),
                vec3(n.x, n.y, x.z),
            ],
        ),
        (
            Vec3::NEG_Z,
            [
                vec3(n.x, n.y, n.z),
                vec3(n.x, x.y, n.z),
                vec3(x.x, x.y, n.z),
                vec3(x.x, n.y, n.z),
            ],
        ),
    ];
    let mut m = Mesh::default();
    for (i, (nrm, quad)) in faces.iter().enumerate() {
        let base = (i * 4) as u32;
        m.positions.extend_from_slice(quad);
        m.normals.extend(std::iter::repeat(*nrm).take(4));
        m.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

/// Box whose local +X axis runs from `start` toward `end` in plan (yaw only),
/// spanning [u0,u1] along that axis, centred laterally with `thickness`, from
/// `y0` to `y1` vertically. Used for walls and beams.
fn oriented_segment(
    start: Vec3,
    end: Vec3,
    u0: f32,
    u1: f32,
    thickness: f32,
    y0: f32,
    y1: f32,
) -> Mesh {
    let d = end - start;
    let yaw = (-d.z).atan2(d.x);
    let rot = Mat3::from_rotation_y(yaw);
    let local = box_mesh(&Aabb {
        min: vec3(u0, y0, -thickness / 2.0),
        max: vec3(u1, y1, thickness / 2.0),
    });
    let origin = vec3(start.x, 0.0, start.z);
    Mesh {
        positions: local.positions.iter().map(|p| rot * *p + origin).collect(),
        normals: local.normals.iter().map(|n| rot * *n).collect(),
        indices: local.indices,
    }
}

/// Wedge along +X: full height at x=run, zero at x=0.
fn wedge_mesh(base: Vec3, run: f32, rise: f32, width: f32) -> Mesh {
    let b = base;
    let p = [
        b,
        b + vec3(run, 0.0, 0.0),
        b + vec3(run, rise, 0.0),
        b + vec3(0.0, 0.0, width),
        b + vec3(run, 0.0, width),
        b + vec3(run, rise, width),
    ];
    let slope_n = vec3(-rise, run, 0.0).normalize_or_zero();
    let mut m = Mesh::default();
    let mut face = |verts: &[Vec3], n: Vec3| {
        let base = m.positions.len() as u32;
        m.positions.extend_from_slice(verts);
        m.normals.extend(std::iter::repeat(n).take(verts.len()));
        for k in 1..verts.len() as u32 - 1 {
            m.indices.extend_from_slice(&[base, base + k, base + k + 1]);
        }
    };
    face(&[p[0], p[2], p[1]], Vec3::NEG_Z);
    face(&[p[3], p[4], p[5]], Vec3::Z);
    face(&[p[0], p[1], p[4], p[3]], Vec3::NEG_Y);
    face(&[p[1], p[2], p[5], p[4]], Vec3::X);
    face(&[p[0], p[3], p[5], p[2]], slope_n);
    m
}

/// World-space bounds without building a mesh (fast path for clash/broadphase).
pub fn aabb(project: &Project, e: &Element) -> Result<Aabb, GeomError> {
    Ok(match &e.kind {
        ElementKind::Wall {
            start,
            end,
            height,
            thickness,
        } => {
            let t = *thickness / 2.0;
            let (lo, hi) = (start.min(*end), start.max(*end));
            Aabb {
                min: vec3(lo.x - t, lo.y, lo.z - t),
                max: vec3(hi.x + t, hi.y + height, hi.z + t),
            }
        }
        ElementKind::Slab {
            origin,
            size_x,
            size_z,
            thickness,
        } => Aabb {
            min: *origin,
            max: *origin + vec3(*size_x, *thickness, *size_z),
        },
        ElementKind::Column {
            base,
            width,
            depth,
            height,
        } => {
            let h = vec3(width / 2.0, 0.0, depth / 2.0);
            Aabb {
                min: *base - h,
                max: *base + h + vec3(0.0, *height, 0.0),
            }
        }
        ElementKind::Beam {
            start,
            end,
            width,
            depth,
        } => {
            let h = vec3(width / 2.0, 0.0, width / 2.0);
            Aabb {
                min: start.min(*end) - h,
                max: start.max(*end) + h + vec3(0.0, *depth, 0.0),
            }
        }
        ElementKind::Stair {
            base,
            run_length,
            total_rise,
            width,
            ..
        } => Aabb {
            min: *base,
            max: *base + vec3(*run_length, *total_rise, *width),
        },
        ElementKind::Ramp {
            base,
            run_length,
            rise,
            width,
        } => Aabb {
            min: *base,
            max: *base + vec3(*run_length, *rise, *width),
        },
        ElementKind::Door {
            host,
            offset,
            width,
            height,
            ..
        } => {
            let hw = project.element(*host)?;
            let ElementKind::Wall {
                start,
                end,
                thickness,
                ..
            } = &hw.kind
            else {
                return Err(GeomError::DoorHostNotWall(e.id));
            };
            let dir = (*end - *start).normalize_or_zero();
            let a = *start + dir * *offset;
            let b = a + dir * *width;
            let t = thickness / 2.0 + 0.01;
            Aabb {
                min: vec3(a.min(b).x - t, a.y, a.min(b).z - t),
                max: vec3(a.max(b).x + t, a.y + height, a.max(b).z + t),
            }
        }
        ElementKind::Mesh { .. } => return Err(GeomError::NoGeometry(e.id)),
    })
}

/// Render mesh. Doors return the leaf panel (thin box in the opening).
pub fn mesh_for(project: &Project, e: &Element) -> Result<Mesh, GeomError> {
    Ok(match &e.kind {
        ElementKind::Wall {
            start,
            end,
            height,
            thickness,
        } => {
            let len = start.distance(*end);
            let y0 = start.y;
            let y1 = y0 + height;
            let mut m = Mesh::default();
            let mut cursor = 0.0;
            for (off, w, h) in project.openings_in(e.id) {
                let (off, w) = (off.clamp(0.0, len), w.min(len - off.clamp(0.0, len)));
                if off > cursor {
                    m.append(oriented_segment(
                        *start, *end, cursor, off, *thickness, y0, y1,
                    ));
                }
                if h < *height {
                    m.append(oriented_segment(
                        *start,
                        *end,
                        off,
                        off + w,
                        *thickness,
                        y0 + h,
                        y1,
                    ));
                }
                cursor = off + w;
            }
            if cursor < len {
                m.append(oriented_segment(
                    *start, *end, cursor, len, *thickness, y0, y1,
                ));
            }
            m
        }
        ElementKind::Beam {
            start,
            end,
            width,
            depth,
        } => {
            let len = start.distance(*end);
            // beams may slope: treat as oriented in plan, spanning y from start.y (approximation for LOD200)
            oriented_segment(*start, *end, 0.0, len, *width, start.y, start.y + depth)
        }
        ElementKind::Stair {
            base,
            run_length,
            total_rise,
            width,
            riser_count,
        } => {
            let n = (*riser_count).max(1);
            let riser = total_rise / n as f32;
            let tread = run_length / n as f32;
            let mut m = Mesh::default();
            for i in 0..n {
                let x0 = base.x + tread * i as f32;
                m.append(box_mesh(&Aabb {
                    min: vec3(x0, base.y, base.z),
                    max: vec3(x0 + tread, base.y + riser * (i + 1) as f32, base.z + width),
                }));
            }
            m
        }
        ElementKind::Ramp {
            base,
            run_length,
            rise,
            width,
        } => wedge_mesh(*base, *run_length, *rise, *width),
        ElementKind::Door {
            host,
            offset,
            width,
            height,
            ..
        } => {
            let hw = project.element(*host)?;
            let ElementKind::Wall {
                start,
                end,
                thickness,
                ..
            } = &hw.kind
            else {
                return Err(GeomError::DoorHostNotWall(e.id));
            };
            oriented_segment(
                *start,
                *end,
                *offset,
                offset + width,
                thickness * 0.2,
                start.y,
                start.y + height,
            )
        }
        ElementKind::Slab { .. } | ElementKind::Column { .. } => box_mesh(&aabb(project, e)?),
        ElementKind::Mesh { .. } => return Err(GeomError::NoGeometry(e.id)),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clash {
    pub a: ElementId,
    pub b: ElementId,
    pub overlap_m3: f32,
}

/// Cross-discipline AABB clashes. Sort-and-sweep on X; doors excluded (they live inside their host).
/// `tolerance_m3`: ignore overlaps smaller than this (touching faces, minor penetration).
pub fn clashes(project: &Project, tolerance_m3: f32) -> Vec<Clash> {
    let mut boxes: Vec<(&Element, Aabb)> = project
        .elements
        .iter()
        .filter(|e| !matches!(e.kind, ElementKind::Door { .. }))
        .filter_map(|e| aabb(project, e).ok().map(|b| (e, b)))
        .collect();
    boxes.sort_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x));
    let mut out = Vec::new();
    for i in 0..boxes.len() {
        let (ea, ba) = boxes[i];
        for &(eb, bb) in &boxes[i + 1..] {
            if bb.min.x >= ba.max.x {
                break;
            }
            if ea.discipline == eb.discipline || !ba.intersects(&bb) {
                continue;
            }
            let v = ba.intersection(&bb).volume();
            if v > tolerance_m3 {
                out.push(Clash {
                    a: ea.id,
                    b: eb.id,
                    overlap_m3: v,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn by_name<'a>(p: &'a Project, n: &str) -> &'a Element {
        p.elements.iter().find(|e| e.name == n).unwrap()
    }
    #[test]
    fn box_has_36_indices() {
        let m = box_mesh(&Aabb {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        });
        assert_eq!(m.indices.len(), 36);
        assert_eq!(m.positions.len(), 24);
    }
    #[test]
    fn wall_with_door_is_three_boxes() {
        let p = Project::sample();
        let m = mesh_for(&p, by_name(&p, "South wall")).unwrap();
        assert_eq!(m.triangle_count(), 36);
        let b = m.aabb().unwrap();
        assert!(
            (b.max.x - 12.0).abs() < 1e-3
                && (b.max.y - 3.2).abs() < 1e-3
                && (b.max.z - 0.125).abs() < 1e-3,
            "{b:?}"
        );
    }
    #[test]
    fn angled_wall_orientation() {
        let mut p = Project::new("x", "IBC-2024");
        p.phases.push(Phase {
            id: PhaseId(0),
            name: "p".into(),
            duration_days: 1,
            depends_on: vec![],
        });
        let e = Element::new(
            "diag",
            ElementKind::Wall {
                start: Vec3::ZERO,
                end: vec3(4.0, 0.0, 4.0),
                height: 2.0,
                thickness: 0.2,
            },
            Discipline::Architectural,
            PhaseId(0),
        );
        let m = mesh_for(&p, &e).unwrap();
        let b = m.aabb().unwrap();
        // corner of the oriented box lands near the far end, not at (4,0,0)
        assert!(
            (b.max.x - (4.0 + 0.1 * std::f32::consts::FRAC_1_SQRT_2)).abs() < 1e-3
                && (b.max.z - (4.0 + 0.1 * std::f32::consts::FRAC_1_SQRT_2)).abs() < 1e-3,
            "{b:?}"
        );
    }
    #[test]
    fn stair_step_count() {
        let p = Project::sample();
        assert_eq!(
            mesh_for(&p, by_name(&p, "Stair to mezzanine"))
                .unwrap()
                .triangle_count(),
            17 * 12
        );
    }
    #[test]
    fn ramp_is_wedge() {
        let m = wedge_mesh(Vec3::ZERO, 2.0, 0.2, 1.0);
        assert_eq!(m.triangle_count(), 8);
        assert!(m.normals.iter().any(|n| n.y > 0.9));
    }
    #[test]
    fn sample_clashes() {
        let p = Project::sample();
        let c = clashes(&p, 0.0);
        // wall vs corner columns (4) + beam vs walls (2) — structural/architectural overlaps by design
        assert!(c.len() >= 4 && c.len() <= 6, "{c:?}");
        assert!(clashes(&p, 1.0).len() < c.len());
    }
    #[test]
    fn mesh_element_has_no_geometry() {
        let p = Project::sample();
        let e = Element::new(
            "ext",
            ElementKind::Mesh {
                asset: "x.glb".into(),
                origin: Vec3::ZERO,
            },
            Discipline::Site,
            PhaseId(0),
        );
        assert!(matches!(aabb(&p, &e), Err(GeomError::NoGeometry(_))));
    }
}
