//! Real-file corpus: models exported by authoring tools (corpus/manifest.json), imported and
//! checked end to end, compared with reviewed expected answers (corpus/expected/<id>.json).
//!
//!   bash corpus/fetch.sh                                   # download (checksummed)
//!   cargo test --release -p bim-report --test corpus -- --nocapture
//!   UPDATE_CORPUS=1 cargo test --release -p bim-report --test corpus   # rewrite expectations
//!
//! Every error/warning finding in an expected file carries a reviewed verdict:
//! - "valid": correct for this model under the rule as written
//! - "rule_limit": correct under our rule, but the code has an exception or condition we do
//!   not model (the note says which)
//! - "model_gap": the model itself lacks something (e.g. no door into a room); correct
//!   relative to the model
//! - "false_positive": wrong; a defect to fix
//!
//! UPDATE_CORPUS keeps verdicts of findings that are still there and marks new ones
//! "unreviewed", which fails the test until someone reviews them. Without the files the test
//! is skipped, unless CORPUS_REQUIRED is set (CI).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Manifest {
    models: Vec<Model>,
}
#[derive(Deserialize)]
struct Model {
    id: String,
    file: String,
    rules: String,
    /// What a user would set after importing, applied before the check.
    #[serde(default)]
    setup: Option<Setup>,
}
#[derive(Deserialize)]
struct Setup {
    /// Occupancy for every space that is not Unoccupied.
    occupancy: Option<bim_core::Occupancy>,
    /// "suggest": dwelling units from `bim_egress::suggest_dwelling_units`.
    dwelling_units: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Default)]
