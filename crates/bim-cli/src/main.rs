//! Headless pipeline: validate -> schedule -> advisories -> clashes -> takeoff.
//! Usage: bim-cli [project.json] [--rules rules/IN-2026.json] [--csv out.csv] [--html report.html] [--sample-out sample.json]
//! --html writes a self-contained 4D viewer (three.js inlined) with all geometry, schedule,
//! cost, clash and advisory data computed here in Rust — the hybrid render path from the plan.
use std::{env, fs, process::ExitCode};

const VIEWER_HTML: &str = include_str!("../viewer.html");
/// three.js r128, MIT (see vendor/three.LICENSE); inlined so reports work offline and from email.
const THREE_JS: &str = include_str!("../vendor/three.r128.min.js");
const VIEWER_CSS: &str = include_str!("../../../web/viewer.css");
const VIEWER_JS: &str = include_str!("../../../web/viewer.js");

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
    let rules_path = flag("--rules").unwrap_or_else(|| "rules/IN-2026.json".into());

    let project = match project_path.as_deref() {
        Some(p) => match fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|s| bim_core::Project::from_json(&s).map_err(|e| e.to_string()))
        {
            Ok(pr) => pr,
            Err(e) => {
                eprintln!("error: cannot load {p}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => bim_core::Project::sample(),
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

    let problems = project.validate();
    if !problems.is_empty() {
        eprintln!("project has {} validation error(s):", problems.len());
        for p in &problems {
            eprintln!("  - {p}");
        }
        return ExitCode::FAILURE;
    }

    println!("== {} (rule set {}) ==", project.name, project.rule_set);
    let schedule = match bim_schedule::schedule(&project.phases) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("schedule error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "\nSchedule: {} days total (* = critical)",
        schedule.total_days
    );
    for sp in &schedule.phases {
        let name = project.phase(sp.id).map(|p| p.name.as_str()).unwrap_or("?");
        println!(
            "  {}{:<14} day {:>3} - {:>3}",
            if sp.critical { "*" } else { " " },
            name,
            sp.start_day,
            sp.end_day
        );
    }

    let mut rule_set: Option<bim_rules::RuleSet> = None;
    match fs::read_to_string(rules_path)
        .map_err(|e| e.to_string())
        .and_then(|s| bim_rules::RuleSet::from_json(&s).map_err(|e| e.to_string()))
    {
        Ok(rs) => {
            let adv = rs.evaluate(&project);

            println!("\nAdvisories ({}) — {} [{}] verified: {} — ADVISORY ONLY, not a compliance determination", adv.len(), rs.id, rs.code_edition, rs.verified);
            for a in &adv {
                println!(
                    "  [{:?}] {}: {} ({})",
                    a.severity, a.subject, a.message, a.cite
                );
            }
            rule_set = Some(rs);
        }
        Err(e) => eprintln!("\nrules: skipped ({e})"),
    }

    let clashes = bim_geom::clashes(&project, 0.001);
    println!(
        "\nClashes ({}) — cross-discipline AABB overlap > 0.001 m3",
        clashes.len()
    );
    for c in &clashes {
        let n = |id| project.element(id).map(|e| e.name.as_str()).unwrap_or("?");
        println!("  {} <-> {} : {:.3} m3", n(c.a), n(c.b), c.overlap_m3);
    }

    let lines = bim_takeoff::takeoff(&project);
    println!("\nCost by phase");
    let mut total = bim_core::Cents(0);
    for (ph, c) in bim_takeoff::cost_by_phase(&lines) {
        let name = project.phase(ph).map(|p| p.name.as_str()).unwrap_or("?");
        println!("  {:<14} ${:>12.2}", name, c.dollars());
        total = total + c;
    }
    println!("  {:<14} ${:>12.2}", "TOTAL", total.dollars());
    if let Some(html) = flag("--html") {
        match render_html(&project, rule_set.as_ref()) {
            Ok(page) => {
                if let Err(e) = fs::write(&html, page) {
                    eprintln!("html: {e}");
                    return ExitCode::FAILURE;
                }
                println!("wrote {html}");
            }
            Err(e) => {
                eprintln!("html: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(csv) = flag("--csv") {
        if let Err(e) = fs::write(&csv, bim_takeoff::to_csv(&lines)) {
            eprintln!("csv: {e}");
            return ExitCode::FAILURE;
        }
        println!("wrote {csv}");
    }
    ExitCode::SUCCESS
}

/// Static report page: the same viewer.js the live app uses, with data pre-computed here.
fn render_html(
    project: &bim_core::Project,
    rules: Option<&bim_rules::RuleSet>,
) -> Result<String, String> {
    let report = bim_report::build(project, rules).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&report)
        .map_err(|e| e.to_string())?
        .replace("</", "<\\/");
    Ok(VIEWER_HTML
        .replacen("/*__CSS__*/", VIEWER_CSS, 1)
        .replacen("/*__THREE__*/", THREE_JS, 1)
        .replacen("/*__VIEWER__*/", VIEWER_JS, 1)
        .replacen("/*__DATA__*/", &json, 1))
}
