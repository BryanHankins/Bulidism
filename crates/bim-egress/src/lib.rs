//! Egress geometry: which rooms each door connects, walking distances inside rooms (around
//! corners, not through walls), exit access travel distance and common path of egress travel.
//!
//! Model, in plan (x, z) per storey:
//! - A room is a space with a plan shape (`Space::polygon`: its outline or rectangle).
//! - A door connects the rooms found just either side of it (0.3 m past the wall face); a side
//!   with no room is "outside" (or an unmodelled room).
//! - An exit is an egress door (`is_egress`) leading outside, or any door into an exit stair
//!   room: a room holding a stair that is named as a stair or mostly stair (the flight covers
//!   a quarter of it or more). A room with an open stair in it is not an exit. Exit access
//!   travel ends at the exit.
//! - Travel distance of a room: the longest walk from any point in it (sampled on a grid, its
//!   corners and edges) to the nearest exit, through doors and around room corners. Furniture
//!   and fixed seating are not modelled.
//! - Common path: walked distance until the occupant can reach two distinct exits by routes
//!   that share no room. In a room that itself has such routes it is taken as 0 (exit
//!   separation is a separate check, not made here).
use bim_core::plan::{contains, dist, segments_cross, signed_area};
use bim_core::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Probe distance past the wall face used to find the room on each side of a door.
const PROBE_M: f32 = 0.3;

/// One door and the rooms (indices into `Project::spaces`) on its two sides.
#[derive(Debug, Clone)]
pub struct DoorLink {
    pub id: ElementId,
    pub name: String,
    /// Centre of the opening in plan.
    pub center: [f32; 2],
    pub width: f32,
    pub is_egress: bool,
    /// Room on each side; None = outside (or a room not modelled as a space).
    pub sides: [Option<usize>; 2],
    /// Walking point just inside each side (on the wall face).
    pub points: [[f32; 2]; 2],
    /// Counts as an exit: egress door to outside, or a door into an exit stair room.
    pub exit: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SpaceEgress {
    /// The room has a plan shape, so it was analysed.
    pub analyzed: bool,
    /// Holds a stair: treated as an exit stair enclosure (its doors are exits).
    pub stair: bool,
    /// Longest walk to the nearest exit; None when not analysed or no exit can be reached.
    pub travel_m: Option<f32>,
    pub common_path_m: Option<f32>,
    /// The longest walk, from the most remote point to the exit, in plan.
    pub path: Vec<[f32; 2]>,
    pub exit_door: Option<ElementId>,
}

#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub doors: Vec<DoorLink>,
    /// One per `Project::spaces`, same order.
    pub spaces: Vec<SpaceEgress>,
}

/// A room shape prepared for shortest walks: reflex corners are the only places a shortest
/// path can bend.
struct Room {
    poly: Vec<[f32; 2]>,
    reflex: Vec<[f32; 2]>,
}

impl Room {
    fn new(mut poly: Vec<[f32; 2]>) -> Self {
        if signed_area(&poly) < 0.0 {
            poly.reverse();
        }
        let n = poly.len();
        let reflex = (0..n)
            .filter(|&i| {
                let (a, b, c) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
                (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) < -1e-6
            })
            .map(|i| poly[i])
            .collect();
        Room { poly, reflex }
    }

    /// A straight walk from a to b stays inside the room.
    fn visible(&self, a: [f32; 2], b: [f32; 2]) -> bool {
        let n = self.poly.len();
        for i in 0..n {
            if segments_cross(a, b, self.poly[i], self.poly[(i + 1) % n]) {
                return false;
            }
        }
        // touching corners is fine, but the walk must not leave through a gap between them
        [0.25f32, 0.5, 0.75].iter().all(|t| {
            contains(
                &self.poly,
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            )
        })
    }

