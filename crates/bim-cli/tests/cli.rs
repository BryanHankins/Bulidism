use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_bim-cli"))
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .args(args)
        .output()
        .expect("binary runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn sample_report_golden() {
    let (ok, out, err) = run(&[]);
    assert!(ok, "{err}");
    for needle in [
        "57 days total",
        "Advisories (2)",
        "Clashes (4)",
        "TOTAL          $    29442.46",
        "egress door clear width",
    ] {
        assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
    }
}

#[test]
fn loads_exported_sample_and_writes_html() {
    let dir = std::env::temp_dir().join("bim-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("r.html");
    let (ok, out, err) = run(&[
        "assets/sample_office.json",
        "--html",
        html.to_str().unwrap(),
    ]);
    assert!(ok, "{err}");
    assert!(out.contains("wrote"));
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(
        page.contains("\"total_days\":57")
            && page.contains("THREE.WebGLRenderer")
            && !page.contains("/*__DATA__*/")
    );
}

#[test]
fn invalid_project_exits_nonzero() {
    let dir = std::env::temp_dir().join("bim-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.json");
    std::fs::write(&bad, r#"{"schema_version":1,"name":"x","rule_set":"IN-2026","phases":[],"elements":[],"spaces":[{"name":"s","occupancy":"Business","floor_area_m2":10.0,"egress_doors":["00000000-0000-0000-0000-000000000000"]}],"unit_costs":{}}"#).unwrap();
    let (ok, _, err) = run(&[bad.to_str().unwrap()]);
    assert!(!ok && err.contains("validation error"), "{err}");
}

#[test]
fn explicit_missing_rules_fails() {
    let (ok, _, err) = run(&["--rules", "rules/NOPE.json"]);
    assert!(!ok && err.contains("cannot load rules"), "{err}");
}

#[test]
fn default_rules_follow_project_rule_set() {
    let dir = std::env::temp_dir().join("bim-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ibc.json");
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/sample_office.json"
    ))
    .unwrap()
    .replace("\"IN-2026\"", "\"IBC-2024\"");
    std::fs::write(&p, src).unwrap();
    // no rules/IBC-2024.json ships: warn and skip, never silently apply IN-2026
    let (ok, out, err) = run(&[p.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(err.contains("rules/IBC-2024.json"), "{err}");
    assert!(!out.contains("Advisories"), "{out}");
    // forcing another jurisdiction's file is allowed but flagged
    let (ok, out, err) = run(&[p.to_str().unwrap(), "--rules", "rules/IN-2026.json"]);
    assert!(ok, "{err}");
    assert!(out.contains("evaluated against 'IN-2026'"), "{out}");
}
