use calyx_anneal::{
    CALYX_ANNEAL_J_INVALID_METRIC, JMetricSources, JObjectiveContext, JWeights, compute_j,
    j_weights_path, read_objective_weights_from_vault, set_objective_weights,
};
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
struct Sources {
    info: f64,
    n_eff: f64,
    sufficiency: f64,
    kernel_recall: f64,
    oracle_accuracy: f64,
    mistake_rate: f64,
    compression: f64,
    coverage: f64,
    dpi_ceiling: f64,
    provisional_count: usize,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            info: 0.0,
            n_eff: 0.0,
            sufficiency: 0.0,
            kernel_recall: 0.0,
            oracle_accuracy: 0.0,
            mistake_rate: 0.0,
            compression: 0.0,
            coverage: 0.0,
            dpi_ceiling: 10.0,
            provisional_count: 0,
        }
    }
}

impl JMetricSources for Sources {
    fn mutual_info_panel_anchor(&self) -> f64 {
        self.info
    }

    fn n_eff(&self) -> f64 {
        self.n_eff
    }

    fn panel_sufficiency(&self, _domain: &str) -> f64 {
        self.sufficiency
    }

    fn kernel_recall(&self) -> f64 {
        self.kernel_recall
    }

    fn oracle_accuracy(&self) -> f64 {
        self.oracle_accuracy
    }

    fn mistake_rate(&self) -> f64 {
        self.mistake_rate
    }

    fn compression_yield(&self) -> f64 {
        self.compression
    }

    fn coverage(&self) -> f64 {
        self.coverage
    }

    fn dpi_ceiling(&self) -> f64 {
        self.dpi_ceiling
    }

    fn provisional_count(&self) -> usize {
        self.provisional_count
    }
}

#[test]
fn all_terms_match_weighted_formula() {
    let context = JObjectiveContext::new("fixture", 4);
    let sources = Sources {
        info: 1.5,
        n_eff: 3.5,
        sufficiency: 0.8,
        kernel_recall: 0.7,
        oracle_accuracy: 0.6,
        mistake_rate: 0.1,
        compression: 0.4,
        coverage: 0.3,
        dpi_ceiling: 2.0,
        ..Sources::default()
    };

    let value = compute_j(&context, &sources).expect("compute j");

    assert_close(value.j, 7.2);
    assert_close(value.terms.p_redundant, 0.5);
    assert_close(value.terms.p_goodhart, 0.0);
    assert_eq!(value.provisional_excluded, 0);
}

#[test]
fn dpi_cap_clips_info_and_records_negative_headroom() {
    let context = JObjectiveContext::new("fixture", 1);
    let sources = Sources {
        info: 3.0,
        n_eff: 1.0,
        sufficiency: 1.0,
        dpi_ceiling: 2.0,
        ..Sources::default()
    };

    let value = compute_j(&context, &sources).expect("compute j");

    assert_close(value.terms.w1_info, 2.0);
    assert_close(value.dpi_headroom, -1.0);
}

#[test]
fn provisional_count_excludes_info_and_penalizes_j() {
    let context = JObjectiveContext::new("fixture", 0);
    let sources = Sources {
        info: 5.0,
        dpi_ceiling: 10.0,
        provisional_count: 5,
        ..Sources::default()
    };

    let value = compute_j(&context, &sources).expect("compute j");

    assert_close(value.terms.w1_info, 0.0);
    assert_close(value.terms.p_ungrounded, 5.0);
    assert_close(value.j, -5.0);
    assert_eq!(value.provisional_excluded, 5);
}

#[test]
fn zero_terms_and_zero_weights_edges_are_stable() {
    let zero =
        compute_j(&JObjectiveContext::new("fixture", 0), &Sources::default()).expect("zero terms");
    assert_close(zero.j, 0.0);

    let context = JObjectiveContext::new("fixture", 3).with_weights(JWeights::zero());
    let sources = Sources {
        n_eff: 1.0,
        provisional_count: 2,
        dpi_ceiling: 0.0,
        ..Sources::default()
    };
    let value = compute_j(&context, &sources).expect("zero weights");

    assert_close(value.terms.p_redundant, 2.0);
    assert_close(value.terms.p_ungrounded, 2.0);
    assert_close(value.terms.p_goodhart, 0.0);
    assert_close(value.j, -4.0);
}

#[test]
fn invalid_metric_fails_closed() {
    let sources = Sources {
        info: f64::NAN,
        ..Sources::default()
    };

    let error = compute_j(&JObjectiveContext::new("fixture", 0), &sources).unwrap_err();

    assert_eq!(error.code, CALYX_ANNEAL_J_INVALID_METRIC);
}

#[test]
fn objective_weights_roundtrip_vault_config() {
    let root = std::env::temp_dir().join(format!("calyx-j-weights-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create weight vault");
    let weights = JWeights {
        w1: 1.1,
        w2: 1.2,
        w3: 1.3,
        w4: 1.4,
        w5: 1.5,
        w6: 1.6,
        w7: 1.7,
        w8: 1.8,
    };

    set_objective_weights(&root, weights).expect("write weights");
    let readback = read_objective_weights_from_vault(&root).expect("read weights");

    assert_eq!(readback, weights);
    assert!(j_weights_path(&root).exists());
    let _ = std::fs::remove_dir_all(root);
}

proptest! {
    #[test]
    fn finite_inputs_return_finite_j(
        info in 0.0f64..10.0,
        n_eff in 0.0f64..10.0,
        sufficiency in 0.0f64..10.0,
        kernel_recall in 0.0f64..10.0,
        oracle_accuracy in 0.0f64..10.0,
        mistake_rate in 0.0f64..10.0,
        compression in 0.0f64..10.0,
        coverage in 0.0f64..10.0,
        dpi_ceiling in 0.0f64..10.0,
        panel_len in 0usize..20,
        provisional_count in 0usize..20,
    ) {
        let context = JObjectiveContext::new("fixture", panel_len);
        let sources = Sources {
            info,
            n_eff,
            sufficiency,
            kernel_recall,
            oracle_accuracy,
            mistake_rate,
            compression,
            coverage,
            dpi_ceiling,
            provisional_count,
        };

        let value = compute_j(&context, &sources)?;

        prop_assert!(value.j.is_finite());
        prop_assert!(value.dpi_headroom.is_finite());
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}