struct Expected {
    counts: BTreeMap<String, usize>,
    /// Rooms with no walking route to an exit.
    cut_off: Vec<String>,
    /// The longest walk to an exit, in metres to 0.1 (room, distance).
    longest_walk: Option<(String, f64)>,
    findings: Vec<Finding>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
struct Finding {
    rule: String,
    subject: String,
    check: String,
    measured: Option<f64>,
    verdict: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    note: String,
}

impl Finding {
    fn key(&self) -> (String, String, String, Option<String>) {
        (
            self.rule.clone(),
            self.subject.clone(),
            self.check.clone(),
            self.measured.map(|m| format!("{m:.3}")),
        )
    }
}

const VERDICTS: &[&str] = &["valid", "rule_limit", "model_gap", "false_positive"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn facts(model: &Model, ifc: &str) -> Expected {
    let mut r = bim_ifc::import(ifc, &model.id).expect("imports");
    if let Some(setup) = &model.setup {
        if let Some(o) = setup.occupancy {
            for s in &mut r.project.spaces {
                if s.occupancy != bim_core::Occupancy::Unoccupied {
                    s.occupancy = o;
                }
            }
        }
        if setup.dwelling_units.as_deref() == Some("suggest") {
            let units = bim_egress::suggest_dwelling_units(&r.project);
            for (s, u) in r.project.spaces.iter_mut().zip(units) {
                s.dwelling_unit = u;
            }
        }
    }
    // the rule set the user picked for the project
    r.project.rule_set = model.rules.clone();
    let p = &r.project;
    let rules = bim_rules::RuleSet::from_json(
        &std::fs::read_to_string(root().join(format!("rules/{}.json", model.rules))).unwrap(),
    )
    .unwrap();
    let report = bim_report::build(p, Some(&rules)).expect("runs");
    let mut counts = BTreeMap::new();
    let kind =
        |f: fn(&bim_core::ElementKind) -> bool| p.elements.iter().filter(|e| f(&e.kind)).count();
    use bim_core::ElementKind as K;
    counts.insert("walls".into(), kind(|k| matches!(k, K::Wall { .. })));
    counts.insert(
        "doors".into(),
        kind(|k| matches!(k, K::Door { leaves: 1.., .. })),
    );
    counts.insert(
        "passages".into(),
        kind(|k| matches!(k, K::Door { leaves: 0, .. })),
    );
    counts.insert("stairs".into(), kind(|k| matches!(k, K::Stair { .. })));
    counts.insert("windows".into(), kind(|k| matches!(k, K::Window { .. })));
    counts.insert("slabs".into(), kind(|k| matches!(k, K::Slab { .. })));
    counts.insert("levels".into(), p.levels.len());
    counts.insert("spaces".into(), p.spaces.len());
    counts.insert(
        "spaces_with_outline".into(),
        p.spaces.iter().filter(|s| s.outline.is_some()).count(),
    );
    let mut units: Vec<&str> = p
        .spaces
        .iter()
        .filter_map(|s| s.dwelling_unit.as_deref())
        .collect();
    units.sort();
    units.dedup();
    counts.insert("dwelling_units".into(), units.len());
    counts.insert(
        "egress_doors".into(),
        kind(|k| {
            matches!(
                k,
                K::Door {
                    is_egress: true,
                    ..
                }
            )
        }),
    );
    let checked: Vec<_> = report
        .spaces
        .iter()
        .filter(|s| s.outline.is_some() && s.occupancy != "Unoccupied")
        .collect();
    let mut cut_off: Vec<String> = checked
        .iter()
        .filter(|s| s.travel_m.is_none())
        .map(|s| s.name.clone())
        .collect();
    cut_off.sort();
    let longest_walk = checked
        .iter()
        .filter_map(|s| s.travel_m.map(|t| (s.name.clone(), t)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, t)| (n, (t as f64 * 10.0).round() / 10.0));
    let mut findings: Vec<Finding> = report
        .advisories
        .iter()
        .filter(|a| a.severity != bim_rules::Severity::Info)
        .map(|a| Finding {
            rule: a.rule_id.clone(),
            subject: a.subject.clone(),
            check: a.check.clone().unwrap_or_default(),
            measured: a.measured.map(|m| (m * 1000.0).round() / 1000.0),
            verdict: "unreviewed".into(),
            note: String::new(),
        })
        .collect();
    findings.sort_by_key(Finding::key);
    Expected {
        counts,
        cut_off,
        longest_walk,
        findings,
    }
}

#[test]
fn corpus() {
    let manifest: Manifest = serde_json::from_str(
        &std::fs::read_to_string(root().join("corpus/manifest.json")).unwrap(),
    )
    .unwrap();
    let update = std::env::var_os("UPDATE_CORPUS").is_some();
    let required = std::env::var_os("CORPUS_REQUIRED").is_some();
    let mut failures = Vec::new();
    println!(
        "{:<14} {:>8} {:>6} {:>10} {:>9} {:>14}",
        "model", "findings", "valid", "rule_limit", "model_gap", "false_positive"
    );
    for m in &manifest.models {
        let path = root().join("corpus/cache").join(&m.file);
        let Ok(ifc) = std::fs::read_to_string(&path) else {
            assert!(
                !required,
                "corpus file missing: {} (run corpus/fetch.sh)",
                path.display()
            );
            println!(
                "{:<14} skipped: {} not downloaded (corpus/fetch.sh)",
                m.id, m.file
            );
            continue;
        };
        let mut got = facts(m, &ifc);
        let exp_path = root().join(format!("corpus/expected/{}.json", m.id));
        let expected: Option<Expected> = std::fs::read_to_string(&exp_path)
            .ok()
            .map(|s| serde_json::from_str(&s).expect("expected file parses"));
        // carry reviewed verdicts over to the same findings
        let mut reviewed: BTreeMap<_, Vec<Finding>> = BTreeMap::new();
        for f in expected.iter().flat_map(|e| &e.findings) {
            reviewed.entry(f.key()).or_default().push(f.clone());
        }
        for f in &mut got.findings {
            if let Some(r) = reviewed
                .get_mut(&f.key())
                .and_then(|v| (!v.is_empty()).then(|| v.remove(0)))
            {
                f.verdict = r.verdict;
                f.note = r.note;
            }
        }
        if update {
            std::fs::write(
                &exp_path,
                serde_json::to_string_pretty(&got).unwrap() + "\n",
            )
            .unwrap();
        } else {
            match &expected {
                None => failures.push(format!(
                    "{}: no {} (run with UPDATE_CORPUS=1)",
                    m.id,
                    exp_path.display()
                )),
                Some(e) if *e != got => {
                    let e_keys: Vec<_> = e.findings.iter().map(Finding::key).collect();
                    let g_keys: Vec<_> = got.findings.iter().map(Finding::key).collect();
                    let new: Vec<_> = g_keys.iter().filter(|k| !e_keys.contains(k)).collect();
                    let gone: Vec<_> = e_keys.iter().filter(|k| !g_keys.contains(k)).collect();
                    failures.push(format!(
                        "{}: differs from {}\n  counts {:?} -> {:?}\n  cut off {:?} -> {:?}\n  longest walk {:?} -> {:?}\n  new findings {new:?}\n  gone {gone:?}",
                        m.id, exp_path.display(), e.counts, got.counts, e.cut_off, got.cut_off, e.longest_walk, got.longest_walk
                    ));
                }
                _ => {}
            }
        }
        for f in &got.findings {
            if !VERDICTS.contains(&f.verdict.as_str()) {
                failures.push(format!(
                    "{}: unreviewed finding {:?} {} {} {:?}",
                    m.id, f.rule, f.subject, f.check, f.measured
                ));
            }
        }
        let n = |v: &str| got.findings.iter().filter(|f| f.verdict == v).count();
        println!(
            "{:<14} {:>8} {:>6} {:>10} {:>9} {:>14}",
            m.id,
            got.findings.len(),
            n("valid"),
            n("rule_limit"),
            n("model_gap"),
            n("false_positive")
        );
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
