use calyx_core::SlotId;

use super::*;

const DIM: usize = 8;

#[test]
fn ten_lens_card_reports_marginal_redundancy_synergy_and_sufficiency() {
    let (lenses, labels) = fixture_panel(160);
    let card = ensemble_card(&lenses, &labels, None, &EnsembleConfig::default()).unwrap();

    assert_eq!(card.schema_version, ENSEMBLE_CARD_SCHEMA_VERSION);
    assert_eq!(card.pid_method, ENSEMBLE_CARD_PID_METHOD);
    assert_eq!(card.panel_lens_count, 10);
    assert_eq!(card.n_samples, 160);
    assert_eq!(card.lenses.len(), 10);
    assert_eq!(card.pairs.len(), 45);
    assert!(card.panel_bits.is_finite());
    assert!(card.anchor_entropy_bits > 0.9);
    assert!(card.n_eff.is_finite());
    assert_eq!(
        card.keep_count + card.park_count + card.retire_count,
        card.lenses.len()
    );

    let redundant = card
        .lenses
        .iter()
        .find(|lens| lens.name == "redundant_a")
        .unwrap();
    assert_eq!(redundant.decision, EnsembleDecision::Retire);
    assert!(redundant.max_pairwise_corr > DEFAULT_MAX_REDUNDANCY);
    assert!(redundant.marginal_bits < DEFAULT_MIN_MARGINAL_BITS);
    assert!(redundant.pid.redundant_bits > 0.0);

    let noisy_pair = card
        .pairs
        .iter()
        .find(|pair| pair.a == "noisy_a" && pair.b == "noisy_b")
        .unwrap();
    assert!(
        noisy_pair.synergy_gain_bits > 0.0,
        "pair synergy {}",
        noisy_pair.synergy_gain_bits
    );
    assert!(
        card.lenses
            .iter()
            .any(|lens| lens.pid.synergistic_bits >= noisy_pair.synergy_gain_bits)
    );
}

#[test]
fn panels_below_theoretical_floor_fail_closed() {
    let (lenses, labels) = fixture_panel(80);
    let error = ensemble_card(&lenses[..2], &labels, None, &EnsembleConfig::default()).unwrap_err();

    assert_eq!(error.code, CALYX_ASSAY_PANEL_TOO_SMALL);
    assert!(error.message.contains("at least 3"));
}

#[test]
fn sub_ten_gate_panels_fail_closed() {
    let (lenses, labels) = fixture_panel(80);
    let error = ensemble_card(&lenses[..9], &labels, None, &EnsembleConfig::default()).unwrap_err();

    assert_eq!(error.code, CALYX_ASSAY_PANEL_TOO_SMALL);
    assert!(error.message.contains("at least 10"));
}

fn fixture_panel(rows: usize) -> (Vec<EnsembleLensInput>, Vec<bool>) {
    let labels = (0..rows).map(|idx| idx % 2 == 0).collect::<Vec<_>>();
    let real_a = lens_rows(rows, &labels, 1.0, 0.18, 1);
    let redundant_a = real_a
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            row.iter()
                .enumerate()
                .map(|(dim, value)| value + 0.001 * jitter(90, idx, dim))
                .collect()
        })
        .collect::<Vec<Vec<f32>>>();
    let specs = vec![
        ("real_a", real_a),
        ("noisy_a", lens_rows(rows, &labels, 0.36, 0.92, 2)),
        ("noisy_b", lens_rows(rows, &labels, 0.36, 0.92, 3)),
        ("real_b", lens_rows(rows, &labels, -0.82, 0.26, 4)),
        ("style", lens_rows(rows, &labels, 0.52, 0.55, 5)),
        ("syntax", lens_rows(rows, &labels, -0.48, 0.60, 6)),
        ("entity", lens_rows(rows, &labels, 0.44, 0.66, 7)),
        ("affect", lens_rows(rows, &labels, -0.40, 0.70, 8)),
        ("time_context", lens_rows(rows, &labels, 0.32, 0.74, 9)),
        ("redundant_a", redundant_a),
    ];
    let lenses = specs
        .into_iter()
        .enumerate()
        .map(|(idx, (name, rows))| EnsembleLensInput::new(name, SlotId::new(idx as u16), rows))
        .collect();
    (lenses, labels)
}

fn lens_rows(rows: usize, labels: &[bool], weight: f32, noise: f32, seed: u64) -> Vec<Vec<f32>> {
    (0..rows)
        .map(|row| {
            let signal = if labels[row] { 1.0 } else { -1.0 };
            (0..DIM)
                .map(|dim| signal * weight + noise * jitter(seed, row, dim))
                .collect()
        })
        .collect()
}

fn jitter(seed: u64, row: usize, dim: usize) -> f32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_be_bytes());
    hasher.update(&(row as u64).to_be_bytes());
    hasher.update(&(dim as u64).to_be_bytes());
    let bytes = hasher.finalize();
    let raw = u32::from_be_bytes([
        bytes.as_bytes()[0],
        bytes.as_bytes()[1],
        bytes.as_bytes()[2],
        bytes.as_bytes()[3],
    ]);
    (raw as f32 / u32::MAX as f32) * 2.0 - 1.0
}
