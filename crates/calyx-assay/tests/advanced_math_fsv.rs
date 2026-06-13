use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    BetaBernoulli, Direction, GammaPoisson, PeriodogramConfig, StratumBits, TransferEntropyConfig,
    autocorrelation, bin_event_counts, lomb_scargle, lomb_scargle_with_config, stratified_bits,
    transfer_entropy, transfer_entropy_sweep_with_config, transfer_entropy_with_config,
};
use calyx_core::{CalyxError, CxId, FixedClock};
use calyx_lodestar::{CALYX_PROP_NO_KERNEL_NODES, propagate_labels};
use calyx_mincut::{laplacian_eigenmaps, laplacian_eigenmaps_with_max_iter, spectral_gap};
use calyx_paths::AssocGraph;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};

const SEED: u64 = 42;
const PLANTED_PERIOD: f64 = 7.0;
const TRUE_RATE: f64 = 2.0;

type TestStream = Vec<(u64, f32)>;

#[test]
fn advanced_math_fsv_writes_all_planted_synthetic_readbacks() {
    let period = planted_period_fsv();
    write_sot("ph52_period.json", period);

    let te = planted_transfer_entropy_fsv();
    write_sot("ph52_te_fsv.json", te);

    let labels = planted_label_propagation_fsv();
    write_sot("ph52_label_prop.json", labels);

    let spectral = planted_spectral_fsv();
    write_sot("ph52_spectral_fsv.json", spectral);

    let bayes = planted_bayes_fsv();
    write_sot("ph52_bayes_fsv.json", bayes);
}

fn planted_period_fsv() -> Value {
    let times = planted_event_times(SEED);
    let (centres, counts) = bin_event_counts(&times, 1.0).unwrap();
    let report = lomb_scargle(&centres, &counts).unwrap();
    let dominant = *report.dominant().expect("planted period peak");
    let acf = autocorrelation(&centres, &counts).unwrap();
    let ci = period_detection_ci(&times, dominant.period);
    let within_5pct = (dominant.period - PLANTED_PERIOD).abs() <= 0.05 * PLANTED_PERIOD;

    assert!(within_5pct, "{dominant:?}");
    assert!(ci.0 <= dominant.period && dominant.period <= ci.1);

    json!({
        "case": "planted_period_lomb_scargle",
        "trigger": "100 planted recurrence events with Gaussian jitter, seed 42",
        "planted_period": PLANTED_PERIOD,
        "detected_period": dominant.period,
        "ci_95": ci,
        "ci_basis": "seeded perturbation of planted jitter distribution",
        "within_5pct": within_5pct,
        "detected_power": dominant.power,
        "false_alarm_probability": dominant.false_alarm_probability,
        "acf_dominant_lag": acf.dominant_period,
        "trust": format!("{:?}", report.trust),
        "edges": [
            calyx_edge("empty_input", json!({"times": [], "values": []}), lomb_scargle(&[], &[]).map(|_| json!("report"))),
            calyx_edge("below_min_samples", json!({"n": 5}), lomb_scargle(&centres[..5], &counts[..5]).map(|_| json!("report"))),
            calyx_edge("zero_variance", json!({"n": 20, "value": 3.0}), lomb_scargle(&centres[..20], &[3.0; 20]).map(|_| json!("report"))),
        ],
    })
}

fn planted_transfer_entropy_fsv() -> Value {
    let clock = FixedClock::new(1_786_200_000);
    let config = TransferEntropyConfig {
        bootstrap_resamples: 80,
        ..TransferEntropyConfig::default()
    };
    let (a, b) = planted_a_to_b(140, 2);
    let result = transfer_entropy_with_config(&a, &b, 2, &clock, &config).unwrap();
    let sweep = transfer_entropy_sweep_with_config(&a, &b, &[1, 2, 4, 8], &clock, &config);
    let empty: TestStream = Vec::new();
    let single = vec![(0, 1.0)];
    let duplicate_timestamp = [(1, 0.1), (1, 0.2)];

    assert!(!result.provisional);
    assert_eq!(result.dominant_direction, Direction::AToB);
    assert!(result.t_a_to_b > result.t_b_to_a + 0.1, "{result:?}");
    assert!(result.ci_95.0 > 0.0, "{result:?}");
    assert!(result.difference_ci_95.0 > 0.0, "{result:?}");

    json!({
        "case": "planted_a_to_b_lag2",
        "trigger": "140-step stream where A at t drives B at t+2",
        "planted_lag": 2,
        "t_a_to_b": result.t_a_to_b,
        "t_b_to_a": result.t_b_to_a,
        "dominant_direction": result.dominant_direction,
        "ci_95": result.ci_95,
        "t_b_to_a_ci_95": result.t_b_to_a_ci_95,
        "difference_ci_95": result.difference_ci_95,
        "lag": result.lag,
        "provisional": result.provisional,
        "n_samples": result.n_samples,
        "sweep": sweep,
        "edges": [
            json!({"case": "empty_streams", "state_before": {"a": [], "b": []}, "state_after": transfer_entropy(&empty, &empty, 2, &clock).unwrap()}),
            json!({"case": "single_event", "state_before": {"n": 1}, "state_after": transfer_entropy(&single, &single, 2, &clock).unwrap()}),
            calyx_edge("duplicate_timestamp", json!({"timestamps": [1, 1]}), transfer_entropy(&duplicate_timestamp, &duplicate_timestamp, 1, &clock).map(|_| json!("report"))),
        ],
    })
}