    /// Shortest walk from `a` to every target, bending only at reflex corners. Returns, per
    /// target, the length and the bend points between a and the target.
    fn walks(&self, a: [f32; 2], targets: &[[f32; 2]]) -> Vec<(f32, Vec<[f32; 2]>)> {
        // nodes: 0 = a, 1..=r reflex corners
        let r = self.reflex.len();
        let node = |i: usize| if i == 0 { a } else { self.reflex[i - 1] };
        let mut d = vec![f32::INFINITY; r + 1];
        let mut prev = vec![usize::MAX; r + 1];
        let mut done = vec![false; r + 1];
        d[0] = 0.0;
        for _ in 0..=r {
            let Some(u) = (0..=r)
                .filter(|&i| !done[i] && d[i].is_finite())
                .min_by(|&i, &j| d[i].total_cmp(&d[j]))
            else {
                break;
            };
            done[u] = true;
            for v in 1..=r {
                if !done[v] && self.visible(node(u), node(v)) {
                    let nd = d[u] + dist(node(u), node(v));
                    if nd < d[v] {
                        d[v] = nd;
                        prev[v] = u;
                    }
                }
            }
        }
        targets
            .iter()
            .map(|&t| {
                let mut best = (f32::INFINITY, usize::MAX);
                for (i, di) in d.iter().enumerate() {
                    if di.is_finite() && self.visible(node(i), t) {
                        let c = di + dist(node(i), t);
                        if c < best.0 {
                            best = (c, i);
                        }
                    }
                }
                let mut bends = Vec::new();
                let mut i = best.1;
                while i != usize::MAX && i != 0 {
                    bends.push(node(i));
                    i = prev[i];
                }
                bends.reverse();
                (best.0, bends)
            })
            .collect()
    }

    /// Shortest-walk distances to `targets` from any point, prepared once per room: a walk
    /// either goes straight or first bends at a reflex corner it can see.
    fn field(&self, targets: &[[f32; 2]]) -> Field<'_> {
        let via = targets
            .iter()
            .map(|&t| {
                self.walks(t, &self.reflex)
                    .into_iter()
                    .map(|w| w.0)
                    .collect()
            })
            .collect();
        Field {
            room: self,
            targets: targets.to_vec(),
            via,
        }
    }

    /// Points standing for "anywhere in the room": corners, points along the edges and a
    /// grid inside.
    fn samples(&self) -> Vec<[f32; 2]> {
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for p in &self.poly {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        // about 1,500 points per room, never further apart than needed in small rooms; the
        // longest walk is found to within about one step
        let step = (signed_area(&self.poly).abs() / 1500.0).sqrt().max(0.5);
        let mut out = self.poly.clone();
        let n = self.poly.len();
        for i in 0..n {
            let (a, b) = (self.poly[i], self.poly[(i + 1) % n]);
            let k = (dist(a, b) / step).floor() as usize;
            for j in 1..=k {
                let t = j as f32 / (k + 1) as f32;
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        let mut x = lo[0] + step / 2.0;
        while x < hi[0] {
            let mut z = lo[1] + step / 2.0;
            while z < hi[1] {
                if contains(&self.poly, [x, z]) {
                    out.push([x, z]);
                }
                z += step;
            }
            x += step;
        }
        out
    }
}

struct Field<'a> {
    room: &'a Room,
    targets: Vec<[f32; 2]>,
    /// via[t][r]: shortest walk from target t to reflex corner r
    via: Vec<Vec<f32>>,
}

impl Field<'_> {
    fn from(&self, p: [f32; 2]) -> Vec<f32> {
        let seen: Vec<Option<f32>> = self
            .room
            .reflex
            .iter()
            .map(|&r| self.room.visible(p, r).then(|| dist(p, r)))
            .collect();
        self.targets
            .iter()
            .zip(&self.via)
            .map(|(&t, via)| {
                let direct = if self.room.visible(p, t) {
                    dist(p, t)
                } else {
                    f32::INFINITY
                };
                seen.iter()
                    .zip(via)
                    .filter_map(|(s, v)| s.map(|s| s + v))
                    .fold(direct, f32::min)
            })
            .collect()
    }
}

fn level_match(a: Option<LevelId>, b: Option<LevelId>) -> bool {
    a.is_none() || b.is_none() || a == b
}

