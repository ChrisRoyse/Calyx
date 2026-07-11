use std::fs;

use serde_json::json;

use super::*;

#[test]
fn mcp_guard_check_text_uses_persisted_lens_measurement() {
    let env = TestEnv::new("guard-text-measurement");
    let server = server();
    one_dense_doc(&server, "v");
    let set = env.path("guard.jsonl");
    fs::write(&set, calibration_jsonl(8)).unwrap();
    call_ok(
        &server,
        22,
        "calyx.guard.calibrate",
        json!({"vault": "v", "domain": "unit", "set": set, "target_far": 0.01}),
    );

    let verdict = call_ok(
        &server,
        23,
        "calyx.guard.check",
        json!({"vault": "v", "text": "alpha"}),
    );

    assert_eq!(verdict["verdict"], "pass");
    assert!(
        verdict["distance"].as_f64().unwrap() <= 1.0e-6,
        "same text must reproduce the stored lens vector: {verdict}"
    );
    if let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("mcp-guard-check-real-measurement.json"),
            serde_json::to_vec_pretty(&verdict).unwrap(),
        )
        .unwrap();
    }
}
