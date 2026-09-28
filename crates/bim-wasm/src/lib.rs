//! Browser entry point. Strings in, strings out: the JS side holds the project
//! JSON and calls `run` after every edit. Errors come back as `Err(String)`
//! which wasm-bindgen turns into a thrown JS exception.
//!
//! Build:  wasm-pack build crates/bim-wasm --target web --out-dir ../../web/pkg
//!    or:  cargo build -p bim-wasm --target wasm32-unknown-unknown --release &&
//!         wasm-bindgen target/wasm32-unknown-unknown/release/bim_wasm.wasm --target web --out-dir web/pkg
use wasm_bindgen::prelude::*;

/// Full pipeline: validate → schedule → rules → clashes → takeoff → meshes. Returns report JSON.
#[wasm_bindgen]
pub fn run(project_json: &str, rules_json: Option<String>) -> Result<String, String> {
    bim_report::build_json(project_json, rules_json.as_deref()).map_err(|e| e.to_string())
}

/// Validation only (cheap; call on every keystroke). Returns a JSON array of messages.
#[wasm_bindgen]
pub fn validate(project_json: &str) -> Result<String, String> {
    let p = bim_core::Project::from_json(project_json).map_err(|e| e.to_string())?;
    let msgs: Vec<String> = p.validate().iter().map(|e| e.to_string()).collect();
    serde_json::to_string(&msgs).map_err(|e| e.to_string())
}

/// Starter project so the page has something to show before the user builds.
#[wasm_bindgen]
pub fn sample_project() -> Result<String, String> {
    bim_core::Project::sample()
        .to_json()
        .map_err(|e| e.to_string())
}

/// Empty project with one phase, for "New".
#[wasm_bindgen]
pub fn new_project(name: &str, rule_set: &str) -> Result<String, String> {
    let mut p = bim_core::Project::new(name, rule_set);
    p.phases.push(bim_core::Phase {
        id: bim_core::PhaseId(0),
        name: "Phase 1".into(),
        duration_days: 10,
        depends_on: vec![],
    });
    p.to_json().map_err(|e| e.to_string())
}

#[wasm_bindgen]
pub fn schema_version() -> u32 {
    bim_core::SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    const RS: &str = include_str!("../../../rules/IN-2026.json");
    #[test]
    fn run_sample_natively() {
        let j = run(&sample_project().unwrap(), Some(RS.to_string())).unwrap();
        assert!(j.contains("\"total_days\":57"));
    }
    #[test]
    fn validate_reports_problems() {
        let bad = r#"{"schema_version":1,"name":"x","rule_set":"x","phases":[],"elements":[],"spaces":[{"name":"s","occupancy":"Business","floor_area_m2":1.0,"egress_doors":["00000000-0000-0000-0000-000000000000"]}],"unit_costs":{}}"#;
        let v: Vec<String> = serde_json::from_str(&validate(bad).unwrap()).unwrap();
        assert_eq!(v.len(), 1);
    }
    #[test]
    fn bad_json_is_err() {
        assert!(run("{", None).is_err());
    }
    #[test]
    fn new_project_is_valid() {
        let p = bim_core::Project::from_json(&new_project("n", "IN-2026").unwrap()).unwrap();
        assert!(p.validate().is_empty());
    }
}
