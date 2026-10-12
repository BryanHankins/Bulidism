//! One entry point that runs the whole pipeline and returns a JSON report.
//! Shared by bim-cli (static HTML) and bim-wasm (live in the browser), so both
//! produce byte-identical data for the same viewer.
use bim_core::*;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("project: {0}")]
    Project(#[from] ProjectError),
    #[error("project has {} validation error(s): {}", .0.len(), .0.join("; "))]
    Invalid(Vec<String>),
    #[error("schedule: {0}")]
    Schedule(#[from] bim_schedule::ScheduleError),
    #[error("rules: {0}")]
    Rules(#[from] bim_rules::RuleError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("geometry: {0}")]
    Geom(#[from] bim_geom::GeomError),
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseOut {
    pub id: u32,
    pub name: String,
    pub start_day: u32,
    pub end_day: u32,
    pub critical: bool,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElementOut {
    pub id: ElementId,
    pub name: String,
    pub discipline: String,
    pub phase: u32,
    pub start_day: u32,
    pub cost_usd: f64,
    pub qty: f32,
    pub unit: String,
    pub cost_code: String,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClashOut {
    pub a: ElementId,
    pub b: ElementId,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub overlap_m3: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceOut {
    pub name: String,
    pub occupancy: String,
    pub area_m2: f32,
    pub exits: usize,
    pub bounds: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub project: String,
    pub rule_set: String,
    pub code_edition: String,
    pub rules_verified: String,
    pub total_days: u32,
    /// Exact sum of element costs (summed in cents, then converted).
    pub total_cost_usd: f64,
    pub phases: Vec<PhaseOut>,
    pub elements: Vec<ElementOut>,
    pub clashes: Vec<ClashOut>,
    pub advisories: Vec<bim_rules::Advisory>,
    pub spaces: Vec<SpaceOut>,
}

pub const CLASH_TOLERANCE_M3: f32 = 0.001;

/// Validate, schedule, evaluate rules, detect clashes, take off, mesh. Fails on invalid projects.
pub fn build(project: &Project, rules: Option<&bim_rules::RuleSet>) -> Result<Report, ReportError> {
    let problems = project.validate();
    if !problems.is_empty() {
        return Err(ReportError::Invalid(
            problems.iter().map(|e| e.to_string()).collect(),
        ));
    }
    let schedule = bim_schedule::schedule(&project.phases)?;
    let lines = bim_takeoff::takeoff(project);
    let by_phase = bim_takeoff::cost_by_phase(&lines);
    let phases = schedule
        .phases
        .iter()
        .map(|sp| PhaseOut {
            id: sp.id.0,
            name: project
                .phase(sp.id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "?".into()),
            start_day: sp.start_day,
            end_day: sp.end_day,
            critical: sp.critical,
            cost_usd: by_phase.get(&sp.id).copied().unwrap_or_default().dollars(),
        })
        .collect();
    let mut elements = Vec::with_capacity(project.elements.len());
    for e in &project.elements {
        // external-mesh elements have no parametric geometry but are still
        // scheduled and costed: keep them, with an empty mesh
        let m = match bim_geom::mesh_for(project, e) {
            Ok(m) => m,
            Err(bim_geom::GeomError::NoGeometry(_)) => bim_geom::Mesh::default(),
            Err(err) => return Err(err.into()),
        };
        let line = lines.iter().find(|l| l.element == e.id);
        elements.push(ElementOut {
            id: e.id,
            name: e.name.clone(),
            discipline: format!("{:?}", e.discipline),
            phase: e.phase.0,
            start_day: schedule.element_start_day(e).unwrap_or(0),
            cost_usd: line.map(|l| l.cost.dollars()).unwrap_or(0.0),
            qty: line.map(|l| l.qty).unwrap_or(0.0),
            unit: line.map(|l| format!("{:?}", l.unit)).unwrap_or_default(),
            cost_code: e.cost_code.clone(),
            positions: m.positions.iter().flat_map(|v| [v.x, v.y, v.z]).collect(),
            normals: m.normals.iter().flat_map(|v| [v.x, v.y, v.z]).collect(),
            indices: m.indices,
        });
    }
    let clashes = bim_geom::clashes(project, CLASH_TOLERANCE_M3)
        .into_iter()
        .map(|c| ClashOut {
            a: c.a,
            b: c.b,
            min: c.region.min.to_array(),
            max: c.region.max.to_array(),
            overlap_m3: c.overlap_m3,
        })
        .collect();
    let (rule_set, code_edition, rules_verified, advisories) = match rules {
        Some(rs) => (
            rs.id.clone(),
            rs.code_edition.clone(),
            rs.verified.clone(),
            {
                let mut adv = Vec::new();
                // a rule file for another jurisdiction must not pass silently
                if rs.id != project.rule_set {
                    adv.push(bim_rules::Advisory {
                        rule_id: "rule_set.mismatch".into(),
                        cite: String::new(),
                        severity: bim_rules::Severity::Warning,
                        subject: project.name.clone(),
                        message: format!(
                            "project specifies rule set '{}' but was evaluated against '{}'",
                            project.rule_set, rs.id
                        ),
                    });
                }
                adv.extend(rs.evaluate(project));
                adv
            },
        ),
        None => ("none".into(), "no rule file".into(), "n/a".into(), vec![]),
    };
    let mut advisories = advisories;
    // an unpriced cost code silently totals $0; surface it
    for e in &project.elements {
        if !project.unit_costs.contains_key(&e.cost_code) {
            advisories.push(bim_rules::Advisory {
                rule_id: "cost.missing_rate".into(),
                cite: String::new(),
                severity: bim_rules::Severity::Info,
                subject: e.name.clone(),
                message: format!("no unit cost for cost code '{}'; priced at $0", e.cost_code),
            });
        }
    }
    let spaces = project
        .spaces
        .iter()
        .map(|s| SpaceOut {
            name: s.name.clone(),
            occupancy: format!("{:?}", s.occupancy),
            area_m2: s.area_m2(),
            exits: bim_rules::egress_exits(project, s),
            bounds: s.bounds,
        })
        .collect();
    Ok(Report {
        spaces,
        project: project.name.clone(),
        rule_set,
        code_edition,
        rules_verified,
        total_days: schedule.total_days,
        total_cost_usd: lines.iter().map(|l| l.cost).sum::<Cents>().dollars(),
        phases,
        elements,
        clashes,
        advisories,
    })
}

/// JSON-in / JSON-out convenience for FFI and CLI.
pub fn build_json(project_json: &str, rules_json: Option<&str>) -> Result<String, ReportError> {
    let project = Project::from_json(project_json)?;
    let rules = rules_json.map(bim_rules::RuleSet::from_json).transpose()?;
    Ok(serde_json::to_string(&build(&project, rules.as_ref())?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    const RS: &str = include_str!("../../../rules/IN-2026.json");
    #[test]
    fn sample_report_shape() {
        let r = build(
            &Project::sample(),
            Some(&bim_rules::RuleSet::from_json(RS).unwrap()),
        )
        .unwrap();
        assert_eq!(r.total_days, 57);
        assert_eq!(r.elements.len(), 12);
        assert_eq!(r.advisories.len(), 2);
        assert_eq!(r.clashes.len(), 4);
        assert_eq!(r.spaces.len(), 1);
        assert_eq!(r.spaces[0].exits, 2);
        assert_eq!(r.spaces[0].area_m2, 96.0);
        // walls are net of door openings
        assert!((r.phases.iter().map(|p| p.cost_usd).sum::<f64>() - 29442.46).abs() < 1e-6);
    }
    #[test]
    fn json_round_trip_matches_struct() {
        let p = Project::sample();
        let j = build_json(&p.to_json().unwrap(), Some(RS)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["total_days"], 57);
        assert_eq!(v["elements"].as_array().unwrap().len(), 12);
    }
    #[test]
    fn rule_set_mismatch_is_flagged() {
        let rs = bim_rules::RuleSet::from_json(RS).unwrap();
        let mut p = Project::sample();
        let ok = build(&p, Some(&rs)).unwrap();
        assert!(!ok
            .advisories
            .iter()
            .any(|a| a.rule_id == "rule_set.mismatch"));
        assert!((ok.total_cost_usd - 29442.46).abs() < 1e-6);
        p.rule_set = "IBC-2024".into();
        let r = build(&p, Some(&rs)).unwrap();
        assert_eq!(r.advisories[0].rule_id, "rule_set.mismatch");
        assert_eq!(r.advisories.len(), ok.advisories.len() + 1);
    }
    #[test]
    fn external_mesh_element_is_kept_and_costed() {
        let mut p = Project::sample();
        let mut e = Element::new(
            "AHU",
            ElementKind::Mesh {
                asset: "ahu.glb".into(),
                origin: Default::default(),
            },
            Discipline::Mechanical,
            PhaseId(3),
        );
        e.cost_code = "D3040".into();
        p.add(e);
        p.unit_costs.insert("D3040".into(), Cents(500_000));
        let r = build(&p, None).unwrap();
        let ahu = r.elements.iter().find(|e| e.name == "AHU").unwrap();
        assert!(ahu.positions.is_empty() && ahu.indices.is_empty());
        assert_eq!(ahu.cost_usd, 5000.0);
        let by_elements: f64 = r.elements.iter().map(|e| e.cost_usd).sum();
        assert!((by_elements - r.total_cost_usd).abs() < 1e-6);
    }
    #[test]
    fn unpriced_cost_code_is_flagged() {
        let mut p = Project::sample();
        p.elements[0].cost_code = "Z9999".into();
        let r = build(&p, None).unwrap();
        assert_eq!(r.advisories.len(), 1);
        assert_eq!(r.advisories[0].rule_id, "cost.missing_rate");
    }
    #[test]
    fn invalid_project_is_error() {
        let mut p = Project::sample();
        p.elements[0].phase = PhaseId(99);
        assert!(matches!(build(&p, None), Err(ReportError::Invalid(_))));
    }
}
