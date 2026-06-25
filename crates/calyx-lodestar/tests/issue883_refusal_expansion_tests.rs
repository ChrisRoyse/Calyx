use std::fs;

use calyx_core::{CxId, SlotId};
use calyx_lodestar::{
    ProbeFusionMode, ProbeHit, ProbeLength, ProbeMatrixLog, ProbeMatrixSpec, ProbePhrasing,
    ProbeRefusal, ProbeResponse, RefusalExpansionActionKind, RefusalExpansionParams,
    plan_refusal_expansion, run_probe_matrix, verify_refusal_expansion,
};
use calyx_sextant::RrfProfile;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue883-refusal-expansion")
}

fn spec() -> ProbeMatrixSpec {
    ProbeMatrixSpec {
        frontier: "statin myopathy".to_string(),
        active_slots: vec![SlotId::new(8)],
        weighted_profiles: vec![RrfProfile::Bridge],
        phrasings: vec![ProbePhrasing::Clinical],
        lengths: vec![ProbeLength::Phrase],
        top_k: 3,
    }
}

#[test]
fn plans_ranked_expansion_actions_from_refusals() {
    let before = before_log();
    let plan = plan_refusal_expansion(
        &before,
        &RefusalExpansionParams {
            min_deficit_bits: 0.20,
            max_actions: 8,
        },
    )
    .unwrap();

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].code, "CALYX_REFUSAL_LENS_DEFICIT");
    assert_eq!(plan.actions[0].kind, RefusalExpansionActionKind::AddLens);
    assert!(plan.actions[0].priority_score > plan.actions[0].deficit_bits);
}

#[test]
fn verifies_refusal_closed_by_new_grounded_hit() {
    let before = before_log();
    let after = after_log();

    let verification = verify_refusal_expansion(&before, &after).unwrap();

    assert!(verification.closed);
    assert_eq!(verification.before_refusal_count, 2);
    assert_eq!(verification.after_refusal_count, 0);
    assert_eq!(verification.closed_refusal_count, 2);
    assert_eq!(verification.new_grounded_hits, vec![id("after-grounded")]);
}

#[test]
fn no_new_grounded_hit_does_not_close() {
    let before = before_log();
    let after = run_probe_matrix(&spec(), |_| Ok(ProbeResponse::default())).unwrap();

    let verification = verify_refusal_expansion(&before, &after).unwrap();

    assert!(!verification.closed);
    assert_eq!(verification.closed_refusal_count, 2);
    assert!(verification.new_grounded_hits.is_empty());
}

#[test]
fn invalid_params_fail_closed() {
    let before = before_log();
    let err = plan_refusal_expansion(
        &before,
        &RefusalExpansionParams {
            min_deficit_bits: f32::NAN,
            max_actions: 8,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let before = before_log();
    let after = after_log();
    let plan = plan_refusal_expansion(&before, &RefusalExpansionParams::default()).unwrap();
    let verification = verify_refusal_expansion(&before, &after).unwrap();
    let value = json!({
        "issue": 883,
        "schema_version": plan.schema_version,
        "action_count": plan.actions.len(),
        "top_action_kind": plan.actions.first().map(|action| format!("{:?}", action.kind)),
        "before_refusal_count": verification.before_refusal_count,
        "after_refusal_count": verification.after_refusal_count,
        "closed_refusal_count": verification.closed_refusal_count,
        "new_grounded_count": verification.new_grounded_hits.len(),
        "closed": verification.closed,
        "plan": plan,
        "verification": verification,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue883_refusal_expansion_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["action_count"], 2);
    assert_eq!(readback["closed"], true);
    assert_eq!(readback["new_grounded_count"], 1);
    println!("issue883_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn before_log() -> ProbeMatrixLog {
    run_probe_matrix(&spec(), |variant| {
        let mut response = ProbeResponse::default();
        if variant.fusion == ProbeFusionMode::WeightedRrf {
            response.refusals.push(refusal(
                "CALYX_REFUSAL_LENS_DEFICIT",
                "per-sensor lens deficit",
                0.40,
            ));
        }
        if variant.fusion == ProbeFusionMode::Pipeline {
            response.refusals.push(refusal(
                "CALYX_REFUSAL_EVIDENCE_GAP",
                "targeted evidence missing",
                0.10,
            ));
        }
        Ok(response)
    })
    .unwrap()
}

fn after_log() -> ProbeMatrixLog {
    run_probe_matrix(&spec(), |variant| {
        let mut response = ProbeResponse::default();
        if variant.fusion == ProbeFusionMode::WeightedRrf {
            response.hits.push(hit(id("after-grounded"), 0.91, true));
        }
        Ok(response)
    })
    .unwrap()
}

fn refusal(code: &str, reason: &str, deficit_bits: f32) -> ProbeRefusal {
    ProbeRefusal {
        code: code.to_string(),
        reason: reason.to_string(),
        deficit_bits: Some(deficit_bits),
    }
}

fn hit(cx_id: CxId, score: f32, grounded: bool) -> ProbeHit {
    ProbeHit {
        cx_id,
        score,
        grounded,
        provenance: vec!["synthetic-expanded-evidence".to_string()],
    }
}
