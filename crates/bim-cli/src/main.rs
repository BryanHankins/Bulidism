//! Headless pipeline: runs bim_report::build (validate -> schedule -> advisories -> clashes -> takeoff).
//! Usage: bim-cli [project.json | model.ifc] [--rules rules/<project rule_set>.json] [--csv out.csv]
//!        [--xlsx out.xlsx] [--bcf clashes.bcfzip] [--html report.html] [--sample-out sample.json]
//!        [--baseline earlier.json|.ifc]  (prints what changed since that version)
//! --html writes a self-contained 4D viewer (three.js inlined) with all geometry, schedule,
//! cost, clash and advisory data computed here in Rust — the hybrid render path from the plan.
use std::{env, fs, process::ExitCode};

const VIEWER_HTML: &str = include_str!("../viewer.html");
/// three.js r128, MIT (see vendor/three.LICENSE); inlined so reports work offline and from email.
const THREE_JS: &str = include_str!("../vendor/three.r128.min.js");
const VIEWER_CSS: &str = include_str!("../../../web/viewer.css");
const VIEWER_JS: &str = include_str!("../../../web/viewer.js");
const CITES_JS: &str = include_str!("../../../web/cites.js");

/// A project from .json, or imported from .ifc (import notes are printed).
fn load(p: &str) -> Result<bim_core::Project, String> {
    let s = fs::read_to_string(p).map_err(|e| e.to_string())?;
    if p.to_ascii_lowercase().ends_with(".ifc") {
        let stem = std::path::Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("IFC import");
        let r = bim_ifc::import(&s, stem).map_err(|e| e.to_string())?;
        for n in &r.notes {
            println!("ifc: {n}");
        }
        Ok(r.project)
    } else {
        bim_core::Project::from_json(&s).map_err(|e| e.to_string())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    // first positional = project path; a positional directly after a --flag is that flag's value
    let project_path = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || !args[i - 1].starts_with("--")))
        .map(|(_, a)| a.clone());
    let project = match project_path.as_deref() {
        Some(p) => match load(p) {
            Ok(pr) => pr,
            Err(e) => {
                eprintln!("error: cannot load {p}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => bim_core::Project::sample(),
    };
    let baseline = match flag("--baseline")
        .map(|p| load(&p).map_err(|e| (p, e)))
        .transpose()
    {
        Ok(b) => b,
        Err((p, e)) => {
            eprintln!("error: cannot load baseline {p}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(out) = flag("--sample-out") {
        match project
            .to_json()
            .map_err(|e| e.to_string())
            .and_then(|j| fs::write(&out, j).map_err(|e| e.to_string()))
        {
            Ok(()) => println!("wrote {out}"),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // explicit --rules must load; the default follows the project's own rule set
    let explicit_rules = flag("--rules");
    let rules_path = explicit_rules
        .clone()
        .unwrap_or_else(|| format!("rules/{}.json", project.rule_set));
    let rule_set = match fs::read_to_string(&rules_path)
        .map_err(|e| e.to_string())
        .and_then(|s| bim_rules::RuleSet::from_json(&s).map_err(|e| e.to_string()))
    {
        Ok(rs) => Some(rs),
        Err(e) if explicit_rules.is_some() => {
            eprintln!("error: cannot load rules {rules_path}: {e}");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("warning: no advisories — cannot load rules {rules_path}: {e}");
            None
        }
    };

    // the one shared pipeline (same as the wasm build)
    let report = match bim_report::build(&project, rule_set.as_ref()) {
        Ok(r) => r,
        Err(bim_report::ReportError::Invalid(problems)) => {
            eprintln!("project has {} validation error(s):", problems.len());
            for p in &problems {
                eprintln!("  - {p}");
            }
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("== {} (rule set {}) ==", project.name, project.rule_set);
    println!(
        "\nSchedule: {} days total (* = critical)",
        report.total_days
    );
    for ph in &report.phases {
        println!(
            "  {}{:<14} day {:>3} - {:>3}",
            if ph.critical { "*" } else { " " },
            ph.name,
            ph.start_day,
            ph.end_day
        );
    }

    if rule_set.is_some() || !report.advisories.is_empty() {
        println!("\nAdvisories ({}) — {} [{}] verified: {} — ADVISORY ONLY, not a compliance determination", report.advisories.len(), report.rule_set, report.code_edition, report.rules_verified);
        for a in &report.advisories {
            println!(
                "  [{:?}] {}: {} ({})",
                a.severity, a.subject, a.message, a.cite
            );
        }
    }

    println!(
        "\nClashes ({}) — cross-discipline overlap > {} m3",
        report.clashes.len(),
        bim_report::CLASH_TOLERANCE_M3
    );
    for c in &report.clashes {
        let n = |id| project.element(id).map(|e| e.name.as_str()).unwrap_or("?");
        println!("  {} <-> {} : {:.3} m3", n(c.a), n(c.b), c.overlap_m3);
    }

    println!("\nCost by phase");
    for ph in &report.phases {
        println!("  {:<14} ${:>12.2}", ph.name, ph.cost_usd);
    }
    println!("  {:<14} ${:>12.2}", "TOTAL", report.total_cost_usd);

    if let Some(old) = &baseline {
        let c = match bim_report::build(old, rule_set.as_ref()) {
            Ok(old_r) => bim_report::compare::diff(old, &old_r, &project, &report),
            Err(e) => {
                eprintln!("error: baseline: {e}");
                return ExitCode::FAILURE;
            }
        };
        print_comparison(&c);
    }
    if let Some(html) = flag("--html") {
        match render_html(&report)
            .and_then(|page| fs::write(&html, page).map_err(|e| e.to_string()))
        {
            Ok(()) => println!("wrote {html}"),
            Err(e) => {
                eprintln!("html: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(csv) = flag("--csv") {
        let lines = bim_takeoff::takeoff(&project);
        if let Err(e) = fs::write(&csv, bim_takeoff::to_csv(&lines)) {
            eprintln!("csv: {e}");
            return ExitCode::FAILURE;
        }
        println!("wrote {csv}");
    }
    let exports: [(&str, &dyn Fn() -> Vec<u8>); 2] = [
        ("--xlsx", &|| bim_export::xlsx(&project)),
        ("--bcf", &|| {
            let c = bim_geom::clashes(&project, bim_report::CLASH_TOLERANCE_M3);
            bim_export::bcf(&project, &c)
        }),
    ];
    for (name, make) in exports {
        if let Some(out) = flag(name) {
            if let Err(e) = fs::write(&out, make()) {
                eprintln!("{}: {e}", &name[2..]);
                return ExitCode::FAILURE;
            }
            println!("wrote {out}");
        }
    }
    ExitCode::SUCCESS
}

/// Static report page: the same viewer.js the live app uses, with data pre-computed here.
fn render_html(report: &bim_report::Report) -> Result<String, String> {
    let json = serde_json::to_string(report)
        .map_err(|e| e.to_string())?
        // inside <script>, '<' must never appear raw (covers </script and <!--)
        .replace('<', "\\u003c");
    Ok(VIEWER_HTML
        .replacen("/*__CSS__*/", VIEWER_CSS, 1)
        .replacen("/*__CITES__*/", CITES_JS, 1)
        .replacen("/*__THREE__*/", THREE_JS, 1)
        .replacen("/*__VIEWER__*/", VIEWER_JS, 1)
        .replacen("/*__DATA__*/", &json, 1))
}

fn print_comparison(c: &bim_report::compare::Comparison) {
    use bim_report::compare::Status;
    let mark = |s: Status| match s {
        Status::Added => "+",
        Status::Removed => "-",
        Status::Changed => "~",
    };
    println!("\nChanges since baseline '{}'", c.baseline);
    if c.rule_set_differs {
        println!("  (the versions name different rule sets; both checked against this one)");
    }
    for t in &c.totals {
        let f = |v: f64| match t.unit.as_str() {
            "usd" => format!("${v:.2}"),
            _ => format!("{v}"),
        };
        if t.before != t.after {
            println!("  {:<14} {} -> {}", t.label, f(t.before), f(t.after));
        }
    }
    for f in &c.findings {
        let what = match f.status {
            Status::Added => "new",
            Status::Removed => "resolved",
            Status::Changed => "changed",
        };
        println!(
            "  finding {what:<8} [{:?}] {}: {} ({})",
            f.finding.severity, f.finding.subject, f.finding.message, f.finding.cite
        );
    }
    if c.unchanged_findings > 0 {
        println!("  {} finding(s) unchanged", c.unchanged_findings);
    }
    for k in &c.clashes {
        println!("  clash {} {} <-> {}", mark(k.status), k.a_name, k.b_name);
    }
    for e in &c.elements {
        println!(
            "  element {} {} {}",
            mark(e.status),
            e.name,
            e.aspects.join(", ")
        );
    }
    for s in &c.spaces {
        println!(
            "  space {} {} {}",
            mark(s.status),
            s.name,
            s.aspects.join(", ")
        );
    }
}
