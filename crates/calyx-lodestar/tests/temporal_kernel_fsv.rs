use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_lodestar::{
    CALYX_LODESTAR_INVALID_FREQUENCY, CALYX_LODESTAR_MISSING_FREQUENCY, FREQ_WEIGHT,
    KernelGraphParams, TimeWindow, apply_frequency_bonuses, frequency_kernel_bonus,
    kernel_for_window_from_graph, kernel_weight_rows, select_kernel_graph,
};
use calyx_mincut::tarjan_scc;
use calyx_paths::AssocGraph;
use serde_json::{Value, json};

#[test]
#[ignore = "FSV trigger writes durable manual evidence under CALYX_LODESTAR_ISSUE389_FSV_DIR"]
fn issue389_lodestar_frequency_kernel_fsv_writes_artifacts() {
    let root = PathBuf::from(
        env::var("CALYX_LODESTAR_ISSUE389_FSV_DIR").expect("set CALYX_LODESTAR_ISSUE389_FSV_DIR"),
    );
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue389-lodestar-temporal-kernel-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let high = cx(1);
    let low = cx(2);
    let out = cx(3);
    put_base(&vault, high, None);
    put_base(&vault, low, None);
    put_base(&vault, out, None);
    for idx in 0..50 {
        append_time(&vault, high, 1_000 + idx);
    }
    append_time(&vault, low, 1_005);
    append_time(&vault, out, 5_000);
    vault.flush().expect("flush setup");
    let before = raw_state(&vault);

    let weights_graph = equal_score_graph(&[high, low]);
    let mut kernel_graph = scored_graph(&weights_graph, &[(high, 0.8), (low, 0.8)]);
    let reads = apply_frequency_bonuses(&mut kernel_graph, &vault).expect("apply frequency");
    let weights = kernel_weight_rows(&kernel_graph, &reads, 2);
    assert_eq!(weights[0].cx_id, high);
    assert_eq!(weights[0].frequency, 50);
    assert_eq!(weights[1].frequency, 1);

    let full_graph = equal_score_graph(&[high, low, out]);
    let window = TimeWindow::new(1_000, 1_100).expect("window");
    let window_kernel =
        kernel_for_window_from_graph(&vault, &full_graph, &window, 10).expect("window kernel");
    let window_ids = window_kernel
        .nodes
        .iter()
        .map(|node| node.cx_id)
        .collect::<Vec<_>>();
    assert!(window_ids.contains(&high));
    assert!(window_ids.contains(&low));
    assert!(!window_ids.contains(&out));
    let after_happy = raw_state(&vault);

    let empty_window = edge_empty_window(&vault, &full_graph);
    let missing_frequency = edge_missing_frequency(&vault);
    let invalid_frequency = edge_invalid_frequency(&vault);
    let after_edges = raw_state(&vault);

    let weight_artifact = json!({
        "schema_version": 1,
        "surface": "kernel-weights",
        "artifact_kind": "ph42.kernel-weights.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 389,
        "hand_computed_expected": {
            "high_frequency": 50,
            "low_frequency": 1,
            "baseline_betweenness": 0.8,
            "freq_weight": FREQ_WEIGHT,
            "high_bonus": frequency_kernel_bonus(50),
            "low_bonus": frequency_kernel_bonus(1),
            "high_total": 0.8 + FREQ_WEIGHT * f64::from(frequency_kernel_bonus(50)),
            "low_total": 0.8 + FREQ_WEIGHT * f64::from(frequency_kernel_bonus(1)),
            "expected_rank_1": high.to_string()
        },
        "trigger": {
            "operation": "apply_frequency_bonuses(kernel_graph, vault)",
            "intended_outcome": "base-CF recurrence.frequency raises equal-betweenness kernel weight"
        },
        "weights": weights,
        "frequency_reads": reads,
        "warnings": kernel_graph.warnings,
        "source_of_truth_bytes": {
            "vault_dir": vault_dir.display().to_string(),
            "before": before,
            "after_happy": after_happy,
            "after_edges": after_edges
        },
        "edges": {
            "empty_window": empty_window,
            "missing_frequency": missing_frequency,
            "invalid_frequency": invalid_frequency
        }
    });
    let window_artifact = json!({
        "schema_version": 1,
        "surface": "kernel-window",
        "artifact_kind": "ph42.kernel-window.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 389,
        "window": window,
        "hand_computed_expected": {
            "included": [high.to_string(), low.to_string()],
            "excluded": [out.to_string()],
            "reason": "high and low have occurrences in [1000,1100); out occurs at 5000"
        },
        "trigger": {
            "operation": "kernel_for_window_from_graph(vault, graph, [1000,1100), k=10)",
            "intended_outcome": "KernelResult scope is TimeWindow and nodes are limited to active CxIds"
        },
        "kernel_result": window_kernel
    });

    let weight_path = root.join("kernel-weights.json");
    let window_path = root.join("kernel-window.json");
    write_json(&weight_path, &weight_artifact);
    write_json(&window_path, &window_artifact);
    write_blake3_manifest(&root, &[weight_path.clone(), window_path.clone()]);
    println!("issue389_fsv_root={}", root.display());
    println!("issue389_kernel_weights={}", weight_path.display());
    println!("issue389_kernel_window={}", window_path.display());
    println!(
        "{}",
        serde_json::to_string_pretty(&weight_artifact).unwrap()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&window_artifact).unwrap()
    );
}

