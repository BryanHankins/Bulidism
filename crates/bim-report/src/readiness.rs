//! Model readiness: data the checks depend on but the model lacks. Not code findings: each
//! item says why a result may be weak (e.g. a space with no exits assigned reads as 0 exits),
//! like an IDS / plan-review "readiness" pass before trusting the check.
use bim_core::*;
use bim_rules::RuleSet;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ReadinessItem {
    pub id: String,
    /// "warning": affects code results; "info": affects cost or presentation.
    pub severity: String,
    pub message: String,
    /// How many items are affected (0 for a project-wide flag).
    pub count: usize,
    /// Up to five example names.
    pub examples: Vec<String>,
}

fn item(id: &str, severity: &str, message: String, names: Vec<String>) -> Option<ReadinessItem> {
    (!names.is_empty()).then(|| ReadinessItem {
        id: id.into(),
        severity: severity.into(),
        message,
        count: names.len(),
        examples: names.into_iter().take(5).collect(),
    })
}

pub fn check(
    p: &Project,
    rules: Option<&RuleSet>,
    analysis: Option<&bim_egress::Analysis>,
) -> Vec<ReadinessItem> {
    let mut out = Vec::new();
    let names = |it: &mut dyn Iterator<Item = &String>| it.cloned().collect::<Vec<_>>();
    if let Some(rs) = rules {
        let mut no_factor: Vec<String> = p
            .spaces
            .iter()
            .filter(|s| rs.occupant_factor(s.occupancy).is_none())
            .map(|s| format!("{} ({:?})", s.name, s.occupancy))
            .collect();
        no_factor.sort();
        out.extend(item(
            "space.occupancy_factor",
            "warning",
            "spaces whose occupancy has no occupant-load factor in this rule set: their load counts as 0".into(),
            no_factor,
        ));
        if rs.code_summary.is_some()
            && p.building.construction_type.is_none()
            && !p.spaces.is_empty()
        {
            out.push(ReadinessItem {
                id: "building.construction_type".into(),
                severity: "info".into(),
                message:
                    "construction type not set (Edit → Building); the code analysis records it"
                        .into(),
                count: 0,
                examples: vec![],
            });
        }
    }
    let egress = |d: &ElementId| {
        matches!(
            p.element(*d).map(|e| &e.kind),
            Ok(ElementKind::Door {
                is_egress: true,
                ..
            })
        )
    };
    out.extend(item(
        "space.no_exits",
        "warning",
        "spaces with no egress door assigned: exit counts and capacity read as 0 (Edit → Spaces)"
            .into(),
        names(
            &mut p
                .spaces
                .iter()
                .filter(|s| !s.egress_doors.iter().any(egress))
                .map(|s| &s.name),
        ),
    ));
    let doors: Vec<&Element> = p
        .elements
        .iter()
        .filter(|e| matches!(e.kind, ElementKind::Door { .. }))
        .collect();
    if !p.spaces.is_empty()
        && !doors.is_empty()
        && !doors.iter().any(|d| {
            matches!(
                d.kind,
                ElementKind::Door {
                    is_egress: true,
                    ..
                }
            )
        })
    {
        out.push(ReadinessItem {
            id: "door.none_egress".into(),
            severity: "warning".into(),
            message: "no door is marked as an egress door; egress checks have nothing to measure"
                .into(),
            count: 0,
            examples: doors.iter().take(5).map(|d| d.name.clone()).collect(),
        });
    }
    if !p.levels.is_empty() {
        out.extend(item(
            "space.no_level",
            "warning",
            "spaces on no level: they are left out of the per-story egress analysis".into(),
            names(
                &mut p
                    .spaces
                    .iter()
                    .filter(|s| s.level.is_none())
                    .map(|s| &s.name),
            ),
        ));
        out.extend(item(
            "element.no_level",
            "info",
            "elements on no level (fine for site work; otherwise assign one)".into(),
            names(
                &mut p
                    .elements
                    .iter()
                    .filter(|e| e.level.is_none() && e.discipline != Discipline::Site)
                    .map(|e| &e.name),
            ),
        ));
    }
    if let (Some(rs), Some(eg)) = (rules, analysis) {
        if rs.needs_egress() {
            out.extend(item(
                "space.no_shape",
                "warning",
                "spaces with no outline or plan rectangle: travel distance and common path are not checked (draw them, or import IFC spaces with geometry)".into(),
                names(
                    &mut p
                        .spaces
                        .iter()
                        .zip(&eg.spaces)
                        .filter(|(_, e)| !e.analyzed)
                        .map(|(s, _)| &s.name),
                ),
            ));
        }
        if eg.spaces.iter().any(|e| e.analyzed) {
            out.extend(item(
                "door.no_room",
                "info",
                "doors with no room on either side: not on any egress route (is the room drawn as a space, on the door's level?)".into(),
                names(
                    &mut eg
                        .doors
                        .iter()
                        .filter(|d| d.sides == [None, None])
                        .map(|d| &d.name),
                ),
            ));
        }
    }
    out.extend(item(
        "element.no_cost_code",
        "info",
        "elements with no cost code: they are not priced".into(),
        names(
            &mut p
                .elements
                .iter()
                .filter(|e| e.cost_code.is_empty())
                .map(|e| &e.name),
        ),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const RS: &str = include_str!("../../../rules/IN-2026.json");

    #[test]
    fn sample_is_ready_except_construction_type() {
        let rs = RuleSet::from_json(RS).unwrap();
        let ids: Vec<String> = check(&Project::sample(), Some(&rs), None)
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, vec!["building.construction_type"]);
    }

    #[test]
    fn gaps_are_listed() {
        let rs = RuleSet::from_json(RS).unwrap();
        let mut p = Project::sample();
        p.building.construction_type = Some("IIB".into());
        p.spaces
            .push(Space::new("Archive", Occupancy::Storage, 20.0)); // no factor, no exits, no level
        for e in &mut p.elements {
            if let ElementKind::Door { is_egress, .. } = &mut e.kind {
                *is_egress = false;
            }
        }
        p.elements[0].cost_code.clear();
        p.elements[0].level = None; // the slab: structural, so worth a level
        let r = check(&p, Some(&rs), None);
        let ids: Vec<(&str, usize)> = r.iter().map(|i| (i.id.as_str(), i.count)).collect();
        assert_eq!(
            ids,
            vec![
                ("space.occupancy_factor", 1),
                ("space.no_exits", 2), // both spaces: their doors are no longer egress
                ("door.none_egress", 0),
                ("space.no_level", 1),
                ("element.no_level", 1),
                ("element.no_cost_code", 1),
            ]
        );
        assert_eq!(r[0].examples, vec!["Archive (Storage)"]);
    }
}
