use serde_json::json;

use super::extension_tests::{
    TestEnv, call_err, call_ok, maybe_write_fsv_json, populated_vault, server,
};

#[test]
fn guard_generate_uses_persisted_lens_measurement() {
    let env = TestEnv::new("guard-generate-measurement");
    let server = server();
    let ingested = populated_vault(&server, "v");
    let identity_cx = ingested[0]["cx_id"].as_str().unwrap();
    let panel = call_ok(&server, 25, "calyx.list_panel", json!({"vault": "v"}));
    let slot = active_byte_axis_slot(&panel);
    let set = env.path("guard.jsonl");
    std::fs::write(&set, calibration_jsonl(slot)).unwrap();
    call_ok(
        &server,
        26,
        "calyx.guard.calibrate",
        json!({"vault": "v", "domain": "unit", "set": set, "target_far": 0.01}),
    );

    let verdict = call_ok(
        &server,
        27,
        "calyx.guard_generate",
        json!({
            "vault": "v",
            "candidate_text": "alpha alpha",
            "identity_cx": identity_cx,
        }),
    );

    assert_eq!(verdict["verdict"], "pass");
    assert!(
        verdict["distance"].as_f64().unwrap() <= 1.0e-6,
        "same text must reproduce the stored lens vector: {verdict}"
    );
    maybe_write_fsv_json("mcp-guard-generate-real-measurement.json", &verdict);
}

#[test]
fn guard_text_paths_fail_closed_after_required_lens_is_parked() {
    let env = TestEnv::new("guard-required-lens-parked");
    let server = server();
    let ingested = populated_vault(&server, "v");
    let identity_cx = ingested[0]["cx_id"].as_str().unwrap();
    let panel = call_ok(&server, 40, "calyx.list_panel", json!({"vault": "v"}));
    let slot = active_byte_axis_slot(&panel);
    let set = env.path("guard.jsonl");
    std::fs::write(&set, calibration_jsonl(slot)).unwrap();
    call_ok(
        &server,
        41,
        "calyx.guard.calibrate",
        json!({"vault": "v", "domain": "unit", "set": set, "target_far": 0.01}),
    );
    call_ok(
        &server,
        42,
        "calyx.park_lens",
        json!({"vault": "v", "slot": slot}),
    );

    let check = call_err(
        &server,
        43,
        "calyx.guard.check",
        json!({"vault": "v", "text": "alpha alpha"}),
    );
    let generate = call_err(
        &server,
        44,
        "calyx.guard_generate",
        json!({
            "vault": "v",
            "candidate_text": "alpha alpha",
            "identity_cx": identity_cx,
        }),
    );

    assert_eq!(check.data.unwrap()["calyx_code"], "CALYX_STALE_DERIVED");
    assert_eq!(generate.data.unwrap()["calyx_code"], "CALYX_STALE_DERIVED");
}

fn active_byte_axis_slot(panel: &serde_json::Value) -> u64 {
    panel["slots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|slot| slot["state"] == "active" && slot["name"] == "byte_axis")
        .and_then(|slot| slot["slot"].as_u64())
        .expect("active byte_axis slot")
}

fn calibration_jsonl(slot: u64) -> String {
    (0..50)
        .flat_map(|_| {
            [
                format!(r#"{{"slot":{slot},"score":0.99,"class":"good"}}"#),
                format!(r#"{{"slot":{slot},"score":0.10,"class":"injection"}}"#),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
