//! Version comparison: what changed between a baseline model (e.g. the last issued set) and
//! the current one, and what that did to the results. Elements match by id (an IFC re-import
//! keeps ids, since they come from GlobalIds), spaces by name, findings by rule + element (or
//! subject) + check, clashes by element pair.
use crate::{build, Report, ReportError};
use bim_core::*;
use bim_rules::{Advisory, Severity};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Change status of an element, space, finding or clash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElementChange {
    pub id: ElementId,
    pub name: String,
    pub status: Status,
    /// What changed: "geometry", "name", "discipline", "phase", "schedule", "cost code",
    /// "material", "level", "LOD". Empty for added/removed.
    pub aspects: Vec<String>,
    pub cost_before_usd: f64,
    pub cost_after_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceChange {
    pub name: String,
    pub status: Status,
    pub aspects: Vec<String>,
}

/// A finding that appeared ("added"), went away ("removed") or persists with a different
/// measured value ("changed"). Only errors and warnings: notes are not issues.
#[derive(Debug, Clone, Serialize)]
pub struct FindingChange {
    pub status: Status,
    /// The finding as it stands now (or, when removed, as it stood in the baseline).
    pub finding: Advisory,
    /// Baseline measured value, for "changed".
    pub measured_before: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClashChange {
    pub status: Status,
    pub a: ElementId,
    pub b: ElementId,
    pub a_name: String,
    pub b_name: String,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct Total {
    pub label: String,
    /// "usd", "days" or "count".
    pub unit: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub baseline: String,
    /// The two versions name different rule sets (both were checked against the same file).
    pub rule_set_differs: bool,
    pub totals: Vec<Total>,
    pub elements: Vec<ElementChange>,
    pub spaces: Vec<SpaceChange>,
    pub findings: Vec<FindingChange>,
    /// Errors/warnings present in both versions with the same measured value.
    pub unchanged_findings: usize,
    pub clashes: Vec<ClashChange>,
}

fn element_aspects(a: &Element, b: &Element) -> Vec<String> {
    let mut out = Vec::new();
    let mut note = |changed: bool, what: &str| {
        if changed {
            out.push(what.to_string());
        }
    };
    note(a.kind != b.kind, "geometry");
    note(a.name != b.name, "name");
    note(a.discipline != b.discipline, "discipline");
    note(a.phase != b.phase, "phase");
    note(
        a.start_offset_days != b.start_offset_days
            || a.depends_on != b.depends_on
            || a.duration_days != b.duration_days,
        "schedule",
    );
    note(a.cost_code != b.cost_code, "cost code");
    note(a.material != b.material, "material");
    note(a.level != b.level, "level");
    note(a.lod != b.lod, "LOD");
    out
}

fn space_aspects(a: &Space, b: &Space) -> Vec<String> {
    let mut out = Vec::new();
    if a.occupancy != b.occupancy {
        out.push("occupancy".into());
    }
    if (a.area_m2() - b.area_m2()).abs() > 0.005 {
        out.push(format!("area {:.1} → {:.1} m2", a.area_m2(), b.area_m2()));
    }
    if a.egress_doors != b.egress_doors {
        out.push("egress doors".into());
    }
    if a.level != b.level {
        out.push("level".into());
    }
    out
}

/// Identity of a finding across versions. Element findings key on the element id, so a
/// renamed door keeps its finding; others (spaces, stories) on the subject text.
fn finding_key(a: &Advisory) -> (String, String, String) {
    (
        a.rule_id.clone(),
        a.element
            .map(|e| e.0.to_string())
            .unwrap_or_else(|| a.subject.clone()),
        a.check.clone().unwrap_or_default(),
    )
}

fn issues(r: &Report) -> BTreeMap<(String, String, String), &Advisory> {
    r.advisories
        .iter()
        .filter(|a| a.severity != Severity::Info)
        .map(|a| (finding_key(a), a))
        .collect()
}

fn count(r: &Report, sev: Severity) -> f64 {
    r.advisories.iter().filter(|a| a.severity == sev).count() as f64
}

/// Diff two already-built reports of `old` and `new`.
pub fn diff(old: &Project, old_r: &Report, new: &Project, new_r: &Report) -> Comparison {
    let cost = |r: &Report| -> HashMap<ElementId, (f64, String)> {
        r.elements
            .iter()
            .map(|e| (e.id, (e.cost_usd, e.name.clone())))
            .collect()
    };
    let (old_cost, new_cost) = (cost(old_r), cost(new_r));
    let old_el: HashMap<ElementId, &Element> = old.elements.iter().map(|e| (e.id, e)).collect();
    let new_ids: std::collections::HashSet<ElementId> = new.elements.iter().map(|e| e.id).collect();

    // elements, in the current model's order, then removed ones in the baseline's order
    let mut elements = Vec::new();
    for e in &new.elements {
        let after = new_cost.get(&e.id).map_or(0.0, |c| c.0);
        match old_el.get(&e.id) {
            None => elements.push(ElementChange {
                id: e.id,
                name: e.name.clone(),
                status: Status::Added,
                aspects: vec![],
                cost_before_usd: 0.0,
                cost_after_usd: after,
            }),
            Some(o) => {
                let aspects = element_aspects(o, e);
                if !aspects.is_empty() {
                    elements.push(ElementChange {
                        id: e.id,
                        name: e.name.clone(),
                        status: Status::Changed,
                        aspects,
                        cost_before_usd: old_cost.get(&e.id).map_or(0.0, |c| c.0),
                        cost_after_usd: after,
                    });
                }
            }
        }
    }
    for e in old.elements.iter().filter(|e| !new_ids.contains(&e.id)) {
        elements.push(ElementChange {
            id: e.id,
            name: e.name.clone(),
            status: Status::Removed,
            aspects: vec![],
            cost_before_usd: old_cost.get(&e.id).map_or(0.0, |c| c.0),
            cost_after_usd: 0.0,
        });
    }

    let old_sp: HashMap<&str, &Space> = old.spaces.iter().map(|s| (s.name.as_str(), s)).collect();
    let new_sp: HashMap<&str, &Space> = new.spaces.iter().map(|s| (s.name.as_str(), s)).collect();
    let mut spaces = Vec::new();
    for s in &new.spaces {
        match old_sp.get(s.name.as_str()) {
            None => spaces.push(SpaceChange {
                name: s.name.clone(),
                status: Status::Added,
                aspects: vec![],
            }),
            Some(o) => {
                let aspects = space_aspects(o, s);
                if !aspects.is_empty() {
                    spaces.push(SpaceChange {
                        name: s.name.clone(),
                        status: Status::Changed,
                        aspects,
                    });
                }
            }
        }
    }
    for s in old
        .spaces
        .iter()
        .filter(|s| !new_sp.contains_key(s.name.as_str()))
    {
        spaces.push(SpaceChange {
            name: s.name.clone(),
            status: Status::Removed,
            aspects: vec![],
        });
    }

    let (old_f, new_f) = (issues(old_r), issues(new_r));
    let mut findings = Vec::new();
    let mut unchanged_findings = 0;
    for (k, a) in &new_f {
        match old_f.get(k) {
            None => findings.push(FindingChange {
                status: Status::Added,
                finding: (*a).clone(),
                measured_before: None,
            }),
            Some(o) => match (o.measured, a.measured) {
                (Some(x), Some(y)) if (x - y).abs() > 1e-6 => findings.push(FindingChange {
                    status: Status::Changed,
                    finding: (*a).clone(),
                    measured_before: Some(x),
                }),
                _ => unchanged_findings += 1,
            },
        }
    }
    for (k, a) in &old_f {
        if !new_f.contains_key(k) {
            findings.push(FindingChange {
                status: Status::Removed,
                finding: (*a).clone(),
                measured_before: a.measured,
            });
        }
    }
    // errors first, then warnings; new before changed before resolved
    let rank = |f: &FindingChange| {
        (
            f.finding.severity != Severity::Error,
            match f.status {
                Status::Added => 0,
                Status::Changed => 1,
                Status::Removed => 2,
            },
        )
    };
    findings.sort_by_key(rank);

    let pair = |a: ElementId, b: ElementId| if a.0 <= b.0 { (a, b) } else { (b, a) };
    let old_c: HashMap<_, _> = old_r.clashes.iter().map(|c| (pair(c.a, c.b), c)).collect();
    let new_c: HashMap<_, _> = new_r.clashes.iter().map(|c| (pair(c.a, c.b), c)).collect();
    let name = |m: &HashMap<ElementId, (f64, String)>, id: ElementId| {
        m.get(&id).map(|c| c.1.clone()).unwrap_or_default()
    };
    let mut clashes = Vec::new();
    for c in &new_r.clashes {
        if !old_c.contains_key(&pair(c.a, c.b)) {
            clashes.push(ClashChange {
                status: Status::Added,
                a: c.a,
                b: c.b,
                a_name: name(&new_cost, c.a),
                b_name: name(&new_cost, c.b),
                min: c.min,
                max: c.max,
            });
        }
    }
    for c in &old_r.clashes {
        if !new_c.contains_key(&pair(c.a, c.b)) {
            clashes.push(ClashChange {
                status: Status::Removed,
                a: c.a,
                b: c.b,
                a_name: name(&old_cost, c.a),
                b_name: name(&old_cost, c.b),
                min: c.min,
                max: c.max,
            });
        }
    }

    let load = |r: &Report| {
        r.spaces
            .iter()
            .filter_map(|s| s.occupant_load)
            .map(f64::from)
            .sum::<f64>()
    };
    let total = |label: &str, unit: &str, before: f64, after: f64| Total {
        label: label.into(),
        unit: unit.into(),
        before,
        after,
    };
    let totals = vec![
        total(
            "Elements",
            "count",
            old_r.elements.len() as f64,
            new_r.elements.len() as f64,
        ),
        total("Cost", "usd", old_r.total_cost_usd, new_r.total_cost_usd),
        total(
            "Duration",
            "days",
            old_r.total_days.into(),
            new_r.total_days.into(),
        ),
        total("Occupant load", "count", load(old_r), load(new_r)),
        total(
            "Errors",
            "count",
            count(old_r, Severity::Error),
            count(new_r, Severity::Error),
        ),
        total(
            "Warnings",
            "count",
            count(old_r, Severity::Warning),
            count(new_r, Severity::Warning),
        ),
        total(
            "Clashes",
            "count",
            old_r.clashes.len() as f64,
            new_r.clashes.len() as f64,
        ),
    ];

    Comparison {
        baseline: old.name.clone(),
        rule_set_differs: old.rule_set != new.rule_set,
        totals,
        elements,
        spaces,
        findings,
        unchanged_findings,
        clashes,
    }
}

/// Build both versions against the same rule set and diff them.
pub fn compare(
    old: &Project,
    new: &Project,
    rules: Option<&bim_rules::RuleSet>,
) -> Result<Comparison, ReportError> {
    let old_r = build(old, rules)?;
    let new_r = build(new, rules)?;
    Ok(diff(old, &old_r, new, &new_r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bim_rules::RuleSet;
    const RS: &str = include_str!("../../../rules/IN-2026.json");

    #[test]
    fn identical_versions_have_no_changes() {
        let rs = RuleSet::from_json(RS).unwrap();
        let p = Project::sample();
        let c = compare(&p, &p, Some(&rs)).unwrap();
        assert!(c.elements.is_empty() && c.spaces.is_empty() && c.clashes.is_empty());
        assert!(c.findings.is_empty());
        assert!(c.unchanged_findings > 0, "the sample has findings");
        assert!(c.totals.iter().all(|t| t.before == t.after));
    }

    #[test]
    fn widening_a_failing_door_resolves_its_finding() {
        let rs = RuleSet::from_json(RS).unwrap();
        let old = Project::sample();
        let mut new = old.clone();
        // the sample's main entry fails clear width (leaf 0.75 m); widen it to 0.95 m
        let d = new
            .elements
            .iter_mut()
            .find(|e| e.name == "Main entry")
            .unwrap();
        let id = d.id;
        if let ElementKind::Door { width, .. } = &mut d.kind {
            *width = 0.95;
        }
        let c = compare(&old, &new, Some(&rs)).unwrap();
        assert_eq!(c.elements.len(), 1);
        assert_eq!(c.elements[0].id, id);
        assert_eq!(c.elements[0].aspects, vec!["geometry"]);
        let resolved: Vec<_> = c
            .findings
            .iter()
            .filter(|f| f.status == Status::Removed)
            .collect();
        assert_eq!(resolved.len(), 1, "{:?}", c.findings);
        assert_eq!(resolved[0].finding.element, Some(id));
        assert!(c.findings.iter().all(|f| f.status != Status::Added));
        let errors = c.totals.iter().find(|t| t.label == "Errors").unwrap();
        assert_eq!(errors.after, errors.before - 1.0);
    }

    #[test]
    fn narrowing_further_is_a_changed_finding() {
        let rs = RuleSet::from_json(RS).unwrap();
        let old = Project::sample();
        let mut new = old.clone();
        let d = new
            .elements
            .iter_mut()
            .find(|e| e.name == "Main entry")
            .unwrap();
        if let ElementKind::Door { width, .. } = &mut d.kind {
            *width = 0.70;
        }
        d.name = "Front door".into();
        let c = compare(&old, &new, Some(&rs)).unwrap();
        assert_eq!(c.elements[0].aspects, vec!["geometry", "name"]);
        let ch: Vec<_> = c
            .findings
            .iter()
            .filter(|f| f.status == Status::Changed)
            .collect();
        assert_eq!(
            ch.len(),
            1,
            "same door, same check: changed, not new + resolved"
        );
        assert!((ch[0].measured_before.unwrap() - 0.686).abs() < 1e-3);
        assert!((ch[0].finding.measured.unwrap() - 0.636).abs() < 1e-3);
    }

    #[test]
    fn added_removed_elements_spaces_and_clashes() {
        let rs = RuleSet::from_json(RS).unwrap();
        let old = Project::sample();
        let mut new = old.clone();
        let i = new
            .elements
            .iter()
            .position(|e| e.name == "Column 4")
            .unwrap();
        let removed = new.elements.remove(i);
        // a column dropped inside the first wall: a new clash
        let wall = new
            .elements
            .iter()
            .find(|e| matches!(e.kind, ElementKind::Wall { .. }))
            .unwrap()
            .clone();
        let ElementKind::Wall { start, .. } = wall.kind else {
            unreachable!()
        };
        let mut col = Element::new(
            "New column",
            ElementKind::Column {
                base: glam::vec3(start.x + 3.0, start.y, start.z),
                width: 0.4,
                depth: 0.4,
                height: 3.0,
            },
            Discipline::Structural,
            wall.phase,
        );
        col.cost_code = "B1010".into();
        col.level = wall.level;
        let col_id = col.id;
        new.elements.push(col);
        new.spaces[0].floor_area_m2 += 10.0;
        new.spaces[0].bounds = None;
        new.spaces
            .push(Space::new("Annex", Occupancy::Business, 30.0));
        let c = compare(&old, &new, Some(&rs)).unwrap();
        let st = |s| {
            c.elements
                .iter()
                .filter(|e| e.status == s)
                .map(|e| e.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(st(Status::Added), vec![col_id]);
        assert_eq!(st(Status::Removed), vec![removed.id]);
        assert!(c.clashes.iter().any(|k| k.status == Status::Added
            && (k.a == col_id || k.b == col_id)
            && !k.a_name.is_empty()));
        let sp: Vec<_> = c
            .spaces
            .iter()
            .map(|s| (s.name.as_str(), s.status))
            .collect();
        assert_eq!(
            sp,
            vec![
                (old.spaces[0].name.as_str(), Status::Changed),
                ("Annex", Status::Added)
            ]
        );
        assert!(c.spaces[0].aspects[0].starts_with("area"));
    }
}