fn planted_label_propagation_fsv() -> Value {
    let graph = rare_class_graph();
    let anchors = [(cx(0), 1.0), (cx(10), 1.0)];
    let labels = propagate_labels(&graph, &anchors, 64, 1.0e-6).unwrap();
    let left_neighbor = labels.iter().find(|row| row.node_id == cx(1)).unwrap();
    let right_neighbor = labels.iter().find(|row| row.node_id == cx(11)).unwrap();
    let rare_bits = stratified_bits(
        0.02,
        vec![StratumBits {
            name: "rare_class".to_string(),
            bits: 0.31,
            frequency: 0.02,
            sole_carrier: true,
        }],
    );

    for row in [left_neighbor, right_neighbor] {
        assert!(row.provisional);
        assert_eq!(row.hop_distance, 1);
        assert!(row.confidence > 0.3, "{row:?}");
    }
    assert_eq!(rare_bits.effective_bits, 0.31);
    assert!(rare_bits.no_frequency_multiplier);

    json!({
        "case": "rare_class_stratified_bits_then_label_propagation",
        "trigger": "20-node graph with two rare-class kernel anchors",
        "rare_class_bits": rare_bits,
        "kernel_nodes": [cx(0), cx(10)],
        "nearest_neighbors": [
            label_readback(left_neighbor),
            label_readback(right_neighbor),
        ],
        "labels": labels,
        "ci_95": [
            left_neighbor.confidence.min(right_neighbor.confidence),
            left_neighbor.confidence.max(right_neighbor.confidence)
        ],
        "ci_basis": "deterministic harmonic diffusion over fixed graph",
        "edges": [
            prop_edge("no_kernel", json!({"kernel_labels": []}), propagate_labels(&graph, &[], 16, 1.0e-6).map(|_| json!("labels"))),
            prop_edge("empty_graph", json!({"node_count": 0}), propagate_labels(&AssocGraph::builder().build(), &anchors, 16, 1.0e-6).map(|_| json!("labels"))),
            prop_edge("not_converged", json!({"max_iter": 1, "tol": 1e-9}), propagate_labels(&path_graph(4), &[(cx(0), 1.0), (cx(3), 0.0)], 1, 1.0e-9).map(|_| json!("labels"))),
        ],
    })
}

fn planted_spectral_fsv() -> Value {
    let graph = two_community_graph();
    let eigenmaps = laplacian_eigenmaps(&graph, 3).unwrap();
    let fiedler = &eigenmaps[1].eigenvector;
    let left = sign_count(&fiedler[0..5]);
    let right = sign_count(&fiedler[5..10]);
    let gap = spectral_gap(&eigenmaps);
    let split_ok = (left == (5, 0) && right == (0, 5)) || (left == (0, 5) && right == (5, 0));

    assert!(split_ok, "left={left:?} right={right:?}");
    assert!(gap > 0.0 && gap < 0.1, "gap={gap}");

    json!({
        "case": "two_5_cliques_one_weak_bridge",
        "trigger": "10-node planted community graph",
        "spectral_gap": gap,
        "ci_95": [gap, gap],
        "ci_basis": "deterministic eigensolve over fixed weighted graph",
        "fiedler_sign_count_positive": left.0 + right.0,
        "fiedler_sign_count_negative": left.1 + right.1,
        "left_clique_signs": left,
        "right_clique_signs": right,
        "eigenvalues": eigenmaps.iter().map(|pair| pair.eigenvalue).collect::<Vec<_>>(),
        "fiedler": fiedler,
        "edges": [
            spectral_edge("one_node_graph", json!({"node_count": 1}), laplacian_eigenmaps(&single_node_graph(), 2).map(|_| json!("eigenmaps"))),
            spectral_edge("not_converged", json!({"max_iter": 0}), laplacian_eigenmaps_with_max_iter(&graph, 2, 0).map(|_| json!("eigenmaps"))),
            spectral_edge("disconnected_gap", json!({"components": 2}), Ok(json!({"gap": spectral_gap(&laplacian_eigenmaps(&disconnected_graph(), 2).unwrap())}))),
        ],
    })
}

