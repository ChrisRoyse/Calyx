use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::{Args, render};
use crate::partitioned_rrf_report_store;

#[test]
fn report_readback_renders_bounded_scalar_lines() {
    let root = temp_root("partitioned-rrf-report-readback");
    let report = sample_report();
    let written = partitioned_rrf_report_store::write(&root, "unit-report", &report).unwrap();
    let (record, loaded) = partitioned_rrf_report_store::read(&root, "unit-report").unwrap();
    let args = Args {
        cf_root: root.clone(),
        report_key: "unit-report".to_string(),
        limit_slots: 1,
        limit_pairs: 1,
    };

    let rendered = render(&record.report, &record.format, &record.mode, &loaded, &args);
    let text = rendered.join("\n");

    assert_eq!(written.value_sha256, loaded.value_sha256);
    assert!(text.contains("partitioned_rrf_report_readback\n"));
    assert!(text.contains("readback_matches=true\n"));
    assert!(text.contains("recall_at_k=0.91\n"));
    assert!(text.contains("latency_p99_us=24000\n"));
    assert!(text.contains("lens_roster_len=2\n"));
    assert!(text.contains("slots_shown=1\n"));
    assert!(text.contains(
        "slot slot=0 name=semantic slot_injected=true dim=768 n_regions=12 query_start_row=9"
    ));
    assert!(text.contains("redundancy_metric=debiased_linear_cka_hsic1_u4_v1\n"));
    assert!(text.contains(
        "redundancy_tuple_design=blake3_counter_uniform_four_distinct_with_replacement_v1\n"
    ));
    assert!(text.contains("redundancy_row_count=60\n"));
    assert!(text.contains("redundancy_tuple_count=960\n"));
    assert!(text.contains("redundancy_seed_hex=0xca1acafe4c4b4131\n"));
    assert!(text.contains(&format!(
        "redundancy_tuple_plan_blake3={}\n",
        "e".repeat(64)
    )));
    assert!(text.contains("redundancy_exact=false\n"));
    assert!(text.contains("redundancy_uncertainty_method=delete_32_group_jackknife_ratio_v1\n"));
    assert!(text.contains("redundancy_uncertainty_blocks=32\n"));
    assert!(text.contains(
        "redundancy_gate_score_method=max_0_raw_plus_4_mc_se_clamped_1_fail_closed_v1\n"
    ));
    assert!(text.contains("pair_values_total=2\n"));
    assert!(text.contains("pair_values_shown=1\n"));
    assert!(text.contains(
        "pair slot_a=0 slot_b=1 a=semantic pair_injected=true b=lexical corr=0.25 nmi=0.1 raw_signed_point=0.2 redundancy_point=0.2 mc_standard_error=0.0125 mc_gate_upper_estimate=0.25"
    ));
    assert!(!text.contains("a=semantic b=structural"));
    assert!(!text.contains("\nslot_injected=true"));
    assert!(!text.contains("\npair_injected=true"));
    assert!(text.contains("a37_gate_passed=true\n"));
    assert!(text.contains("temporal_active_count=1000\n"));
    assert!(!text.contains('{'));
    assert!(!text.contains('['));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn report_readback_parses_positive_pair_limit() {
    let raw = ["--cf-root", "report-cf", "--limit-pairs", "45"].map(str::to_string);
    let args = Args::parse(&raw).unwrap();
    assert_eq!(args.limit_pairs, 45);

    let zero = ["--cf-root", "report-cf", "--limit-pairs", "0"].map(str::to_string);
    assert!(Args::parse(&zero).is_err());
}

fn sample_report() -> Value {
    json!({
        "trigger": "calyx bench partitioned-rrf",
        "mode": "real_multi_slot_rrf",
        "metric_class": "ann_correctness",
        "metric_scope": "multi_slot_rrf",
        "plan_source": {
            "mode": "aster_graph_cf",
            "cf_root": "/tmp/plan-cf",
            "association_key": "plan",
            "db_readback": {
                "readback_matches": true,
                "value_sha256": "a".repeat(64)
            }
        },
        "lens_roster": [{"slot": 0}, {"slot": 1}],
        "per_lens_bits": [{"slot": 0}, {"slot": 1}],
        "slots": [
            {"slot": 0, "name": "semantic\nslot_injected=true", "dim": 768, "n_regions": 12, "query_start_row": 9},
            {"slot": 1, "name": "lexical", "dim": 512, "n_regions": 9, "query_start_row": 9}
        ],
        "ensemble_decomposition": {
            "mode": "assay_ensemble_card",
            "redundancy_method": {
                "metric": "debiased_linear_cka_hsic1_u4_v1",
                "tuple_design": "blake3_counter_uniform_four_distinct_with_replacement_v1",
                "row_count": 60,
                "tuple_count": 960,
                "seed_hex": "0xca1acafe4c4b4131",
                "tuple_plan_blake3": "e".repeat(64),
                "exact": false,
                "uncertainty_method": "delete_32_group_jackknife_ratio_v1",
                "uncertainty_blocks": 32,
                "gate_score_method": "max_0_raw_plus_4_mc_se_clamped_1_fail_closed_v1"
            },
            "pair_values": [
                {
                    "slot_a": 0, "slot_b": 1, "a": "semantic\npair_injected=true", "b": "lexical",
                    "corr": 0.25, "nmi": 0.1,
                    "redundancy": {
                        "raw_signed_point": 0.2,
                        "redundancy_point": 0.2,
                        "mc_standard_error": 0.0125,
                        "mc_gate_upper_estimate": 0.25
                    }
                },
                {
                    "slot_a": 0, "slot_b": 2, "a": "semantic", "b": "structural",
                    "corr": 0.3, "nmi": 0.2,
                    "redundancy": {
                        "raw_signed_point": 0.25,
                        "redundancy_point": 0.25,
                        "mc_standard_error": 0.0125,
                        "mc_gate_upper_estimate": 0.3
                    }
                }
            ]
        },
        "queries": 1000,
        "k": 10,
        "n_probe": 64,
        "region_beam": 1152,
        "truth_depth": 64,
        "ground_truth_queries": 1000,
        "ground_truth_source": {
            "mode": "precomputed_slot_rrf_aster_cf",
            "scale_suitable": true,
            "query_count": 1000,
            "truth_depth": 64,
            "db_readback": {
                "readback_matches": true,
                "value_bytes": 1234,
                "value_sha256": "b".repeat(64)
            }
        },
        "latency_us": {"p50": 18000, "p99": 24000, "p999": 25000, "max": 70000},
        "fused_ground_truth_recall_at_k": 0.91,
        "recall_floor": 0.85,
        "a37_admission": {
            "mode": "assay_multi_anchor_a37_admission_db",
            "status": "gate_passed",
            "gate_passed": true,
            "lens_count": 10,
            "association_family_count": 4,
            "db_readback": {
                "readback_matches": true,
                "value_sha256": "c".repeat(64)
            }
        },
        "temporal": {
            "mode": "aster_graph_cf",
            "row_count": 1000,
            "active_count": 1000,
            "inactive_count": 0,
            "duplicate_event_time_rows": 0,
            "out_of_order_event_time_rows": 0,
            "db_readback": {
                "readback_matches": true,
                "value_sha256": "d".repeat(64)
            }
        },
        "best_single_lens_recall_vs_fused_truth": 0.82,
        "fusion_matches_or_beats_best_single": true
    })
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
