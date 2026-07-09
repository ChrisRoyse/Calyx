use super::*;

#[test]
fn prediction_ledger_writes_dpi_bits_and_unit_ceiling() {
    let vault = vault();
    let panel = panel(&[1, 2]);
    put_sufficiency(&vault, &panel, 1.05, 1.0);
    seed_ceiling_point_95(&vault, DOMAIN);
    for seed in 180..200 {
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
        &FixedClock::new(907),
    )
    .expect("prediction");
    let expected_unit = dpi_unit(1.05, 1.0);

    assert_close(prediction.bound.dpi_ceiling.get(), 1.05);
    assert_close(prediction.bound.dpi_ceiling_unit.get(), expected_unit);
    assert!(prediction.confidence <= expected_unit + f32::EPSILON);

    let payload = ledger_payload(&vault, prediction.provenance);
    assert_close(
        payload["sufficiency_basis_bits"].as_f64().unwrap() as f32,
        1.05,
    );
    assert_close(payload["anchor_entropy_bits"].as_f64().unwrap() as f32, 1.0);
    assert_close(payload["dpi_ceiling"].as_f64().unwrap() as f32, 1.05);
    assert_eq!(payload["dpi_ceiling_deprecated"], true);
    assert_close(
        payload["dpi_ceiling_unit"].as_f64().unwrap() as f32,
        expected_unit,
    );
}

fn dpi_unit(bits: f32, entropy: f32) -> f32 {
    1.0 - 2.0_f32.powf(-2.0 * bits / entropy)
}
