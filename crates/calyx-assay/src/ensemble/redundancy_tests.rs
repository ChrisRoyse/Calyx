use calyx_core::SlotId;

use super::model::{EnsembleConfig, EnsembleLensInput};
use super::redundancy::{
    LINEAR_CKA_REDUNDANCY_METHOD, ensemble_redundancy_from_lenses, linear_cka_sketch_from_rows,
    linear_cka_tuple_plan, test_pair_estimate, test_tuple_z, test_tuples,
    validate_ensemble_card_redundancy, validate_evidence,
};

#[test]
fn complementary_pair_reduction_matches_literal_song_kernel() {
    let x = vec![
        vec![1.0, 2.0],
        vec![3.0, -1.0],
        vec![0.5, 4.0],
        vec![-2.0, 1.0],
    ];
    let y = vec![
        vec![2.0, 0.0],
        vec![-1.0, 3.0],
        vec![4.0, 1.0],
        vec![0.0, -2.0],
    ];
    let zx = test_tuple_z([&x[0], &x[1], &x[2], &x[3]]).unwrap();
    let zy = test_tuple_z([&y[0], &y[1], &y[2], &y[3]]).unwrap();
    let reduced = dot3(&zx, &zy);
    let mut literal = 0.0;
    for [s, t, u, v] in permutations4() {
        literal +=
            kernel(&x, s, t) * (kernel(&y, s, t) + kernel(&y, u, v) - 2.0 * kernel(&y, s, u));
    }
    literal /= 24.0;
    assert!(
        (reduced - literal).abs() < 1.0e-12,
        "{reduced} != {literal}"
    );
}

#[test]
fn complete_tuple_average_matches_unbiased_linear_hsic_oracle() {
    let x = base_rows(8);
    let y = transformed_rows(&x, 1.7, [5.0, -3.0, 2.0, 11.0]);
    let plan = linear_cka_tuple_plan(x.len()).unwrap();
    assert!(plan.is_exact());
    let tuple_average = test_tuples(&plan)
        .iter()
        .map(|tuple| {
            let zx = tuple_z_for(&x, *tuple);
            let zy = tuple_z_for(&y, *tuple);
            dot3(&zx, &zy)
        })
        .sum::<f64>()
        / plan.tuple_count() as f64;
    let oracle = unbiased_hsic(&linear_gram(&x), &linear_gram(&y), x.len());
    assert!(
        (tuple_average - oracle).abs() < 1.0e-9,
        "{tuple_average} != {oracle}"
    );
}

#[test]
fn exact_sketch_is_invariant_to_orthogonal_scale_and_translation() {
    let base = base_rows(19);
    let transformed = transformed_rows(&base, 3.0, [1000.0, -700.0, 250.0, 80.0]);
    let plan = linear_cka_tuple_plan(base.len()).unwrap();
    assert!(plan.is_exact());
    let left = linear_cka_sketch_from_rows(&plan, &base).unwrap();
    let right = linear_cka_sketch_from_rows(&plan, &transformed).unwrap();
    let estimate = test_pair_estimate(&left, &right, true).unwrap();
    assert!((estimate.raw_signed_point - 1.0).abs() < 1.0e-6);
    assert_eq!(estimate.mc_standard_error, 0.0);
    assert!((estimate.mc_gate_upper_estimate - 1.0).abs() < 1.0e-6);
}