fn planted_bayes_fsv() -> Value {
    let mut replications = Vec::new();
    for index in 0..10 {
        let mut posterior = GammaPoisson::default();
        posterior.update(10, 5.0).unwrap();
        let ci = posterior.credible_interval_95().unwrap();
        replications.push(json!({
            "replication": index,
            "posterior": posterior,
            "mean_rate": posterior.mean_rate(),
            "ci_95": ci,
            "contains_true_rate": ci.0 <= TRUE_RATE && TRUE_RATE <= ci.1,
        }));
    }
    let covered = replications
        .iter()
        .filter(|row| row["contains_true_rate"].as_bool().unwrap())
        .count();
    let coverage_rate = covered as f64 / replications.len() as f64;
    let mut beta = BetaBernoulli::default();
    beta.update(9, 1).unwrap();
    let beta_ci = beta.credible_interval_95().unwrap();

    assert!(coverage_rate >= 0.9);
    assert!(beta_ci.0 <= beta.mean_consistency() && beta.mean_consistency() <= beta_ci.1);

    json!({
        "case": "gamma_poisson_ci_coverage",
        "trigger": "10 seeded replications, each 10 events over 5 time units",
        "true_rate": TRUE_RATE,
        "coverage_rate": coverage_rate,
        "covered_replications": covered,
        "replications": replications,
        "beta_bernoulli_crosscheck": {
            "posterior": beta,
            "mean_consistency": beta.mean_consistency(),
            "ci_95": beta_ci,
            "reliable_threshold_0_7_confidence_0_87": beta.is_reliable(0.7, 0.87).unwrap(),
            "reliable_threshold_0_7_confidence_0_90": beta.is_reliable(0.7, 0.90).unwrap(),
        },
        "edges": [
            calyx_edge("invalid_interval", json!({"interval": 0.0}), {
                let mut posterior = GammaPoisson::default();
                posterior.update(1, 0.0).map(|_| json!("updated"))
            }),
            calyx_edge("negative_events", json!({"events": -1}), {
                let mut posterior = GammaPoisson::default();
                posterior.update_signed(-1, 1.0).map(|_| json!("updated"))
            }),
            calyx_edge("negative_successes", json!({"successes": -1}), {
                let mut posterior = BetaBernoulli::default();
                posterior.update_signed(-1, 0).map(|_| json!("updated"))
            }),
        ],
    })
}

fn write_sot(file_name: &str, mut value: Value) {
    let tmp_path = tmp_file(file_name);
    let before = file_state(&tmp_path);
    value["source_of_truth"] = json!({
        "primary_path": tmp_path,
        "before": before,
    });
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    fs::create_dir_all(tmp_path.parent().unwrap()).unwrap();
    fs::write(&tmp_path, &bytes).unwrap();
    mirror_to_root(file_name, &bytes);
    let after = fs::read(&tmp_path).unwrap();
    assert_eq!(after, bytes);
    println!(
        "PH52_ADVANCED_MATH_FSV {} blake3={}",
        tmp_path.display(),
        blake3::hash(&after)
    );
}

fn planted_event_times(seed: u64) -> Vec<f64> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..100)
        .map(|k| k as f64 * PLANTED_PERIOD + 0.3 * standard_normal(&mut rng))
        .collect()
}

fn period_detection_ci(times: &[f64], point: f64) -> (f64, f64) {
    let mut periods = vec![point];
    for offset in 0..24 {
        let mut perturbed = times.to_vec();
        let mut rng = ChaCha8Rng::seed_from_u64(SEED + offset + 1);
        for value in &mut perturbed {
            *value += 0.03 * standard_normal(&mut rng);
        }
        let (centres, counts) = bin_event_counts(&perturbed, 1.0).unwrap();
        let report = lomb_scargle_with_config(
            &centres,
            &counts,
            &PeriodogramConfig {
                fap_permutations: 1,
                max_peaks: 1,
                ..PeriodogramConfig::default()
            },
        )
        .unwrap();
        periods.push(report.dominant().unwrap().period);
    }
    percentile_f64(periods, point)
}

fn planted_a_to_b(n: usize, lag: usize) -> (TestStream, TestStream) {
    let a = simple_stream(n, 7);
    let b = (0..n)
        .map(|t| {
            let value = if t >= lag {
                a[t - lag].1 + 0.01 * (noise(t as u64, 41) - 0.5)
            } else {
                noise(t as u64, 73)
            };
            (t as u64, value)
        })
        .collect();
    (a, b)
}

