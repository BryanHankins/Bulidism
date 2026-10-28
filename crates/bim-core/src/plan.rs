//! Plan (top view) polygon helpers. Plan coordinates are model (x, z) in metres.

/// Signed area (shoelace); positive when counter-clockwise in (x, z).
pub fn signed_area(poly: &[[f32; 2]]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        / 2.0
}

/// Point strictly inside, or on the boundary of, a simple polygon (even-odd rule; boundary
/// points within `1e-4` m count as inside).
pub fn contains(poly: &[[f32; 2]], p: [f32; 2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if dist_to_segment(p, a, b) <= 1e-4 {
            return true;
        }
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

pub fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub fn dist_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(p, [a[0] + t * dx, a[1] + t * dz])
}

fn orient(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Segments a-b and c-d cross at a single interior point of both (touching at an
/// endpoint or overlapping collinearly does not count).
pub fn segments_cross(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    const E: f32 = 1e-6;
    let (d1, d2) = (orient(c, d, a), orient(c, d, b));
    let (d3, d4) = (orient(a, b, c), orient(a, b, d));
    ((d1 > E && d2 < -E) || (d1 < -E && d2 > E)) && ((d3 > E && d4 < -E) || (d3 < -E && d4 > E))
}

/// Why `poly` is not a usable room outline, if it is not: fewer than 3 corners, non-finite
/// coordinates, (near) zero area, or edges that cross each other.
pub fn outline_problem(poly: &[[f32; 2]]) -> Option<String> {
    if poly.len() < 3 {
        return Some(format!("has {} corner(s); at least 3 needed", poly.len()));
    }
    if poly.iter().flatten().any(|v| !v.is_finite()) {
        return Some("has a non-finite coordinate".into());
    }
    let n = poly.len();
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let (c, d) = (poly[j], poly[(j + 1) % n]);
            if segments_cross(a, b, c, d) {
                return Some(format!("edges {} and {} cross", i + 1, j + 1));
            }
        }
    }
    if signed_area(poly).abs() < 1e-3 {
        return Some("encloses no area".into());
    }
    None
}

/// Clean an outline read from a file: drop repeated points (incl. a closing copy of the first)
/// and collinear corners, and orient it counter-clockwise.
pub fn tidy(mut poly: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    poly.dedup_by(|a, b| dist(*a, *b) < 1e-4);
    while poly.len() > 1 && dist(poly[0], *poly.last().unwrap()) < 1e-4 {
        poly.pop();
    }
    let mut changed = true;
    while changed && poly.len() > 3 {
        changed = false;
        for i in 0..poly.len() {
            let n = poly.len();
            let (a, b, c) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            if orient(a, b, c).abs() < 1e-5 * dist(a, c).max(1.0) {
                poly.remove(i);
                changed = true;
                break;
            }
        }
    }
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    poly
}

#[cfg(test)]
mod tests {
    use super::*;
    const L: [[f32; 2]; 6] = [[0., 0.], [6., 0.], [6., 2.], [2., 2.], [2., 5.], [0., 5.]];

    #[test]
    fn area_and_containment_of_an_l_room() {
        assert!((signed_area(&L) - 18.0).abs() < 1e-5);
        assert!(contains(&L, [1.0, 4.0]) && contains(&L, [5.0, 1.0]));
        assert!(!contains(&L, [4.0, 4.0]), "the notch is outside");
        assert!(contains(&L, [6.0, 1.0]), "boundary counts");
        assert!(outline_problem(&L).is_none());
    }

    #[test]
    fn bad_outlines() {
        assert!(outline_problem(&L[..2]).unwrap().contains("corner"));
        let bow = [[0., 0.], [2., 2.], [2., 0.], [0., 2.]];
        assert!(outline_problem(&bow).unwrap().contains("cross"));
        assert!(outline_problem(&[[0., 0.], [1., 0.], [2., 0.]])
            .unwrap()
            .contains("no area"));
        assert!(outline_problem(&[[0., 0.], [f32::NAN, 0.], [0., 1.]]).is_some());
    }

    #[test]
    fn tidy_closes_dedups_and_orients() {
        let mut cw: Vec<[f32; 2]> = L.iter().rev().copied().collect();
        cw.insert(2, cw[1]); // repeated point
        cw.insert(4, [3.0, 2.0]); // collinear on the 2 m edge? (between [6,2] and [2,2] reversed)
        cw.push(cw[0]); // closing copy
        let t = tidy(cw);
        assert_eq!(t.len(), 6, "{t:?}");
        assert!(signed_area(&t) > 0.0);
        assert!((signed_area(&t) - 18.0).abs() < 1e-4);
    }
}