/// Doors and the rooms on each side of them.
fn doors(p: &Project, polys: &[Option<Vec<[f32; 2]>>], stair: &[bool]) -> Vec<DoorLink> {
    let room_at = |pt: [f32; 2], level: Option<LevelId>| -> Option<usize> {
        // the smallest room containing the point (rooms drawn inside rooms)
        polys
            .iter()
            .enumerate()
            .filter(|(i, poly)| {
                level_match(p.spaces[*i].level, level)
                    && poly.as_deref().is_some_and(|o| contains(o, pt))
            })
            .min_by(|a, b| {
                let area = |o: &Option<Vec<[f32; 2]>>| signed_area(o.as_deref().unwrap()).abs();
                area(a.1).total_cmp(&area(b.1))
            })
            .map(|(i, _)| i)
    };
    let mut out = Vec::new();
    for e in &p.elements {
        let ElementKind::Door {
            host,
            offset,
            width,
            is_egress,
            ..
        } = e.kind
        else {
            continue;
        };
        let Some(h) = p.elements.iter().find(|w| w.id == host) else {
            continue;
        };
        let ElementKind::Wall {
            start,
            end,
            thickness,
            ..
        } = h.kind
        else {
            continue;
        };
        let (dx, dz) = (end.x - start.x, end.z - start.z);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 1e-6 {
            continue;
        }
        let (ux, uz) = (dx / len, dz / len);
        let s = offset + width / 2.0;
        let c = [start.x + ux * s, start.z + uz * s];
        let nrm = [-uz, ux];
        let level = e.level.or(h.level);
        let at = |sign: f32, d: f32| [c[0] + nrm[0] * sign * d, c[1] + nrm[1] * sign * d];
        let mut sides = [None, None];
        let mut points = [c, c];
        for (k, sign) in [1.0f32, -1.0].into_iter().enumerate() {
            sides[k] = room_at(at(sign, thickness / 2.0 + PROBE_M), level);
            // walk from the wall face when it is inside the room, else from the probe point
            let face = at(sign, thickness / 2.0 + 0.02);
            points[k] = match sides[k] {
                Some(i) if contains(polys[i].as_deref().unwrap(), face) => face,
                _ => at(sign, thickness / 2.0 + PROBE_M),
            };
        }
        if sides[0].is_some() && sides[0] == sides[1] {
            // a door inside one room (e.g. on a partial-height partition): no connection
            continue;
        }
        let to_stair = sides.iter().flatten().any(|&i| stair[i]);
        let outside = sides.iter().any(Option::is_none);
        out.push(DoorLink {
            id: e.id,
            name: e.name.clone(),
            center: c,
            width,
            is_egress,
            sides,
            points,
            exit: (is_egress && outside) || to_stair,
        });
    }
    out
}

#[derive(PartialEq)]
struct Item(f32, usize);
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0)
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Door-point graph. Node 2*d + k is door d's walking point on side k. Edges: across each
/// door, and between door points of the same room (shortest walk inside it).
struct Net {
    adj: Vec<Vec<(usize, f32)>>,
}

impl Net {
    fn dijkstra(&self, sources: &[(usize, f32)]) -> (Vec<f32>, Vec<usize>) {
        let n = self.adj.len();
        let (mut d, mut prev) = (vec![f32::INFINITY; n], vec![usize::MAX; n]);
        let mut q = BinaryHeap::new();
        for &(s, c) in sources {
            if c < d[s] {
                d[s] = c;
                q.push(Item(c, s));
            }
        }
        while let Some(Item(c, u)) = q.pop() {
            if c > d[u] {
                continue;
            }
            for &(v, w) in &self.adj[u] {
                if c + w < d[v] {
                    d[v] = c + w;
                    prev[v] = u;
                    q.push(Item(d[v], v));
                }
            }
        }
        (d, prev)
    }
}

