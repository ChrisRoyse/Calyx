use super::*;

#[test]
fn prediction_succeeds_with_one_ground_truth_calibration_sample() {
    let vault = vault();
    let panel = panel(&[1, 2]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    add_series(
        &vault,
        210,
        DOMAIN,
        "calibration",
        &[
            Row::truth("pass"),
            Row::prediction("pass", None),
            Row::prediction("pass", None),
            Row::prediction("pass", None),
            Row::prediction("pass", None),
            Row::prediction("pass", None),
        ],
    );
    for seed in 211..231 {
        add_series(
            &vault,
            seed,
            DOMAIN,
            ACTION,
            &[Row::prediction("Pass", Some("ci_passed"))],
        );
    }

    let prediction = oracle_predict(
        &vault,
        &action(ACTION, panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(908),
    )
    .expect("prediction with sparse ground truth");

    assert_eq!(prediction.outcome, AnchorValue::Text("Pass".to_string()));
    assert_eq!(prediction.confidence, 0.0);
    assert!(!prediction.consequences.is_empty());

    let payload = ledger_payload(&vault, prediction.provenance);
    assert_eq!(payload["recurrence_observations"], 20);
    assert!(payload["raw_confidence"].as_f64().unwrap() > 0.0);
    assert_eq!(payload["self_consistency_ceiling"], 0.0);
    assert_eq!(payload["confidence"], 0.0);
}
