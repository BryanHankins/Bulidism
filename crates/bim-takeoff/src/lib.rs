//! Quantities from parametric geometry, priced by cost code, rolled up per phase.
use bim_core::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    M2,
    M3,
    Each,
    M,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub element: ElementId,
    pub name: String,
    pub cost_code: String,
    pub phase: PhaseId,
    pub qty: f32,
    pub unit: Unit,
    pub cost: Cents,
    pub lod: Lod,
}

pub fn quantity(e: &Element) -> (f32, Unit) {
    match &e.kind {
        ElementKind::Wall {
            start, end, height, ..
        } => (start.distance(*end) * height, Unit::M2),
        ElementKind::Slab {
            size_x,
            size_z,
            thickness,
            ..
        } => (size_x * size_z * thickness, Unit::M3),
        ElementKind::Column {
            width,
            depth,
            height,
            ..
        } => (width * depth * height, Unit::M3),
        ElementKind::Beam {
            start,
            end,
            width,
            depth,
        } => (start.distance(*end) * width * depth, Unit::M3),
        ElementKind::Door { .. } => (1.0, Unit::Each),
        ElementKind::Stair { run_length, .. } => (*run_length, Unit::M),
        ElementKind::Ramp {
            run_length, width, ..
        } => (run_length * width, Unit::M2),
        ElementKind::Mesh { .. } => (1.0, Unit::Each),
    }
}

pub fn takeoff(p: &Project) -> Vec<Line> {
    p.elements
        .iter()
        .map(|e| {
            let (qty, unit) = quantity(e);
            let unit_cost = p.unit_costs.get(&e.cost_code).copied().unwrap_or_default();
            Line {
                element: e.id,
                name: e.name.clone(),
                cost_code: e.cost_code.clone(),
                phase: e.phase,
                qty,
                unit,
                cost: Cents((qty as f64 * unit_cost.0 as f64).round() as i64),
                lod: e.lod,
            }
        })
        .collect()
}

pub fn cost_by_phase(lines: &[Line]) -> BTreeMap<PhaseId, Cents> {
    let mut m: BTreeMap<PhaseId, Cents> = BTreeMap::new();
    for l in lines {
        let e = m.entry(l.phase).or_default();
        *e = *e + l.cost;
    }
    m
}

/// Sum of lines whose phase satisfies `started` (e.g. schedule.is_started(ph, day)).
pub fn cumulative(lines: &[Line], started: impl Fn(PhaseId) -> bool) -> Cents {
    lines
        .iter()
        .filter(|l| started(l.phase))
        .map(|l| l.cost)
        .sum()
}

pub fn to_csv(lines: &[Line]) -> String {
    let mut s = String::from("element,cost_code,phase,qty,unit,cost_usd,lod\n");
    for l in lines {
        s.push_str(&format!(
            "{},{},{},{:.3},{:?},{:.2},{:?}\n",
            l.name.replace(',', " "),
            l.cost_code,
            l.phase.0,
            l.qty,
            l.unit,
            l.cost.dollars(),
            l.lod
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_costs() {
        let t = takeoff(&Project::sample());
        let by = cost_by_phase(&t);
        assert_eq!(
            by[&PhaseId(0)],
            Cents((12.0f64 * 8.0 * 0.2 * 18_000.0).round() as i64)
        );
        assert_eq!(by[&PhaseId(3)], Cents(2 * 85_000 + 420_000 + 100_800)); // 2 doors + 3.5 m stair + 2.88 m2 ramp
        assert_eq!(to_csv(&t).lines().count(), t.len() + 1);
        assert_eq!(cumulative(&t, |ph| ph == PhaseId(3)), by[&PhaseId(3)]);
    }
    #[test]
    fn missing_cost_code_is_zero() {
        let mut p = Project::new("x", "IBC-2024");
        p.add(Element::new(
            "w",
            ElementKind::Wall {
                start: glam::Vec3::ZERO,
                end: glam::Vec3::X,
                height: 1.0,
                thickness: 0.1,
            },
            Discipline::Architectural,
            PhaseId(0),
        ));
        assert_eq!(takeoff(&p)[0].cost, Cents(0));
    }
}