/// Rooms from which two exits can be reached by routes sharing no room (max flow of 2 with
/// unit room capacities).
fn branch_rooms(n_rooms: usize, doors: &[DoorLink], stair: &[bool]) -> Vec<bool> {
    // nodes: room i in = 2i, room i out = 2i+1, sink = 2n
    let sink = 2 * n_rooms;
    let mut cap: std::collections::HashMap<(usize, usize), i32> = Default::default();
    let mut adj = vec![Vec::new(); sink + 1];
    let mut add =
        |cap: &mut std::collections::HashMap<(usize, usize), i32>, a: usize, b: usize, c: i32| {
            *cap.entry((a, b)).or_default() += c;
            cap.entry((b, a)).or_default();
            adj[a].push(b);
            adj[b].push(a);
        };
    for i in 0..n_rooms {
        add(&mut cap, 2 * i, 2 * i + 1, 1);
    }
    for d in doors {
        match d.sides {
            [Some(a), Some(b)] => {
                if d.exit {
                    // into an exit stair: the stair room is the exit
                    for (x, y) in [(a, b), (b, a)] {
                        if stair[y] && !stair[x] {
                            add(&mut cap, 2 * x + 1, sink, 1);
                        }
                    }
                } else {
                    add(&mut cap, 2 * a + 1, 2 * b, 1);
                    add(&mut cap, 2 * b + 1, 2 * a, 1);
                }
            }
            [Some(a), None] | [None, Some(a)] if d.exit => add(&mut cap, 2 * a + 1, sink, 1),
            _ => {}
        }
    }
    (0..n_rooms)
        .map(|s| {
            let mut c = cap.clone();
            let mut flow = 0;
            // source is room s's out-node (its own capacity does not limit it)
            while flow < 2 {
                let mut prev = vec![usize::MAX; sink + 1];
                let mut q = std::collections::VecDeque::from([2 * s + 1]);
                prev[2 * s + 1] = 2 * s + 1;
                while let Some(u) = q.pop_front() {
                    for &v in &adj[u] {
                        if prev[v] == usize::MAX && c[&(u, v)] > 0 {
                            prev[v] = u;
                            q.push_back(v);
                        }
                    }
                }
                if prev[sink] == usize::MAX {
                    break;
                }
                let mut v = sink;
                while v != 2 * s + 1 {
                    let u = prev[v];
                    *c.get_mut(&(u, v)).unwrap() -= 1;
                    *c.get_mut(&(v, u)).unwrap() += 1;
                    v = u;
                }
                flow += 1;
            }
            flow >= 2
        })
        .collect()
}