fn edge_empty_window(vault: &AsterVault, graph: &AssocGraph) -> Value {
    let before = raw_state(vault);
    let result =
        kernel_for_window_from_graph(vault, graph, &TimeWindow::new(7_000, 8_000).unwrap(), 10)
            .expect("empty window");
    let after = raw_state(vault);
    assert!(result.nodes.is_empty());
    json!({
        "expected": "no nodes because no recurrence occurrence falls in [7000,8000)",
        "actual_node_count": result.nodes.len(),
        "before": before,
        "after": after
    })
}

fn edge_missing_frequency(vault: &AsterVault) -> Value {
    let missing = cx(20);
    put_base(vault, missing, None);
    let before = raw_state(vault);
    let graph = equal_score_graph(&[missing]);
    let mut kernel_graph = scored_graph(&graph, &[(missing, 0.5)]);
    apply_frequency_bonuses(&mut kernel_graph, vault).expect("missing frequency is warning");
    let after = raw_state(vault);
    assert_eq!(kernel_graph.scores[0].frequency_bonus, 0.0);
    assert!(
        kernel_graph
            .warnings
            .iter()
            .any(|warning| warning.starts_with(CALYX_LODESTAR_MISSING_FREQUENCY))
    );
    json!({
        "expected_warning": CALYX_LODESTAR_MISSING_FREQUENCY,
        "actual_warnings": kernel_graph.warnings,
        "actual_bonus": kernel_graph.scores[0].frequency_bonus,
        "before": before,
        "after": after
    })
}

fn edge_invalid_frequency(vault: &AsterVault) -> Value {
    let bad = cx(21);
    put_base(vault, bad, Some(1.5));
    let before = raw_state(vault);
    let graph = equal_score_graph(&[bad]);
    let mut kernel_graph = scored_graph(&graph, &[(bad, 0.5)]);
    let error = apply_frequency_bonuses(&mut kernel_graph, vault).expect_err("invalid frequency");
    let after = raw_state(vault);
    assert_eq!(error.code(), CALYX_LODESTAR_INVALID_FREQUENCY);
    json!({
        "expected_code": CALYX_LODESTAR_INVALID_FREQUENCY,
        "actual_code": error.code(),
        "message": error.to_string(),
        "before": before,
        "after": after
    })
}

fn scored_graph(
    graph: &AssocGraph,
    betweenness_rows: &[(CxId, f64)],
) -> calyx_lodestar::KernelGraph {
    let betweenness = betweenness_rows.iter().copied().collect::<BTreeMap<_, _>>();
    let params = KernelGraphParams {
        target_fraction: 1.0,
        degree_weight: 0.0,
        betweenness_weight: 1.0,
        groundedness_weight: 0.0,
        ..KernelGraphParams::default()
    };
    select_kernel_graph(graph, &tarjan_scc(graph), &betweenness, &[], &params).unwrap()
}

fn equal_score_graph(ids: &[CxId]) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for id in ids {
        builder.add_node(*id, 1.0).unwrap();
    }
    for pair in ids.windows(2) {
        builder.add_edge(pair[0], pair[1], 1.0).unwrap();
    }
    builder.build()
}

fn append_time(vault: &AsterVault, cx_id: CxId, time: i64) {
    append_occurrence(
        vault,
        cx_id,
        EpochSecs(time),
        OccurrenceContext::new(format!("t={time}").into_bytes()).unwrap(),
        EpochSecs(time),
        RetentionPolicy::default(),
    )
    .unwrap();
}

fn put_base(vault: &AsterVault, cx_id: CxId, frequency: Option<f64>) {
    let mut cx = base_cx(cx_id);
    if let Some(frequency) = frequency {
        cx.scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    vault.put(cx).unwrap();
}

fn raw_state(vault: &AsterVault) -> Value {
    json!({
        "snapshot": vault.snapshot(),
        "base": raw_rows(vault, ColumnFamily::Base),
        "recurrence": raw_rows(vault, ColumnFamily::Recurrence),
        "ledger": raw_rows(vault, ColumnFamily::Ledger)
    })
}

fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Value {
    let rows = vault.scan_cf_at(vault.snapshot(), cf).expect("scan cf");
    json!({
        "row_count": rows.len(),
        "rows": rows.iter().map(|(key, value)| {
            json!({ "key_hex": hex(key), "value_hex": hex(value) })
        }).collect::<Vec<_>>()
    })
}

fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 42,
        created_at: 1_786_406_600,
        input_ref: InputRef {
            hash: [cx_id.to_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_blake3_manifest(root: &Path, paths: &[PathBuf]) {
    let mut manifest = String::new();
    for path in paths {
        let bytes = fs::read(path).expect("read artifact");
        let name = path.file_name().unwrap().to_string_lossy();
        manifest.push_str(&format!("{}  {name}\n", blake3::hash(&bytes).to_hex()));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), manifest).expect("write blake3 manifest");
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old fsv root");
    }
    fs::create_dir_all(path).expect("create fsv root");
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
