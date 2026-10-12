//! Data-driven advisory rules. Rule text is NOT stored (ICC copyright);
//! only parameters + citation. Output is advisory, never "compliant".
use bim_core::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum Check {
    /// Doors are modelled by leaf width; clear width = leaf - `leaf_to_clear_deduction_m`
    /// (stop + leaf thickness when open 90 degrees). Required, not defaulted: rule
    /// fields can't deny unknown keys (flatten), so a misspelt optional key would
    /// silently become 0.
    EgressDoorMinWidth {
        min_clear_width_m: f32,
        leaf_to_clear_deduction_m: f32,
    },
    DoorMinHeight {
        min_height_m: f32,
    },
    StairGeometry {
        max_riser_m: f32,
        min_riser_m: f32,
        min_tread_m: f32,
        min_width_m: f32,
    },
    /// Single door leaf wider than `max_leaf_width_m`.
    DoorMaxLeafWidth {
        max_leaf_width_m: f32,
    },
    /// Vertical rise of one stair flight above `max_rise_m` (a landing is required).
    StairMaxRisePerFlight {
        max_rise_m: f32,
    },
    /// Ramp slope steeper than 1:`min_run_per_rise`, or a single run rising more than `max_rise_m`.
    RampSlope {
        min_run_per_rise: f32,
        max_rise_m: f32,
        min_width_m: f32,
    },
    /// Occupant load = area / factor; spaces over the single-exit limit with one exit are flagged.
    OccupantLoadExits {
        occupancy: Occupancy,
        area_per_occupant_m2: f32,
        max_occupants_single_exit: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    /// e.g. "IBC 2024 §1010.1.1" — citation only, no code text.
    pub cite: String,
    pub severity: Severity,
    /// Per-rule provenance: date + source the parameters were checked against, or None if unverified.
    #[serde(default)]
    pub verified: Option<String>,
    #[serde(flatten)]
    pub check: Check,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub id: String,
    pub jurisdiction: String,
    pub code_edition: String,
    /// Date the parameters were last verified against the primary source, or "unverified".
    pub verified: String,
    pub reviewer: Option<String>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Advisory {
    pub rule_id: String,
    pub cite: String,
    pub severity: Severity,
    pub subject: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("rule file: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Distinct referenced doors that are actually marked egress.
pub fn egress_exits(p: &Project, s: &Space) -> usize {
    let mut ids: Vec<ElementId> = s
        .egress_doors
        .iter()
        .copied()
        .filter(|d| {
            matches!(
                p.element(*d).map(|e| &e.kind),
                Ok(ElementKind::Door {
                    is_egress: true,
                    ..
                })
            )
        })
        .collect();
    ids.sort_by_key(|d| d.0);
    ids.dedup();
    ids.len()
}

impl RuleSet {
    pub fn from_json(s: &str) -> Result<Self, RuleError> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn evaluate(&self, p: &Project) -> Vec<Advisory> {
        let mut out = Vec::new();
        for r in &self.rules {
            let mut hits: Vec<(String, String)> = Vec::new();
            match &r.check {
                Check::EgressDoorMinWidth {
                    min_clear_width_m,
                    leaf_to_clear_deduction_m: ded,
                } => {
                    for e in &p.elements {
                        if let ElementKind::Door {
                            width,
                            is_egress: true,
                            ..
                        } = e.kind
                        {
                            let clear = width - ded;
                            if clear < *min_clear_width_m {
                                hits.push((e.name.clone(), format!("egress door clear width {clear:.3} m (leaf {width:.3} m - {ded:.3} m) < {min_clear_width_m:.3} m")));
                            }
                        }
                    }
                }
                Check::DoorMinHeight { min_height_m } => {
                    for e in &p.elements {
                        if let ElementKind::Door { height, .. } = e.kind {
                            if height < *min_height_m {
                                hits.push((
                                    e.name.clone(),
                                    format!("door height {height:.3} m < {min_height_m:.3} m"),
                                ));
                            }
                        }
                    }
                }
                Check::DoorMaxLeafWidth { max_leaf_width_m } => {
                    for e in &p.elements {
                        if let ElementKind::Door { width, .. } = e.kind {
                            if width > *max_leaf_width_m {
                                hits.push((
                                    e.name.clone(),
                                    format!(
                                        "door leaf width {width:.3} m > {max_leaf_width_m:.3} m"
                                    ),
                                ));
                            }
                        }
                    }
                }
                Check::StairMaxRisePerFlight { max_rise_m } => {
                    for e in &p.elements {
                        if let ElementKind::Stair { total_rise, .. } = e.kind {
                            if total_rise > *max_rise_m {
                                hits.push((e.name.clone(), format!("flight rise {total_rise:.3} m > {max_rise_m:.3} m without a landing")));
                            }
                        }
                    }
                }
                Check::StairGeometry {
                    max_riser_m,
                    min_riser_m,
                    min_tread_m,
                    min_width_m,
                } => {
                    for e in &p.elements {
                        if let ElementKind::Stair {
                            run_length,
                            total_rise,
                            width,
                            riser_count,
                            ..
                        } = e.kind
                        {
                            if riser_count == 0 {
                                hits.push((e.name.clone(), "stair has zero risers".into()));
                                continue;
                            }
                            let riser = total_rise / riser_count as f32;
                            let tread = run_length / riser_count.saturating_sub(1).max(1) as f32;
                            if riser > *max_riser_m || riser < *min_riser_m {
                                hits.push((e.name.clone(), format!("riser {riser:.3} m outside [{min_riser_m:.3}, {max_riser_m:.3}]")));
                            }
                            if tread < *min_tread_m {
                                hits.push((
                                    e.name.clone(),
                                    format!("tread {tread:.3} m < {min_tread_m:.3} m"),
                                ));
                            }
                            if width < *min_width_m {
                                hits.push((
                                    e.name.clone(),
                                    format!("stair width {width:.3} m < {min_width_m:.3} m"),
                                ));
                            }
                        }
                    }
                }
                Check::RampSlope {
                    min_run_per_rise,
                    max_rise_m,
                    min_width_m,
                } => {
                    for e in &p.elements {
                        if let ElementKind::Ramp {
                            run_length,
                            rise,
                            width,
                            ..
                        } = e.kind
                        {
                            if rise > 0.0 && run_length / rise < *min_run_per_rise {
                                hits.push((
                                    e.name.clone(),
                                    format!(
                                        "ramp slope 1:{:.1} steeper than 1:{min_run_per_rise:.0}",
                                        run_length / rise
                                    ),
                                ));
                            }
                            if rise > *max_rise_m {
                                hits.push((
                                    e.name.clone(),
                                    format!("ramp rise {rise:.3} m > {max_rise_m:.3} m per run"),
                                ));
                            }
                            if width < *min_width_m {
                                hits.push((
                                    e.name.clone(),
                                    format!("ramp width {width:.3} m < {min_width_m:.3} m"),
                                ));
                            }
                        }
                    }
                }
                Check::OccupantLoadExits {
                    occupancy,
                    area_per_occupant_m2,
                    max_occupants_single_exit,
                } => {
                    for s in p.spaces.iter().filter(|s| s.occupancy == *occupancy) {
                        let load = (s.area_m2() / area_per_occupant_m2).ceil() as u32;
                        let exits = egress_exits(p, s);
                        if load > *max_occupants_single_exit && exits < 2 {
                            hits.push((s.name.clone(), format!("occupant load {load} > {max_occupants_single_exit} with {exits} exit(s)")));
                        }
                    }
                }
            }
            out.extend(hits.into_iter().map(|(subject, message)| Advisory {
                rule_id: r.id.clone(),
                cite: r.cite.clone(),
                severity: r.severity,
                subject,
                message,
            }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RS: &str = include_str!("../../../rules/IN-2026.json");
    #[test]
    fn parses_shipped_rule_file() {
        assert!(RuleSet::from_json(RS).unwrap().rules.len() >= 4);
    }
    #[test]
    fn sample_project_flags_narrow_egress_door() {
        let adv = RuleSet::from_json(RS).unwrap().evaluate(&Project::sample());
        assert!(
            adv.iter()
                .any(|a| a.rule_id == "egress.door.min_clear_width"),
            "{adv:?}"
        );
    }
    #[test]
    fn clear_width_deducts_from_leaf() {
        let rs = RuleSet::from_json(RS).unwrap();
        let mut p = Project::sample();
        let set_rear = |p: &mut Project, w: f32| {
            for e in &mut p.elements {
                if let ElementKind::Door { width, .. } = &mut e.kind {
                    if e.name == "Rear exit" {
                        *width = w;
                    }
                }
            }
        };
        let flagged = |p: &Project| {
            rs.evaluate(p)
                .iter()
                .any(|a| a.subject == "Rear exit" && a.rule_id == "egress.door.min_clear_width")
        };
        // a 0.84 m leaf passes on nominal width but not after the stop/leaf deduction
        set_rear(&mut p, 0.84);
        assert!(flagged(&p));
        set_rear(&mut p, 0.9);
        assert!(!flagged(&p));
    }
    #[test]
    fn duplicate_or_non_egress_exits_do_not_count() {
        let rs = RuleSet::from_json(RS).unwrap();
        let mut p = Project::sample();
        p.spaces[0].bounds = None;
        p.spaces[0].floor_area_m2 = 1000.0; // load 72 > 49
        let flagged = |p: &Project| {
            rs.evaluate(p)
                .iter()
                .any(|a| a.rule_id.starts_with("occupant_load"))
        };
        assert!(!flagged(&p));
        let d0 = p.spaces[0].egress_doors[0];
        p.spaces[0].egress_doors = vec![d0, d0];
        assert!(flagged(&p), "same door twice is one exit");
        let d1 = p
            .elements
            .iter()
            .find(|e| e.name == "Rear exit")
            .unwrap()
            .id;
        p.spaces[0].egress_doors = vec![d0, d1];
        for e in &mut p.elements {
            if let ElementKind::Door { is_egress, .. } = &mut e.kind {
                if e.id == d1 {
                    *is_egress = false;
                }
            }
        }
        assert!(flagged(&p), "non-egress door is not an exit");
    }
    #[test]
    fn misspelt_deduction_is_rejected() {
        let bad = RS.replace("leaf_to_clear_deduction_m", "leaf_to_clear_deduction");
        assert!(RuleSet::from_json(&bad).is_err());
    }
    #[test]
    fn unknown_field_rejected() {
        assert!(RuleSet::from_json(r#"{"id":"x","jurisdiction":"x","code_edition":"x","verified":"x","reviewer":null,"rules":[],"bogus":1}"#).is_err());
    }
    #[test]
    fn sample_flags_stair_tread_and_nothing_on_ramp() {
        let adv = RuleSet::from_json(RS).unwrap().evaluate(&Project::sample());
        assert!(
            adv.iter()
                .any(|a| a.rule_id == "stair.geometry" && a.message.contains("tread")),
            "{adv:?}"
        );
        assert!(!adv.iter().any(|a| a.rule_id == "ramp.slope"), "{adv:?}");
        assert!(
            !adv.iter().any(|a| a.rule_id.starts_with("occupant_load")),
            "two exits present: {adv:?}"
        );
    }
    #[test]
    fn wide_leaf_and_tall_flight_flagged() {
        let rs = RuleSet::from_json(RS).unwrap();
        let mut p = Project::sample();
        let wall = p
            .elements
            .iter()
            .find(|e| e.name == "South wall")
            .unwrap()
            .id;
        p.add(Element::new(
            "wide",
            ElementKind::Door {
                host: wall,
                offset: 0.0,
                width: 1.5,
                height: 2.1,
                is_egress: false,
            },
            Discipline::Architectural,
            PhaseId(3),
        ));
        p.add(Element::new(
            "tall flight",
            ElementKind::Stair {
                base: glam::Vec3::ZERO,
                run_length: 6.0,
                total_rise: 4.5,
                width: 1.2,
                riser_count: 25,
            },
            Discipline::Architectural,
            PhaseId(3),
        ));
        let adv = rs.evaluate(&p);
        assert!(
            adv.iter().any(|a| a.rule_id == "door.max_leaf_width"),
            "{adv:?}"
        );
        assert!(
            adv.iter().any(|a| a.rule_id == "stair.max_rise_per_flight"),
            "{adv:?}"
        );
    }
    #[test]
    fn steep_ramp_flagged() {
        let mut p = Project::new("r", "IN-2026");
        p.phases.push(Phase {
            id: PhaseId(0),
            name: "p".into(),
            duration_days: 1,
            depends_on: vec![],
        });
        p.add(Element::new(
            "steep",
            ElementKind::Ramp {
                base: glam::Vec3::ZERO,
                run_length: 1.0,
                rise: 0.2,
                width: 1.5,
            },
            Discipline::Site,
            PhaseId(0),
        ));
        let adv = RuleSet::from_json(RS).unwrap().evaluate(&p);
        assert_eq!(
            adv.iter().filter(|a| a.rule_id == "ramp.slope").count(),
            1,
            "{adv:?}"
        );
    }
    #[test]
    fn empty_project_no_advisories() {
        assert!(RuleSet::from_json(RS)
            .unwrap()
            .evaluate(&Project::new("e", "IN-2026"))
            .is_empty());
    }
}