/// Analyse every space with a plan shape.
pub fn analyze(p: &Project) -> Analysis {
    let polys: Vec<Option<Vec<[f32; 2]>>> = p.spaces.iter().map(Space::polygon).collect();
    let elev = |l: Option<LevelId>| {
        l.and_then(|id| p.levels.iter().find(|x| x.id == id))
            .map_or(0.0, |x| x.elevation_m)
    };
    // exit stair rooms: hold a stair that starts or arrives on the room's level
    let stair: Vec<bool> = p
        .spaces
        .iter()
        .zip(&polys)
        .map(|(s, poly)| {
            let Some(poly) = poly else { return false };
            p.elements.iter().any(|e| {
                let ElementKind::Stair {
                    base,
                    run_length,
                    total_rise,
                    width,
                    ..
                } = e.kind
                else {
                    return false;
                };
                let centre = [base.x + run_length / 2.0, base.z + width / 2.0];
                let y = elev(s.level);
                // an enclosure, not a room with an open stair in it: named as a stair, or
                // mostly stair (a flight plus landings fills a quarter or more of it)
                let enclosure = s.name.to_lowercase().contains("stair")
                    || run_length * width >= 0.25 * signed_area(poly).abs();
                enclosure
                    && contains(poly, centre)
                    && (level_match(e.level, s.level)
                        || (base.y - y).abs() < 0.3
                        || (base.y + total_rise - y).abs() < 0.3)
            })
        })
        .collect();
    let doors = doors(p, &polys, &stair);
    let rooms: Vec<Option<Room>> = polys.iter().map(|o| o.clone().map(Room::new)).collect();

    // door points per room, and the walks between them
    let mut in_room: Vec<Vec<usize>> = vec![Vec::new(); p.spaces.len()];
    for (d, link) in doors.iter().enumerate() {
        for k in 0..2 {
            if let Some(i) = link.sides[k] {
                in_room[i].push(2 * d + k);
            }
        }
    }
    let pt = |node: usize| doors[node / 2].points[node % 2];
    let mut net = Net {
        adj: vec![Vec::new(); 2 * doors.len()],
    };
    for (d, link) in doors.iter().enumerate() {
        if link.sides[0].is_some() && link.sides[1].is_some() {
            let w = dist(link.points[0], link.points[1]);
            net.adj[2 * d].push((2 * d + 1, w));
            net.adj[2 * d + 1].push((2 * d, w));
        }
    }
    for (i, nodes) in in_room.iter().enumerate() {
        let Some(room) = &rooms[i] else { continue };
        for (a_ix, &a) in nodes.iter().enumerate() {
            let targets: Vec<[f32; 2]> = nodes[a_ix + 1..].iter().map(|&b| pt(b)).collect();
            for (k, (w, _)) in room.walks(pt(a), &targets).into_iter().enumerate() {
                if w.is_finite() {
                    let b = nodes[a_ix + 1 + k];
                    net.adj[a].push((b, w));
                    net.adj[b].push((a, w));
                }
            }
        }
    }

    // distance from each door point to the nearest exit: exits are the door points on the
    // room side of an exit door (plus the short step through it)
    let stair_ref = &stair;
    let exit_sources: Vec<(usize, f32)> = doors
        .iter()
        .enumerate()
        .filter(|(_, l)| l.exit)
        .flat_map(|(d, l)| {
            (0..2)
                .filter(move |&k| l.sides[k].is_some_and(|i| !stair_ref[i]))
                .map(move |k| (2 * d + k, dist(l.points[k], l.center)))
        })
        .collect();
    let (to_exit, prev) = net.dijkstra(&exit_sources);
    let branch = branch_rooms(p.spaces.len(), &doors, &stair);
    // distance to the first point with two separate routes: entering a branch room
    let branch_sources: Vec<(usize, f32)> = (0..2 * doors.len())
        .filter(|&n| doors[n / 2].sides[n % 2].is_some_and(|i| branch[i]))
        .map(|n| (n, 0.0))
        .collect();
    let (to_branch, _) = net.dijkstra(&branch_sources);

    let spaces = p
        .spaces
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let Some(room) = &rooms[i] else {
                return SpaceEgress::default();
            };
            let mut out = SpaceEgress {
                analyzed: true,
                stair: stair[i],
                ..Default::default()
            };
            if stair[i] {
                // inside the exit enclosure: exit access is over
                out.travel_m = Some(0.0);
                out.common_path_m = Some(0.0);
                return out;
            }
            let nodes = &in_room[i];
            let targets: Vec<[f32; 2]> = nodes.iter().map(|&n| pt(n)).collect();
            // the most remote point (travel, sample, door node), and the longest common path
            let mut worst: Option<(f32, [f32; 2], usize)> = None;
            let mut common = 0.0f32;
            let mut unreachable = targets.is_empty();
            // walking on toward the first room with two separate routes; where there is none
            // ahead, the whole route to the exit is common
            let via_branch: Vec<f32> = to_branch
                .iter()
                .zip(&to_exit)
                .map(|(b, e)| if b.is_finite() { *b } else { *e })
                .collect();
            let field = room.field(&targets);
            for s in room.samples() {
                let walks = field.from(s);
                let best = |via: &[f32]| {
                    walks
                        .iter()
                        .zip(nodes)
                        .map(|(w, &n)| (w + via[n], n))
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .filter(|b| b.0.is_finite())
                };
                let Some((t, n)) = best(&to_exit) else {
                    unreachable = true;
                    continue;
                };
                if !branch[i] {
                    common = common.max(best(&via_branch).map_or(t, |b| b.0));
                }
                if !matches!(worst, Some(w) if t <= w.0) {
                    worst = Some((t, s, n));
                }
            }
            if unreachable {
                return out; // travel None: some point cannot reach an exit
            }
            let Some((t, s, n)) = worst else {
                return out;
            };
            let cp = common;
            out.travel_m = Some(t);
            out.common_path_m = Some(cp);
            // the walk: sample → bends → door point → … → exit door centre
            let mut path = vec![s];
            path.extend(room.walks(s, &[pt(n)]).remove(0).1);
            let mut node = n;
            path.push(pt(node));
            while prev[node] != usize::MAX {
                let next = prev[node];
                if next / 2 != node / 2 {
                    // walk across a room between two of its doors
                    if let Some(r) = doors[node / 2].sides[node % 2].and_then(|i| rooms[i].as_ref())
                    {
                        path.extend(r.walks(pt(node), &[pt(next)]).remove(0).1);
                    }
                }
                path.push(pt(next));
                node = next;
            }
            let exit = &doors[node / 2];
            path.push(exit.center);
            out.exit_door = Some(exit.id);
            out.path = path;
            out
        })
        .collect();
    Analysis { doors, spaces }
}

#[cfg(test)]
mod tests;
