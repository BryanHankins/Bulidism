//! Browser entry point. Strings in, strings out (file exports return bytes): the JS
//! side holds the project JSON and calls `run` after every edit. Errors come back as
//! `Err(String)` which wasm-bindgen turns into a thrown JS exception.
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
    p.levels.push(bim_core::Level {
        id: bim_core::LevelId(0),
        name: "Level 1".into(),
        elevation_m: 0.0,
    });
    p.to_json().map_err(|e| e.to_string())
}

/// IFC (STEP text, IFC2x3/IFC4) -> `{"project": {...}, "notes": ["..."]}`. `file_name` names the
/// project when the file has no IfcProject name.
#[wasm_bindgen]
pub fn import_ifc(ifc_text: &str, file_name: &str) -> Result<String, String> {
    let r = bim_ifc::import(ifc_text, file_name).map_err(|e| e.to_string())?;
    serde_json::to_string(&r).map_err(|e| e.to_string())
}

/// Parse and validate (same gate as `run`), for the file exports.
fn valid_project(project_json: &str) -> Result<bim_core::Project, String> {
    let p = bim_core::Project::from_json(project_json).map_err(|e| e.to_string())?;
    bim_report::check(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Takeoff workbook (.xlsx bytes; a Uint8Array in JS): sheets "Takeoff" and "By phase".
#[wasm_bindgen]
pub fn takeoff_xlsx(project_json: &str) -> Result<Vec<u8>, String> {
    Ok(bim_export::xlsx(&valid_project(project_json)?))
}

/// Clashes (same tolerance as the report) as a BCF 2.1 archive (.bcfzip bytes). Components
/// carry element UUIDs in IfcGuid form; see `bim_export::bcf`.
#[wasm_bindgen]
pub fn clashes_bcf(project_json: &str) -> Result<Vec<u8>, String> {
    let p = valid_project(project_json)?;
    let c = bim_geom::clashes(&p, bim_report::CLASH_TOLERANCE_M3);
    Ok(bim_export::bcf(&p, &c))
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
    fn imports_ifc_to_a_runnable_project() {
        let j = import_ifc(include_str!("../../bim-ifc/tests/data/fixture.ifc"), "f").unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(v["notes"][0].as_str().unwrap().starts_with("Imported"));
        let p = serde_json::to_string(&v["project"]).unwrap();
        assert!(run(&p, Some(RS.to_string())).is_ok());
        assert!(import_ifc("not ifc", "f").is_err());
    }
    #[test]
    fn exports_xlsx_and_bcf() {
        let j = sample_project().unwrap();
        let x = takeoff_xlsx(&j).unwrap();
        assert_eq!(&x[..4], b"PK\x03\x04");
        assert!(x.windows(15).any(|w| w == b"xl/workbook.xml"));
        let b = clashes_bcf(&j).unwrap();
        assert_eq!(&b[..4], b"PK\x03\x04");
        assert_eq!(
            b.windows(10).filter(|w| w == b"markup.bcf").count(),
            2 * 4,
            "4 clash topics, each named in a local header and the central directory"
        );
        let mut p = bim_core::Project::from_json(&j).unwrap();
        p.elements[0].phase = bim_core::PhaseId(99);
        let bad = p.to_json().unwrap();
        assert!(takeoff_xlsx(&bad).unwrap_err().contains("validation"));
        assert!(clashes_bcf(&bad).is_err() && clashes_bcf("{").is_err());
    }
    #[test]
    fn new_project_is_valid() {
        let p = bim_core::Project::from_json(&new_project("n", "IN-2026").unwrap()).unwrap();
        assert!(p.validate().is_empty());
    }
}
