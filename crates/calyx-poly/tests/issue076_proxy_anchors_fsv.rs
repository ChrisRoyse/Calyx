use std::fs;
use std::path::PathBuf;

use calyx_assay::TrustTag;
use calyx_core::{Anchor, AnchorKind, AnchorValue};
use calyx_poly::grounding::{
    ERR_PROXY_CONFIDENCE, ERR_UNKNOWN_GROUNDING, GroundingKind, ProxyKind, grounding_kind_of,
    proxy_anchor,
};
use serde_json::json;

#[test]
fn issue076_proxy_anchors_all_axes_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let up_1h = proxy_anchor(ProxyKind::Up1h, false, 0.25, 1_785_000_001).unwrap();
    let up_24h = proxy_anchor(ProxyKind::Up24h, true, 0.62, 1_785_000_024).unwrap();
    let crossed_05 = proxy_anchor(ProxyKind::Crossed05, true, 0.55, 1_785_000_050).unwrap();

    assert_proxy(&up_1h, "proxy:up_1h", GroundingKind::Proxy(ProxyKind::Up1h));
    assert_proxy(
        &up_24h,
        "proxy:up_24h",
        GroundingKind::Proxy(ProxyKind::Up24h),
    );
    assert_proxy(
        &crossed_05,
        "proxy:crossed_0.5",
        GroundingKind::Proxy(ProxyKind::Crossed05),
    );

    let confidence_one =
        proxy_anchor(ProxyKind::Up24h, true, 1.0, 1).expect_err("proxy certainty rejected");
    let confidence_zero =
        proxy_anchor(ProxyKind::Up1h, true, 0.0, 1).expect_err("proxy zero rejected");
    let confidence_nan =
        proxy_anchor(ProxyKind::Crossed05, true, f64::NAN, 1).expect_err("proxy NaN rejected");
    let unknown_axis = grounding_kind_of(&Anchor {
        kind: AnchorKind::Label("proxy_outcome".to_string()),
        value: AnchorValue::Bool(true),
        source: "proxy:unknown_horizon".to_string(),
        observed_at: 1,
        confidence: 0.5,
    })
    .expect_err("unknown proxy axis rejected");

    assert_eq!(confidence_one.code(), ERR_PROXY_CONFIDENCE);
    assert_eq!(confidence_zero.code(), ERR_PROXY_CONFIDENCE);
    assert_eq!(confidence_nan.code(), ERR_PROXY_CONFIDENCE);
    assert_eq!(unknown_axis.code(), ERR_UNKNOWN_GROUNDING);

    let readback = json!({
        "issue": 76,
        "proof_claim": "Poly constructs all three live-market proxy outcome anchors, classifies them as Provisional proxy grounding, and fails closed on invalid proxy certainty or unknown proxy axes",
        "minimum_sufficient_corpus": {
            "valid_proxy_anchors": 3,
            "invalid_proxy_confidence_edges": 3,
            "unknown_axis_edges": 1,
            "why_this_is_sufficient": "one valid anchor per supported proxy axis proves the complete up_1h/up_24h/crossed_0.5 surface; invalid confidence and unknown-axis edges prove fail-closed behavior",
            "why_smaller_is_insufficient": "fewer than three valid anchors would leave at least one supported proxy axis unproven",
            "why_larger_is_wasteful": "more anchors would repeat the same source-prefix and confidence validation paths without proving a new issue #76 invariant"
        },
        "source_of_truth": "physical JSON FSV artifact written after constructing and classifying the anchors",
        "valid": [
            anchor_json(&up_1h),
            anchor_json(&up_24h),
            anchor_json(&crossed_05)
        ],
        "edges": {
            "confidence_one_code": confidence_one.code(),
            "confidence_zero_code": confidence_zero.code(),
            "confidence_nan_code": confidence_nan.code(),
            "unknown_axis_code": unknown_axis.code()
        }
    });
    let out = root.join("readback.json");
    fs::write(&out, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ISSUE076_PROXY_ANCHORS_READBACK={}", out.display());
}

fn assert_proxy(anchor: &Anchor, expected_source: &str, expected_kind: GroundingKind) {
    assert_eq!(anchor.source, expected_source);
    assert!(anchor.confidence > 0.0 && anchor.confidence < 1.0);
    assert_eq!(grounding_kind_of(anchor).unwrap(), expected_kind);
    assert_eq!(
        grounding_kind_of(anchor).unwrap().trust(),
        TrustTag::Provisional
    );
}

fn anchor_json(anchor: &Anchor) -> serde_json::Value {
    json!({
        "kind": format!("{:?}", anchor.kind),
        "value": format!("{:?}", anchor.value),
        "source": anchor.source,
        "observed_at": anchor.observed_at,
        "confidence": anchor.confidence,
        "grounding_kind": format!("{:?}", grounding_kind_of(anchor).unwrap()),
        "trust": format!("{:?}", grounding_kind_of(anchor).unwrap().trust())
    })
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fsv/issue076_proxy_anchors")
}
