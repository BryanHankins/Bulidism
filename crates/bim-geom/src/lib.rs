//! Parametric element -> triangle mesh, plus clash detection (BVH broadphase over
//! element AABBs, exact plan-polygon x height narrowphase).
//! Walls are oriented boxes split around door openings (no CSG needed); beams follow
//! start -> end including slope; stairs are stepped; ramps are wedges. Units: metres, Y up.
//! Functions take any `Lookup`: a `Project` for one-off calls, a `ProjectIndex` for passes
//! over every element (hashed host/opening lookups instead of scans).
use bim_core::*;
use glam::{vec3, Mat3, Vec2, Vec3};

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
        m.normals.extend_from_slice(&[*nrm; 4]);
        m.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

/// Box whose local +X axis runs from `start` toward `end` in plan (yaw), spanning
/// [u0,u1] (plan metres) along that axis, centred laterally with `thickness`, from
/// `y0` to `y1` vertically, sheared up by `slope` (rise per plan metre; vertical
/// edges stay plumb). Used for walls (slope 0) and beams.
#[allow(clippy::too_many_arguments)]
fn oriented_segment(
    start: Vec3,
    end: Vec3,
    u0: f32,
    u1: f32,
    thickness: f32,
    y0: f32,
    y1: f32,
    slope: f32,
) -> Mesh {
    let d = end - start;
    let yaw = (-d.z).atan2(d.x);
    let rot = Mat3::from_rotation_y(yaw);
    let local = box_mesh(&Aabb {
        min: vec3(u0, y0, -thickness / 2.0),
        max: vec3(u1, y1, thickness / 2.0),
    });
    let origin = vec3(start.x, 0.0, start.z);
    let (xf, nf) = if slope == 0.0 {
        (rot, rot)
    } else {
        let m = Mat3::from_cols(rot.x_axis + Vec3::Y * slope, rot.y_axis, rot.z_axis);
        (m, m.inverse().transpose())
    };
    Mesh {
        positions: local.positions.iter().map(|p| xf * *p + origin).collect(),
        normals: local
            .normals
            .iter()
            .map(|n| (nf * *n).normalize_or_zero())
            .collect(),
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
        m.normals.resize(m.normals.len() + verts.len(), n);
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
pub fn aabb<L: Lookup + ?Sized>(project: &L, e: &Element) -> Result<Aabb, GeomError> {
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
            let hw = project.get(*host)?;
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
pub fn mesh_for<L: Lookup + ?Sized>(project: &L, e: &Element) -> Result<Mesh, GeomError> {
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
            for &(off, w, h) in project.openings(e.id).iter() {
                let (off, w) = (off.clamp(0.0, len), w.min(len - off.clamp(0.0, len)));
                if off > cursor {
                    m.append(oriented_segment(
                        *start, *end, cursor, off, *thickness, y0, y1, 0.0,
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
                        0.0,
                    ));
                }
                // overlapping openings must never move the cursor backwards over a gap
                cursor = cursor.max(off + w);
            }
            if cursor < len {
                m.append(oriented_segment(
                    *start, *end, cursor, len, *thickness, y0, y1, 0.0,
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
            // plan run with plumb-cut section [y, y + depth] along the slope (matches aabb());
            // a vertical member has no plan direction, so it is drawn as its bounds
            let run = Vec2::new(end.x - start.x, end.z - start.z).length();
            if run < 1e-6 {
                return Ok(box_mesh(&aabb(project, e)?));
            }
            let slope = (end.y - start.y) / run;
            oriented_segment(
                *start,
                *end,
                0.0,
                run,
                *width,
                start.y,
                start.y + depth,
                slope,
            )
        }
        ElementKind::Stair {
            base,
            run_length,
            total_rise,
            width,
            riser_count,
        } => {
            // n risers climb n-1 treads; the top riser lands on the floor above
            // (same convention as the stair geometry rule)
            let n = (*riser_count).max(1);
            let riser = total_rise / n as f32;
            let treads = (n - 1).max(1);
            let tread = run_length / treads as f32;
            let mut m = Mesh::default();
            for i in 0..treads {
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
            let hw = project.get(*host)?;
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
                0.0,
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
    /// Bounds of the actual overlap region (not of the two elements' AABBs).
    pub region: Aabb,
}

/// Element as a vertical prism: convex plan footprint (XZ) plus [y0, y1].
/// Walls and beams are rotated rectangles, so diagonal members are not
/// inflated to their AABB. Ramps/stairs use their bounding box (conservative).
fn prism<L: Lookup + ?Sized>(project: &L, e: &Element) -> Option<(Vec<Vec2>, f32, f32)> {
    let rotated = |start: &Vec3, end: &Vec3, w: f32| {
        let (s, t) = (Vec2::new(start.x, start.z), Vec2::new(end.x, end.z));
        let n = (t - s).normalize_or_zero().perp() * (w / 2.0);
        vec![s + n, t + n, t - n, s - n]
    };
    Some(match &e.kind {
        ElementKind::Wall {
            start,
            end,
            height,
            thickness,
        } => (rotated(start, end, *thickness), start.y, start.y + height),
        ElementKind::Beam {
            start,
            end,
            width,
            depth,
        } => (rotated(start, end, *width), start.y, start.y + depth),
        ElementKind::Door { .. } | ElementKind::Mesh { .. } => return None,
        _ => {
            let b = aabb(project, e).ok()?;
            (
                vec![
                    Vec2::new(b.min.x, b.min.z),
                    Vec2::new(b.max.x, b.min.z),
                    Vec2::new(b.max.x, b.max.z),
                    Vec2::new(b.min.x, b.max.z),
                ],
                b.min.y,
                b.max.y,
            )
        }
    })
}

fn signed_area(poly: &[Vec2]) -> f32 {
    (0..poly.len())
        .map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()]))
        .sum::<f32>()
        / 2.0
}

/// Sutherland-Hodgman: `subject` clipped to convex `clip` (either winding).
fn clip_convex(subject: &[Vec2], clip: &[Vec2]) -> Vec<Vec2> {
    let orient = signed_area(clip).signum();
    if orient == 0.0 {
        return vec![];
    }
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        if out.is_empty() {
            break;
        }
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let side = |p: Vec2| orient * (b - a).perp_dot(p - a);
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (dp, dq) = (side(p), side(q));
            if dp >= 0.0 {
                out.push(p);
            }
            if (dp >= 0.0) != (dq >= 0.0) {
                out.push(p + (q - p) * (dp / (dp - dq)));
            }
        }
    }
    out
}

/// Exact overlap of two element prisms, or None if they do not overlap.
fn overlap<L: Lookup + ?Sized>(project: &L, a: &Element, b: &Element) -> Option<(f32, Aabb)> {
    let (pa, a0, a1) = prism(project, a)?;
    let (pb, b0, b1) = prism(project, b)?;
    let (y0, y1) = (a0.max(b0), a1.min(b1));
    if y1 <= y0 {
        return None;
    }
    let poly = clip_convex(&pa, &pb);
    let area = signed_area(&poly).abs();
    if area <= 0.0 {
        return None;
    }
    let (lo, hi) = poly
        .iter()
        .fold((poly[0], poly[0]), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    Some((
        area * (y1 - y0),
        Aabb {
            min: vec3(lo.x, y0, lo.y),
            max: vec3(hi.x, y1, hi.y),
        },
    ))
}

/// Bounding-volume hierarchy over item boxes: median split on the longest
/// centroid axis, leaves of at most `LEAF` items. Deterministic for a given input.
struct Bvh {
    nodes: Vec<BvhNode>,
    items: Vec<u32>,
}
struct BvhNode {
    b: Aabb,
    /// Leaf: items[start..end]. Inner: children at `start` and `end` (node indices).
    start: u32,
    end: u32,
    leaf: bool,
}
impl Bvh {
    const LEAF: usize = 4;
    fn new(boxes: &[Aabb]) -> Self {
        let mut t = Bvh {
            nodes: Vec::with_capacity(boxes.len() / 2 + 1),
            items: (0..boxes.len() as u32).collect(),
        };
        if !boxes.is_empty() {
            t.build(boxes, 0, boxes.len());
        }
        t
    }
    fn build(&mut self, boxes: &[Aabb], lo: usize, hi: usize) -> u32 {
        let items = &mut self.items[lo..hi];
        let b0 = boxes[items[0] as usize];
        let (b, cmin, cmax) = items.iter().fold((b0, b0.min, b0.min), |(b, lo, hi), &i| {
            let x = boxes[i as usize];
            let c = (x.min + x.max) * 0.5;
            (
                Aabb {
                    min: b.min.min(x.min),
                    max: b.max.max(x.max),
                },
                lo.min(c),
                hi.max(c),
            )
        });
        let id = self.nodes.len() as u32;
        if items.len() <= Self::LEAF {
            self.nodes.push(BvhNode {
                b,
                start: lo as u32,
                end: hi as u32,
                leaf: true,
            });
            return id;
        }
        let ext = cmax - cmin;
        let axis = if ext.x >= ext.y && ext.x >= ext.z {
            0
        } else if ext.y >= ext.z {
            1
        } else {
            2
        };
        let key = |i: &u32| {
            let x = boxes[*i as usize];
            x.min[axis] + x.max[axis]
        };
        let mid = items.len() / 2;
        items.select_nth_unstable_by(mid, |a, b| key(a).total_cmp(&key(b)));
        self.nodes.push(BvhNode {
            b,
            start: 0,
            end: 0,
            leaf: false,
        });
        let l = self.build(boxes, lo, lo + mid);
        let r = self.build(boxes, lo + mid, hi);
        let n = &mut self.nodes[id as usize];
        (n.start, n.end) = (l, r);
        id
    }
    /// Every item j whose box strictly intersects `q`.
    fn query(&self, boxes: &[Aabb], q: &Aabb, stack: &mut Vec<u32>, mut hit: impl FnMut(usize)) {
        if self.nodes.is_empty() {
            return;
        }
        stack.clear();
        stack.push(0);
        while let Some(n) = stack.pop() {
            let n = &self.nodes[n as usize];
            if !n.b.intersects(q) {
                continue;
            }
            if n.leaf {
                for &j in &self.items[n.start as usize..n.end as usize] {
                    if boxes[j as usize].intersects(q) {
                        hit(j as usize);
                    }
                }
            } else {
                stack.extend([n.end, n.start]);
            }
        }
    }
}

/// Cross-discipline clashes: BVH over element AABBs, then exact prism overlap.
/// Output order is fixed: pairs ordered by the elements' AABB min.x rank (stable, so
/// ties keep model order), first by `a` then by `b`; `a` is the lower-ranked element.
/// Doors excluded (they live inside their host). Same-discipline contact (beam
/// on column, wall on slab) is intentional framing, so it is not reported.
/// `tolerance_m3`: ignore overlaps smaller than this (touching faces, minor penetration).
pub fn clashes(project: &Project, tolerance_m3: f32) -> Vec<Clash> {
    let ix = ProjectIndex::new(project);
    let mut items: Vec<(&Element, Aabb)> = project
        .elements
        .iter()
        .filter(|e| !matches!(e.kind, ElementKind::Door { .. }))
        .filter_map(|e| aabb(&ix, e).ok().map(|b| (e, b)))
        .collect();
    items.sort_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x));
    // after the sort an item's index is its rank
    let boxes: Vec<Aabb> = items.iter().map(|x| x.1).collect();
    let bvh = Bvh::new(&boxes);
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut stack = Vec::new();
    for (i, (ea, ba)) in items.iter().enumerate() {
        bvh.query(&boxes, ba, &mut stack, |j| {
            if j > i && ea.discipline != items[j].0.discipline {
                pairs.push((i as u32, j as u32));
            }
        });
    }
    pairs.sort_unstable();
    pairs
        .into_iter()
        .filter_map(|(i, j)| {
            let (ea, eb) = (items[i as usize].0, items[j as usize].0);
            let (v, region) = overlap(&ix, ea, eb)?;
            (v > tolerance_m3).then_some(Clash {
                a: ea.id,
                b: eb.id,
                overlap_m3: v,
                region,
            })
        })
        .collect()
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
    fn nested_door_does_not_refill_opening() {
        let mut p = Project::sample();
        let wall = by_name(&p, "South wall").id;
        // inside the 5.0..5.75 main entry: cursor must stay at 5.75
        p.add(Element::new(
            "inner",
            ElementKind::Door {
                host: wall,
                offset: 5.1,
                width: 0.3,
                height: 2.1,
                is_egress: false,
            },
            Discipline::Architectural,
            PhaseId(3),
        ));
        let m = mesh_for(&p, by_name(&p, "South wall")).unwrap();
        // no wall vertex strictly inside the opening below the door head
        assert!(
            !m.positions
                .iter()
                .any(|v| v.x > 5.0 + 1e-3 && v.x < 5.75 - 1e-3 && v.y < 0.2 + 2.1 - 1e-3),
            "{:?}",
            m.positions
        );
    }
    #[test]
    fn stair_step_count() {
        let p = Project::sample();
        assert_eq!(
            mesh_for(&p, by_name(&p, "Stair to mezzanine"))
                .unwrap()
                .triangle_count(),
            16 * 12
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
    fn sample_clash_volume_is_exact() {
        // 0.4x0.4 column centred on the wall end overlaps 0.2 (x) * 0.25 (z) * 3.0 (y);
        // the old AABB test also counted the wall's phantom end-cap (0.244 m3)
        let p = Project::sample();
        for c in clashes(&p, 0.0) {
            assert!((c.overlap_m3 - 0.15).abs() < 1e-4, "{c:?}");
            assert!((c.region.volume() - 0.15).abs() < 1e-4, "{c:?}");
        }
    }
    #[test]
    fn diagonal_wall_no_false_clash() {
        let mut p = Project::new("x", "IBC-2024");
        p.add(Element::new(
            "diag",
            ElementKind::Wall {
                start: Vec3::ZERO,
                end: vec3(10.0, 0.0, 10.0),
                height: 3.0,
                thickness: 0.2,
            },
            Discipline::Architectural,
            PhaseId(0),
        ));
        // inside the wall's AABB but 5.6 m from the wall line
        p.add(Element::new(
            "corner col",
            ElementKind::Column {
                base: vec3(9.0, 0.0, 1.0),
                width: 0.4,
                depth: 0.4,
                height: 3.0,
            },
            Discipline::Structural,
            PhaseId(0),
        ));
        assert!(clashes(&p, 0.0).is_empty());
        // on the wall line it does clash
        if let ElementKind::Column { base, .. } = &mut p.elements[1].kind {
            *base = vec3(5.0, 0.0, 5.0);
        }
        let c = clashes(&p, 0.0);
        assert_eq!(c.len(), 1, "{c:?}");
    }
    #[test]
    fn clip_rotated_squares() {
        let sq = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let shifted: Vec<Vec2> = sq.iter().rev().map(|p| *p + Vec2::ONE).collect();
        assert!((signed_area(&clip_convex(&sq, &shifted)).abs() - 1.0).abs() < 1e-5);
        let far: Vec<Vec2> = sq.iter().map(|p| *p + Vec2::splat(5.0)).collect();
        assert!(clip_convex(&sq, &far).len() < 3);
    }
    /// Reference: every pair, same ordering contract as `clashes`.
    fn clashes_brute(project: &Project, tol: f32) -> Vec<Clash> {
        let mut items: Vec<(&Element, Aabb)> = project
            .elements
            .iter()
            .filter(|e| !matches!(e.kind, ElementKind::Door { .. }))
            .filter_map(|e| aabb(project, e).ok().map(|b| (e, b)))
            .collect();
        items.sort_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x));
        let mut out = Vec::new();
        for i in 0..items.len() {
            for j in i + 1..items.len() {
                let ((ea, ba), (eb, bb)) = (items[i], items[j]);
                if ea.discipline == eb.discipline || !ba.intersects(&bb) {
                    continue;
                }
                if let Some((v, region)) = overlap(project, ea, eb) {
                    if v > tol {
                        out.push(Clash {
                            a: ea.id,
                            b: eb.id,
                            overlap_m3: v,
                            region,
                        });
                    }
                }
            }
        }
        out
    }
    #[test]
    fn bvh_matches_brute_force_on_random_project() {
        let mut seed: u64 = 0x5eed_b111;
        let mut rnd = || {
            // Numerical Recipes LCG, top bits
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut p = Project::new("random", "IBC-2024");
        let discs = [
            Discipline::Architectural,
            Discipline::Structural,
            Discipline::Mechanical,
            Discipline::Plumbing,
        ];
        for i in 0..2000 {
            let at = vec3(rnd() * 60.0, rnd() * 12.0, rnd() * 60.0);
            let kind = match i % 4 {
                0 => ElementKind::Wall {
                    start: at,
                    end: at + vec3(rnd() * 8.0 - 4.0, 0.0, rnd() * 8.0 - 4.0),
                    height: 1.0 + rnd() * 3.0,
                    thickness: 0.1 + rnd() * 0.3,
                },
                1 => ElementKind::Column {
                    base: at,
                    width: 0.2 + rnd() * 0.6,
                    depth: 0.2 + rnd() * 0.6,
                    height: 1.0 + rnd() * 4.0,
                },
                2 => ElementKind::Beam {
                    start: at,
                    end: at + vec3(rnd() * 10.0 - 5.0, rnd() * 2.0 - 1.0, rnd() * 10.0 - 5.0),
                    width: 0.2 + rnd() * 0.3,
                    depth: 0.2 + rnd() * 0.6,
                },
                _ => ElementKind::Slab {
                    origin: at,
                    size_x: 1.0 + rnd() * 6.0,
                    size_z: 1.0 + rnd() * 6.0,
                    thickness: 0.1 + rnd() * 0.3,
                },
            };
            let d = discs[(rnd() * 4.0) as usize % 4];
            p.add(Element::new(format!("e{i}"), kind, d, PhaseId(0)));
        }
        let fast = clashes(&p, 0.0);
        let slow = clashes_brute(&p, 0.0);
        assert!(
            fast.len() > 100,
            "test should exercise many clashes: {}",
            fast.len()
        );
        assert_eq!(fast, slow);
        assert_eq!(clashes(&p, 0.05), clashes_brute(&p, 0.05));
        assert!(clashes(&Project::new("e", "x"), 0.0).is_empty());
    }
    #[test]
    fn sloped_beam_mesh_follows_its_axis() {
        let p = Project::new("x", "IBC-2024");
        let e = Element::new(
            "rafter",
            ElementKind::Beam {
                start: vec3(1.0, 3.0, 2.0),
                end: vec3(7.0, 5.0, 2.0),
                width: 0.2,
                depth: 0.4,
            },
            Discipline::Structural,
            PhaseId(0),
        );
        let m = mesh_for(&p, &e).unwrap();
        let (mb, ab) = (m.aabb().unwrap(), aabb(&p, &e).unwrap());
        // the AABB pads both plan axes by width/2 (conservative); Y must match exactly
        assert!(
            mb.min.cmpge(ab.min - 1e-4).all() && mb.max.cmple(ab.max + 1e-4).all(),
            "{mb:?} vs {ab:?}"
        );
        assert!((mb.min.y - 3.0).abs() < 1e-4 && (mb.max.y - 5.4).abs() < 1e-4);
        assert!((mb.min.x - 1.0).abs() < 1e-4 && (mb.max.x - 7.0).abs() < 1e-4);
        // every vertex sits on the section band above the axis at its x
        for v in &m.positions {
            let y_axis = 3.0 + (v.x - 1.0) / 6.0 * 2.0;
            assert!(v.y > y_axis - 1e-4 && v.y < y_axis + 0.4 + 1e-4, "{v:?}");
        }
        // top face normal tilts back against the slope, unit length
        assert!(m
            .normals
            .iter()
            .any(|n| n.y > 0.9 && n.x < -0.1 && (n.length() - 1.0).abs() < 1e-4));
        // a vertical member falls back to its bounds; horizontal beams are unchanged
        let mut v = e.clone();
        v.kind = ElementKind::Beam {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 3.0, 0.0),
            width: 0.2,
            depth: 0.2,
        };
        assert_eq!(mesh_for(&p, &v).unwrap().aabb(), aabb(&p, &v).ok());
        let sp = Project::sample();
        let roof = by_name(&sp, "Roof beam");
        assert_eq!(mesh_for(&sp, roof).unwrap().triangle_count(), 12);
    }
    #[test]
    fn index_and_project_meshes_agree() {
        let p = Project::sample();
        let ix = ProjectIndex::new(&p);
        for e in &p.elements {
            assert_eq!(mesh_for(&p, e).ok(), mesh_for(&ix, e).ok());
            assert_eq!(aabb(&p, e).ok(), aabb(&ix, e).ok());
        }
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
