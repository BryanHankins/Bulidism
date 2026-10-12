//! Code analysis sheet (the "code data block" architects put on the cover sheet): occupant load
//! by space, level and building; exits and egress capacity per story; plumbing fixtures
//! required. Values come from the rule set's `code_summary` parameters and carry their
//! provenance; like every rule here this is advisory, not a compliance determination.
use bim_core::*;
use bim_rules::{Advisory, Bound, Measure, RuleSet, Severity};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LevelEgress {
    /// Level id (None: the whole building when there are no levels, or unassigned spaces).
    pub level: Option<u32>,
    pub name: String,
    pub elevation_m: Option<f32>,
    pub area_m2: f32,
    pub occupant_load: u32,
    /// Spaces on this level whose occupancy has no occupant-load factor (not counted).
    pub spaces_without_factor: usize,
    pub exits_required: u32,
    pub exits_provided: u32,
    pub door_width_required_m: f32,
    pub door_width_provided_m: f32,
    /// Stair capacity applies to stories above the lowest; None for the lowest.
    pub stair_width_required_m: Option<f32>,
    pub stair_width_provided_m: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct OccupancyOut {
    pub occupancy: String,
    pub area_m2: f32,
    pub occupant_load: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlumbingOut {
    pub occupancy: String,
    pub fixture: String,
    pub occupant_load: u32,
    /// Per sex when `per_sex`, else for the whole load.
    pub per_group: u32,
    pub required: u32,
    pub per_sex: bool,
    pub verified: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeSummary {
    pub verified: String,
    pub cite_exits: String,
    pub cite_capacity: String,
    pub cite_plumbing: String,
    pub construction_type: Option<String>,
    pub sprinklered: bool,
    pub voice_alarm: bool,
    /// Reduced capacity factors apply (sprinklered AND voice/alarm).
    pub reduced_factors: bool,
    pub stories: usize,
    /// Top of the model above the lowest level (or above 0 without levels).
    pub height_m: Option<f32>,
    pub total_area_m2: f32,
    pub total_occupant_load: u32,
    pub occupancies: Vec<OccupancyOut>,
    pub levels: Vec<LevelEgress>,
    pub plumbing: Vec<PlumbingOut>,
    pub notes: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn finding(
    rule: &str,
    cite: &str,
    subject: &str,
    check: String,
    measured: f32,
    limit: f32,
    measure: Measure,
    message: String,
) -> Advisory {
    Advisory {
        rule_id: rule.into(),
        cite: cite.into(),
        severity: Severity::Warning,
        subject: subject.into(),
        message,
        element: None,
        check: Some(check),
        measured: Some(measured as f64),
        limit: Some(limit as f64),
        bound: Some(Bound::Min),
        measure: Some(measure),
    }
}

/// The analysis plus its failures as findings; None when the rule set has no `code_summary`.
/// `top_y`: highest point of the model's geometry, for the building height.
pub fn summarize(
    p: &Project,
    rs: &RuleSet,
    top_y: Option<f32>,
) -> Option<(CodeSummary, Vec<Advisory>)> {
    let c = rs.code_summary.as_ref()?;
    let b = &p.building;
    let reduced = b.sprinklered && b.voice_alarm;
    let (stair_f, other_f) = if reduced {
        (
            c.stair_m_per_occupant_reduced,
            c.other_m_per_occupant_reduced,
        )
    } else {
        (c.stair_m_per_occupant, c.other_m_per_occupant)
    };
    let ded = rs.door_clear_deduction();
    let load_of = |s: &Space| {
        rs.occupant_factor(s.occupancy)
            .map(|f| (s.area_m2() / f).ceil() as u32)
    };

    // groups: each level (lowest first), then spaces on no level; or the whole building
    let mut levels: Vec<&Level> = p.levels.iter().collect();
    levels.sort_by(|a, b| {
        a.elevation_m
            .total_cmp(&b.elevation_m)
            .then(a.id.cmp(&b.id))
    });
    let mut groups: Vec<(Option<&Level>, String, Vec<&Space>)> = Vec::new();
    if levels.is_empty() {
        groups.push((None, "Building".into(), p.spaces.iter().collect()));
    } else {
        for l in &levels {
            groups.push((
                Some(*l),
                l.name.clone(),
                p.spaces.iter().filter(|s| s.level == Some(l.id)).collect(),
            ));
        }
        let loose: Vec<&Space> = p.spaces.iter().filter(|s| s.level.is_none()).collect();
        if !loose.is_empty() {
            groups.push((None, "Spaces on no level".into(), loose));
        }
    }

    let mut out_levels = Vec::new();
    let mut findings = Vec::new();
    let lowest = levels.first().map(|l| l.id);
    for (lvl, name, spaces) in &groups {
        let load: u32 = spaces.iter().filter_map(|s| load_of(s)).sum();
        let missing = spaces.iter().filter(|s| load_of(s).is_none()).count();
        // distinct doors marked egress that serve this group's spaces
        let mut doors: Vec<ElementId> = spaces
            .iter()
            .flat_map(|s| s.egress_doors.iter().copied())
            .collect();
        doors.sort_by_key(|d| d.0);
        doors.dedup();
        let door_widths: Vec<f32> = doors
            .iter()
            .filter_map(|d| match p.element(*d).map(|e| &e.kind) {
                Ok(ElementKind::Door {
                    width,
                    is_egress: true,
                    ..
                }) => Some((width - ded).max(0.0)),
                _ => None,
            })
            .collect();
        let exits_required = c.exits_required(load);
        let door_req = load as f32 * other_f;
        let door_prov: f32 = door_widths.iter().sum();
        // stairs serving a story arrive at it: their top is within 0.3 m of the level
        let (stair_req, stair_prov) = match lvl {
            Some(l) if Some(l.id) != lowest => {
                let prov = p
                    .elements
                    .iter()
                    .filter_map(|e| match e.kind {
                        ElementKind::Stair {
                            base,
                            total_rise,
                            width,
                            ..
                        } if (base.y + total_rise - l.elevation_m).abs() <= 0.3 => Some(width),
                        _ => None,
                    })
                    .sum();
                (Some(load as f32 * stair_f), prov)
            }
            _ => (None, 0.0),
        };
        let story = |what: &str| format!("{what} from {name} (occupant load {load})");
        if load > c.single_exit_screen_load && (door_widths.len() as u32) < exits_required {
            findings.push(finding("egress.exits_per_story", &c.cite_exits, name, story("exits"), door_widths.len() as f32, exits_required as f32, Measure::Count,
                format!("{} exit(s) from {name} for an occupant load of {load}; {exits_required} required", door_widths.len())));
        }
        if load > 0 && door_prov + 1e-4 < door_req {
            findings.push(finding("egress.capacity.doors", &c.cite_capacity, name, story("egress door capacity"), door_prov, door_req, Measure::Length,
                format!("egress doors from {name} give {door_prov:.3} m clear; {load} occupants need {door_req:.3} m")));
        }
        if let Some(req) = stair_req {
            if load > 0 && stair_prov + 1e-4 < req {
                findings.push(finding("egress.capacity.stairs", &c.cite_capacity, name, story("stair capacity"), stair_prov, req, Measure::Length,
                    format!("stairs serving {name} total {stair_prov:.3} m wide; {load} occupants need {req:.3} m")));
            }
        }
        out_levels.push(LevelEgress {
            level: lvl.map(|l| l.id.0),
            name: name.clone(),
            elevation_m: lvl.map(|l| l.elevation_m),
            area_m2: spaces.iter().map(|s| s.area_m2()).sum(),
            occupant_load: load,
            spaces_without_factor: missing,
            exits_required,
            exits_provided: door_widths.len() as u32,
            door_width_required_m: door_req,
            door_width_provided_m: door_prov,
            stair_width_required_m: stair_req,
            stair_width_provided_m: stair_prov,
        });
    }

    // building totals by occupancy, and plumbing from them
    let mut occs: Vec<OccupancyOut> = Vec::new();
    for s in &p.spaces {
        let name = format!("{:?}", s.occupancy);
        let i = occs
            .iter()
            .position(|o| o.occupancy == name)
            .unwrap_or_else(|| {
                occs.push(OccupancyOut {
                    occupancy: name.clone(),
                    area_m2: 0.0,
                    occupant_load: 0,
                });
                occs.len() - 1
            });
        occs[i].area_m2 += s.area_m2();
        occs[i].occupant_load += load_of(s).unwrap_or(0);
    }
    let plumbing: Vec<PlumbingOut> = c
        .plumbing
        .iter()
        .filter_map(|row| {
            let occ = format!("{:?}", row.occupancy);
            let load = occs.iter().find(|o| o.occupancy == occ)?.occupant_load;
            (load > 0).then(|| PlumbingOut {
                occupancy: occ,
                fixture: row.fixture.clone(),
                occupant_load: load,
                per_group: row.per_group(load),
                required: row.required(load),
                per_sex: row.per_sex,
                verified: row.verified.clone(),
            })
        })
        .collect();

    let base = levels.first().map(|l| l.elevation_m).unwrap_or(0.0);
    let mut notes = vec![
        format!("Single-exit stories (Table 1006.3.4) are not evaluated; stories with {} or fewer occupants raise no exit-count finding.", c.single_exit_screen_load),
        "Allowable area and height (IBC Chapter 5) are not evaluated; construction type is recorded only.".into(),
        "Stair capacity counts stairs arriving at each story (top within 0.3 m of the level); travel distance and common path are not evaluated.".into(),
    ];
    let missing: usize = out_levels.iter().map(|l| l.spaces_without_factor).sum();
    if missing > 0 {
        notes.push(format!("{missing} space(s) have an occupancy with no occupant-load factor in this rule set; their load is not counted."));
    }
    if b.sprinklered && !b.voice_alarm {
        notes.push("Reduced egress capacity factors need both sprinklers and an emergency voice/alarm system; full factors used.".into());
    }
    let with_ratios: Vec<String> = {
        let mut v: Vec<String> = c
            .plumbing
            .iter()
            .map(|r| format!("{:?}", r.occupancy))
            .collect();
        v.dedup();
        v
    };
    notes.push(format!(
        "Plumbing ratios are provided for: {}. Per-sex counts split the load evenly; exemptions (e.g. small occupant loads) are not evaluated.",
        if with_ratios.is_empty() { "none".to_string() } else { with_ratios.join(", ") }
    ));

    Some((
        CodeSummary {
            verified: c.verified.clone(),
            cite_exits: c.cite_exits.clone(),
            cite_capacity: c.cite_capacity.clone(),
            cite_plumbing: c.cite_plumbing.clone(),
            construction_type: b.construction_type.clone(),
            sprinklered: b.sprinklered,
            voice_alarm: b.voice_alarm,
            reduced_factors: reduced,
            stories: levels.len(),
            height_m: top_y.map(|t| (t - base).max(0.0)),
            total_area_m2: p.spaces.iter().map(|s| s.area_m2()).sum(),
            total_occupant_load: occs.iter().map(|o| o.occupant_load).sum(),
            occupancies: occs,
            levels: out_levels,
            plumbing,
            notes,
        },
        findings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const RS: &str = include_str!("../../../rules/IN-2026.json");

    #[test]
    fn sample_code_sheet() {
        let rs = RuleSet::from_json(RS).unwrap();
        let (s, found) = summarize(&Project::sample(), &rs, Some(3.9)).unwrap();
        assert!(found.is_empty(), "{found:?}");
        assert_eq!((s.stories, s.total_occupant_load), (1, 7)); // 96 m2 Business at 13.9 m2/occupant
        assert!((s.height_m.unwrap() - 3.7).abs() < 1e-5); // beam top 3.9 above Level 1 at 0.2
        let l = &s.levels[0];
        assert_eq!(
            (l.name.as_str(), l.exits_required, l.exits_provided),
            ("Level 1", 2, 2)
        );
        // clear widths: (0.75 - 0.064) + (0.9 - 0.064); required 7 x 0.2 in
        assert!(
            (l.door_width_provided_m - 1.522).abs() < 1e-4
                && (l.door_width_required_m - 7.0 * 0.00508).abs() < 1e-6
        );
        assert_eq!(l.stair_width_required_m, None); // lowest story
        let fx: Vec<(&str, u32)> = s
            .plumbing
            .iter()
            .map(|p| (p.fixture.as_str(), p.required))
            .collect();
        assert_eq!(
            fx,
            vec![
                ("Water closets", 2),
                ("Lavatories", 2),
                ("Drinking fountains", 1),
                ("Service sinks", 1)
            ]
        );
    }

    #[test]
    fn upper_story_findings_and_reduced_factors() {
        let rs = RuleSet::from_json(RS).unwrap();
        let mut p = Project::sample();
        p.levels.push(Level {
            id: LevelId(1),
            name: "Level 2".into(),
            elevation_m: 3.2,
        });
        // the sample stair (base 0.2, rise 3.0, 1.2 m wide) arrives at Level 2
        let door = p
            .elements
            .iter()
            .find(|e| e.name == "Rear exit")
            .unwrap()
            .id;
        let mut big = Space::new("Open plan L2", Occupancy::Business, 1000.0); // 72 occupants
        big.level = Some(LevelId(1));
        big.egress_doors = vec![door];
        p.spaces.push(big);
        let (s, found) = summarize(&p, &rs, None).unwrap();
        let l2 = &s.levels[1];
        assert_eq!(
            (l2.occupant_load, l2.exits_required, l2.exits_provided),
            (72, 2, 1)
        );
        assert!(
            (l2.stair_width_required_m.unwrap() - 72.0 * 0.00762).abs() < 1e-5
                && (l2.stair_width_provided_m - 1.2).abs() < 1e-6
        );
        let ids: Vec<&str> = found.iter().map(|a| a.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["egress.exits_per_story"], "{found:?}");
        assert_eq!(
            (found[0].measured, found[0].limit, found[0].subject.as_str()),
            (Some(1.0), Some(2.0), "Level 2")
        );
        // sprinklers alone keep full factors; with voice/alarm too they drop
        p.building.sprinklered = true;
        assert!(!summarize(&p, &rs, None).unwrap().0.reduced_factors);
        p.building.voice_alarm = true;
        let (s, _) = summarize(&p, &rs, None).unwrap();
        assert!(
            s.reduced_factors
                && (s.levels[1].stair_width_required_m.unwrap() - 72.0 * 0.00508).abs() < 1e-5
        );
    }

    #[test]
    fn no_parameters_no_sheet() {
        let mut rs = RuleSet::from_json(RS).unwrap();
        rs.code_summary = None;
        assert!(summarize(&Project::sample(), &rs, None).is_none());
    }
}