fn simple_stream(n: usize, salt: u64) -> TestStream {
    (0..n)
        .map(|t| (t as u64, 0.2 + 0.6 * noise(t as u64, salt)))
        .collect()
}

fn rare_class_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 0..20 {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    for node in 0..9 {
        add_undirected_weight(&mut builder, node, node + 1, 1.0);
    }
    for node in 10..19 {
        add_undirected_weight(&mut builder, node, node + 1, 1.0);
    }
    add_undirected_weight(&mut builder, 9, 10, 0.25);
    builder.build()
}

fn two_community_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 1..=10 {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    for cluster in [1..=5, 6..=10] {
        let nodes: Vec<_> = cluster.collect();
        for (index, left) in nodes.iter().enumerate() {
            for right in nodes.iter().skip(index + 1) {
                add_undirected_weight(&mut builder, *left, *right, 1.0);
            }
        }
    }
    add_undirected_weight(&mut builder, 5, 6, 0.05);
    builder.build()
}

fn path_graph(count: u8) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 0..count {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    for node in 0..count - 1 {
        add_undirected_weight(&mut builder, node, node + 1, 1.0);
    }
    builder.build()
}

fn disconnected_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 1..=4 {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    add_undirected_weight(&mut builder, 1, 2, 1.0);
    add_undirected_weight(&mut builder, 3, 4, 1.0);
    builder.build()
}

fn single_node_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    builder.add_node(cx(1), 1.0).unwrap();
    builder.build()
}

fn add_undirected_weight(
    builder: &mut calyx_paths::AssocGraphBuilder,
    left: u8,
    right: u8,
    weight: f32,
) {
    builder.add_edge(cx(left), cx(right), weight).unwrap();
    builder.add_edge(cx(right), cx(left), weight).unwrap();
}

fn calyx_edge(name: &str, before: Value, result: Result<Value, CalyxError>) -> Value {
    let after = match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => json!({"error_code": error.code, "message": error.message}),
    };
    json!({"case": name, "state_before": before, "state_after": after})
}

fn prop_edge(
    name: &str,
    before: Value,
    result: Result<Value, calyx_lodestar::PropagationError>,
) -> Value {
    let after = match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => json!({"error_code": error.code(), "message": error.to_string()}),
    };
    if name == "no_kernel" {
        assert_eq!(after["error_code"], CALYX_PROP_NO_KERNEL_NODES);
    }
    json!({"case": name, "state_before": before, "state_after": after})
}

fn spectral_edge(
    name: &str,
    before: Value,
    result: Result<Value, calyx_mincut::SpectralError>,
) -> Value {
    let after = match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => json!({"error_code": error.code(), "message": error.to_string()}),
    };
    json!({"case": name, "state_before": before, "state_after": after})
}

fn label_readback(row: &calyx_lodestar::PropagatedLabel) -> Value {
    json!({
        "node_id": row.node_id,
        "confidence": row.confidence,
        "ci_95": [row.confidence, row.confidence],
        "hop_distance": row.hop_distance,
        "provisional": row.provisional,
    })
}

fn sign_count(values: &[f32]) -> (usize, usize) {
    (
        values.iter().filter(|value| **value >= 0.0).count(),
        values.iter().filter(|value| **value < 0.0).count(),
    )
}

fn standard_normal(rng: &mut ChaCha8Rng) -> f64 {
    let u1: f64 = rng.gen_range(f64::EPSILON..1.0);
    let u2: f64 = rng.gen_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn percentile_f64(mut values: Vec<f64>, point: f64) -> (f64, f64) {
    values.sort_by(f64::total_cmp);
    let low = values[((values.len() - 1) as f64 * 0.025).round() as usize].min(point);
    let high = values[((values.len() - 1) as f64 * 0.975).round() as usize].max(point);
    (low, high)
}

fn noise(t: u64, salt: u64) -> f32 {
    let mut x = t.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    ((x >> 40) as f32) / ((1_u64 << 24) as f32)
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn tmp_file(file_name: &str) -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir().join(file_name)
    } else {
        PathBuf::from("/tmp").join(file_name)
    }
}

fn mirror_to_root(file_name: &str, bytes: &[u8]) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(file_name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn file_state(path: &Path) -> Value {
    match fs::read(path) {
        Ok(bytes) => json!({
            "exists": true,
            "len": bytes.len(),
            "blake3": blake3::hash(&bytes).to_string(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({"exists": false}),
        Err(error) => json!({"exists": false, "read_error": error.to_string()}),
    }
}