#[test]
fn global_energy_normalization_preserves_extreme_valid_scales() {
    let base = base_rows(19);
    let plan = linear_cka_tuple_plan(base.len()).unwrap();
    let reference = linear_cka_sketch_from_rows(&plan, &base).unwrap();
    for scale in [1.0e-20_f32, 1.0e20_f32] {
        let scaled = base
            .iter()
            .map(|row| row.iter().map(|value| value * scale).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let sketch = linear_cka_sketch_from_rows(&plan, &scaled).unwrap();
        let estimate = test_pair_estimate(&reference, &sketch, true).unwrap();
        assert!(
            (estimate.raw_signed_point - 1.0).abs() < 1.0e-5,
            "scale={scale}"
        );
    }
}

#[test]
fn tuple_plan_is_deterministic_distinct_sorted_and_bounded() {
    let first = linear_cka_tuple_plan(50).unwrap();
    let second = linear_cka_tuple_plan(50).unwrap();
    assert!(!first.is_exact());
    assert_eq!(first.tuple_count(), 4_096);
    assert_eq!(test_tuples(&first), test_tuples(&second));
    assert!(test_tuples(&first).iter().all(|tuple| {
        tuple.windows(2).all(|pair| pair[0] < pair[1]) && tuple[3] < first.row_count()
    }));
    let five = linear_cka_tuple_plan(5).unwrap();
    assert!(five.is_exact());
    assert_eq!(five.tuple_count(), 5);
}

#[test]
fn invalid_or_degenerate_representations_fail_closed() {
    let too_short = linear_cka_tuple_plan(3).unwrap_err();
    assert_eq!(too_short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    let plan = linear_cka_tuple_plan(4).unwrap();
    let constant = vec![vec![2.0, -1.0]; 4];
    let constant_error = linear_cka_sketch_from_rows(&plan, &constant).unwrap_err();
    assert_eq!(constant_error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    let mut nonfinite = base_rows(4);
    nonfinite[2][1] = f32::NAN;
    let nonfinite_error = linear_cka_sketch_from_rows(&plan, &nonfinite).unwrap_err();
    assert_eq!(nonfinite_error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    let mut ragged = base_rows(4);
    ragged[3].pop();
    let ragged_error = linear_cka_sketch_from_rows(&plan, &ragged).unwrap_err();
    assert_eq!(ragged_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
}

#[test]
fn sampled_panel_evidence_persists_point_uncertainty_and_gate_score() {
    let base = base_rows(50);
    let transformed = transformed_rows(&base, 2.0, [3.0, -5.0, 7.0, 9.0]);
    let lenses = vec![
        EnsembleLensInput::new("base", SlotId::new(1), base),
        EnsembleLensInput::new("orthogonal", SlotId::new(2), transformed),
    ];
    let evidence = ensemble_redundancy_from_lenses(&lenses, 10).unwrap();
    assert_eq!(evidence.method.metric, LINEAR_CKA_REDUNDANCY_METHOD);
    assert_eq!(evidence.method.row_count, 50);
    assert_eq!(evidence.method.tuple_count, 4_096);
    assert!(!evidence.method.exact);
    let estimate = &evidence.pairs[0].linear_cka;
    assert!(estimate.raw_signed_point > 0.99999);
    assert!(estimate.redundancy_point > 0.99999);
    assert!(estimate.mc_standard_error.is_finite());
    assert!(estimate.mc_gate_upper_estimate >= estimate.redundancy_point);
}

#[test]
fn redundancy_method_metadata_tampering_fails_closed() {
    let base = base_rows(50);
    let transformed = transformed_rows(&base, 2.0, [3.0, -5.0, 7.0, 9.0]);
    let lenses = vec![
        EnsembleLensInput::new("base", SlotId::new(1), base),
        EnsembleLensInput::new("orthogonal", SlotId::new(2), transformed),
    ];
    let evidence = ensemble_redundancy_from_lenses(&lenses, 10).unwrap();

    for mutate in [
        |evidence: &mut super::model::EnsembleRedundancyEvidence| {
            evidence.method.tuple_design = "unverified_design".to_string();
        },
        |evidence: &mut super::model::EnsembleRedundancyEvidence| {
            evidence.method.tuple_plan_blake3 = "not-a-digest".to_string();
        },
        |evidence: &mut super::model::EnsembleRedundancyEvidence| {
            evidence.method.gate_score_method = "unverified_gate".to_string();
        },
    ] {
        let mut tampered = evidence.clone();
        mutate(&mut tampered);
        let error = validate_evidence(&lenses, &tampered).unwrap_err();
        assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    }
}

#[test]
fn current_card_missing_redundancy_method_fails_closed() {
    let mut card = current_card();
    card.redundancy_method = None;

    let error = validate_ensemble_card_redundancy(&card).unwrap_err();

    assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(error.message.contains("missing redundancy method"));
}

#[test]
fn current_card_missing_pair_redundancy_fails_closed() {
    let mut card = current_card();
    card.pairs[0].redundancy = None;

    let error = validate_ensemble_card_redundancy(&card).unwrap_err();

    assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(error.message.contains("missing redundancy evidence"));
}

#[test]
fn current_card_tampered_method_or_compatibility_score_fails_closed() {
    let card = current_card();
    validate_ensemble_card_redundancy(&card).unwrap();

    let mut method_tampered = card.clone();
    method_tampered
        .redundancy_method
        .as_mut()
        .unwrap()
        .tuple_plan_blake3 = "not-a-digest".to_string();
    let method_error = validate_ensemble_card_redundancy(&method_tampered).unwrap_err();
    assert_eq!(method_error.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    let mut score_tampered = card;
    score_tampered.pairs[0].corr = 0.0;
    let score_error = validate_ensemble_card_redundancy(&score_tampered).unwrap_err();
    assert_eq!(score_error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(score_error.message.contains("corr"));

    score_tampered.pairs[0].corr = f32::NAN;
    let non_finite_error = validate_ensemble_card_redundancy(&score_tampered).unwrap_err();
    assert_eq!(non_finite_error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(non_finite_error.message.contains("corr"));
}

#[test]
fn legacy_card_remains_decodable_but_cannot_cross_an_evidence_boundary() {
    let mut legacy = current_card();
    legacy.schema_version = super::model::ENSEMBLE_CARD_SCHEMA_VERSION - 1;
    legacy.redundancy_method = None;
    for pair in &mut legacy.pairs {
        pair.redundancy = None;
    }
    let bytes = serde_json::to_vec(&legacy).unwrap();
    let decoded: super::model::EnsembleCard = serde_json::from_slice(&bytes).unwrap();
    assert!(decoded.redundancy_method.is_none());
    assert!(decoded.pairs.iter().all(|pair| pair.redundancy.is_none()));

    let legacy_error = validate_ensemble_card_redundancy(&decoded).unwrap_err();
    assert_eq!(legacy_error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(
        legacy_error
            .message
            .contains("unsupported EnsembleCard schema")
    );

    legacy.schema_version = super::model::ENSEMBLE_CARD_SCHEMA_VERSION + 1;
    let error = validate_ensemble_card_redundancy(&legacy).unwrap_err();
    assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert!(error.message.contains("unsupported EnsembleCard schema"));
}

fn current_card() -> super::model::EnsembleCard {
    let rows = 60;
    let labels = (0..rows).map(|index| index % 2 == 0).collect::<Vec<_>>();
    let lenses = (0..10)
        .map(|lens| {
            let vectors = base_rows(rows)
                .into_iter()
                .enumerate()
                .map(|(row_index, row)| {
                    let scale = 1.0 + lens as f32 * 0.01;
                    let mut vector = Vec::with_capacity(row.len() + 1);
                    let planted_label = if labels[row_index] { 4.0 } else { -4.0 };
                    vector.push(planted_label * scale + lens as f32);
                    vector.extend(row.into_iter().enumerate().map(|(dimension, value)| {
                        let dimension = dimension + 1;
                        let sign = if (lens + dimension) % 2 == 0 {
                            1.0
                        } else {
                            -1.0
                        };
                        sign * value * scale + lens as f32
                    }));
                    vector
                })
                .collect::<Vec<_>>();
            EnsembleLensInput::new(
                format!("validator-lens-{lens}"),
                SlotId::new(lens as u16),
                vectors,
            )
        })
        .collect::<Vec<_>>();
    super::compute::ensemble_card(&lenses, &labels, None, &EnsembleConfig::default()).unwrap()
}

fn base_rows(rows: usize) -> Vec<Vec<f32>> {
    (0..rows)
        .map(|index| {
            let x = index as f32 - rows as f32 / 2.0;
            vec![
                x * 0.3,
                ((index * index) % 17) as f32 - 8.0,
                (index % 5) as f32,
                x * x * 0.01,
            ]
        })
        .collect()
}

fn transformed_rows(rows: &[Vec<f32>], scale: f32, shift: [f32; 4]) -> Vec<Vec<f32>> {
    rows.iter()
        .map(|row| {
            vec![
                shift[0] + scale * row[2],
                shift[1] - scale * row[0],
                shift[2] + scale * row[3],
                shift[3] - scale * row[1],
            ]
        })
        .collect()
}

fn tuple_z_for(rows: &[Vec<f32>], tuple: [usize; 4]) -> [f64; 3] {
    test_tuple_z([
        &rows[tuple[0]],
        &rows[tuple[1]],
        &rows[tuple[2]],
        &rows[tuple[3]],
    ])
    .unwrap()
}

fn kernel(rows: &[Vec<f32>], a: usize, b: usize) -> f64 {
    rows[a]
        .iter()
        .zip(&rows[b])
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum()
}

fn linear_gram(rows: &[Vec<f32>]) -> Vec<f64> {
    let mut gram = vec![0.0; rows.len() * rows.len()];
    for a in 0..rows.len() {
        for b in 0..rows.len() {
            gram[a * rows.len() + b] = kernel(rows, a, b);
        }
    }
    gram
}

fn unbiased_hsic(k: &[f64], l: &[f64], n: usize) -> f64 {
    let mut trace = 0.0;
    let mut sum_k = 0.0;
    let mut sum_l = 0.0;
    let mut row_k = vec![0.0; n];
    let mut row_l = vec![0.0; n];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            trace += k[i * n + j] * l[i * n + j];
            sum_k += k[i * n + j];
            sum_l += l[i * n + j];
            row_k[i] += k[i * n + j];
            row_l[i] += l[i * n + j];
        }
    }
    let n = n as f64;
    let row_product = row_k.iter().zip(row_l).map(|(a, b)| a * b).sum::<f64>();
    (trace + sum_k * sum_l / ((n - 1.0) * (n - 2.0)) - 2.0 * row_product / (n - 2.0))
        / (n * (n - 3.0))
}

fn permutations4() -> Vec<[usize; 4]> {
    let mut values = Vec::with_capacity(24);
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    if [a, b, c, d]
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == 4
                    {
                        values.push([a, b, c, d]);
                    }
                }
            }
        }
    }
    values
}

fn dot3(left: &[f64; 3], right: &[f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}
