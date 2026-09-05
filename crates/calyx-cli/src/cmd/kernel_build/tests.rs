use super::*;
use calyx_aster::ledger_view::read_ledger_seq;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, CxId, SlotId, SlotShape, SlotState, SlotVector, SparseEntry, VaultId, VaultStore,
};
use calyx_ledger::{EntryKind, decode};
use calyx_lodestar::{
    AsterAssocMetadata, AsterAssocNodeProps, FsKernelStore, PANEL_ASTER_ASSOC_COLLECTION,
    PANEL_RRF_K, RecallPassMode, TypedPanelVectors, encode_assoc_node_props, kernel_health,
    load_typed_panel_kernel_index, read_kernel_artifact, write_assoc_metadata,
};
use calyx_registry::{code_default, materialize_panel_template, persist_vault_panel_state};
use serde_json::json;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<KernelBuildArgs> {
    match super::parse_kernel_build(&toks(parts))? {
        Subcommand::KernelBuild(args) => Ok(args),
        _ => unreachable!("parse_kernel_build must return KernelBuild"),
    }
}

#[test]
fn defaults_apply_when_only_vault_given() {
    let args = parse(&["corpus"]).unwrap();
    assert_eq!(args.vault, "corpus");
    assert_eq!(args.held_out_fraction, DEFAULT_HELD_OUT_FRACTION);
    assert_eq!(args.top_k, DEFAULT_TOP_K);
    assert_eq!(args.min_recall, DEFAULT_MIN_RECALL);
}

#[test]
fn all_flags_parse() {
    let args = parse(&[
        "corpus",
        "--held-out-fraction",
        "0.01",
        "--top-k",
        "5",
        "--min-recall",
        "0.9",
        "--admission-queries",
        "tools/lawdemo/cuyahoga_admission_queries.v1.jsonl",
        "--resident-addr",
        "127.0.0.1:18460",
    ])
    .unwrap();
    assert!((args.held_out_fraction - 0.01).abs() < 1e-6);
    assert_eq!(args.top_k, 5);
    assert!((args.min_recall - 0.9).abs() < 1e-6);
    assert_eq!(
        args.admission_queries.as_deref(),
        Some(std::path::Path::new(
            "tools/lawdemo/cuyahoga_admission_queries.v1.jsonl"
        ))
    );
    assert_eq!(args.resident_addr, Some("127.0.0.1:18460".parse().unwrap()));
}

#[test]
fn admission_flags_are_atomic_and_loopback_only() {
    let missing_resident = parse(&[
        "corpus",
        "--admission-queries",
        "tools/lawdemo/cuyahoga_admission_queries.v1.jsonl",
    ])
    .unwrap_err();
    assert!(missing_resident.message().contains("supplied together"));

    let non_loopback = parse(&[
        "corpus",
        "--admission-queries",
        "tools/lawdemo/cuyahoga_admission_queries.v1.jsonl",
        "--resident-addr",
        "10.0.0.1:18460",
    ])
    .unwrap_err();
    assert!(non_loopback.message().contains("not loopback"));
}

#[test]
fn missing_vault_fails_closed() {
    let err = super::parse_kernel_build(&[]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn fraction_out_of_range_fails_closed() {
    assert_eq!(
        parse(&["corpus", "--held-out-fraction", "1.5"])
            .unwrap_err()
            .code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse(&["corpus", "--min-recall", "nan"])
            .unwrap_err()
            .code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn top_k_zero_fails_closed() {
    let err = parse(&["corpus", "--top-k", "0"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn unknown_flag_fails_closed() {
    let err = parse(&["corpus", "--bogus", "1"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("unexpected kernel-build flag"));
}

#[test]
fn run_persists_kernel_and_index_then_reads_back_source_of_truth() {
    let (home, vault_dir) = seed_home("happy", GraphShape::Triangle, true);

    run_kernel_build_with_home(
        &home,
        KernelBuildArgs {
            vault: "happy".to_string(),
            held_out_fraction: 1.0,
            top_k: 1,
            min_recall: 0.0,
            ..KernelBuildArgs::default()
        },
    )
    .unwrap();

    let store = FsKernelStore::new(&vault_dir);
    let generation = super::super::kernel_generation::load_current_generation(&vault_dir).unwrap();
    let kernel_id = generation.manifest.kernel_id;
    let kernel = read_kernel_artifact(kernel_id, &store).unwrap();
    // The no-flatten contract persists the typed_block_rrf index; read it back
    // through the same typed loader every production consumer uses (kernel_build
    // readback, kernel-answer, kernel-reproduce). The legacy `rrf`/flat-`dims`
    // panel loader no longer matches the sealed on-disk contract.
    let index = load_typed_panel_kernel_index(kernel_id, &store).unwrap();
    let health = kernel_health(kernel_id, &store).unwrap();

    assert_eq!(kernel.kernel_id, kernel_id);
    assert!(!kernel.members.is_empty());
    assert_eq!(index.rows().len(), kernel.members.len());
    assert_eq!(health.recall.pass_mode, RecallPassMode::Passed);
    assert_eq!(health.recall.min_recall_ratio, 0.0);
    assert!(store.kernel_file_path(kernel_id).exists());
    assert!(store.index_file_path(kernel_id).exists());

    // Physical ledger provenance: the build ledger row recorded in the CURRENT
    // manifest must exist on disk and decode to the Kernel entry with the exact
    // hash — proving the point-read append/readback path (issue #1863).
    let seq = generation.manifest.build_ledger_seq;
    let row = read_ledger_seq(&vault_dir, seq)
        .unwrap()
        .expect("kernel build ledger row physically present");
    let entry = decode(&row.bytes).unwrap();
    assert_eq!(entry.kind, EntryKind::Kernel);
    let entry_hash_hex = entry
        .entry_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(entry_hash_hex, generation.manifest.build_ledger_hash);
}

#[test]
fn strict_gate_refines_members_before_persisting_artifacts() {
    let (home, vault_dir) = seed_home("strict-gate", GraphShape::Triangle, true);

    run_kernel_build_with_home(
        &home,
        KernelBuildArgs {
            vault: "strict-gate".to_string(),
            held_out_fraction: 1.0,
            top_k: 1,
            min_recall: 1.0,
            ..KernelBuildArgs::default()
        },
    )
    .unwrap();

    let store = FsKernelStore::new(&vault_dir);
    let kernel_id = only_kernel_id(&vault_dir);
    let kernel = read_kernel_artifact(kernel_id, &store).unwrap();
    let index = load_typed_panel_kernel_index(kernel_id, &store).unwrap();

    // A strict 1.0 gate must be met by refining the kernel until every held-out
    // full top-1 (each node itself) is a member — so the sealed panel index has
    // one row per member and the recall contract holds exactly.
    assert_eq!(kernel.recall.ratio, 1.0);
    assert_eq!(kernel.members.len(), 30);
    assert_eq!(index.rows().len(), kernel.members.len());
    assert!(
        kernel
            .warnings
            .iter()
            .any(|warning| warning.starts_with("CALYX_KERNEL_RECALL_REFINED"))
    );
}

#[test]
fn zero_held_out_fraction_fails_closed_without_persisting_artifacts() {
    let (home, vault_dir) = seed_home("zero-held-out", GraphShape::Triangle, true);

    let err = run_kernel_build_with_home(
        &home,
        KernelBuildArgs {
            vault: "zero-held-out".to_string(),
            held_out_fraction: 0.0,
            top_k: 1,
            min_recall: 1.0,
            ..KernelBuildArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_RECALL_EMPTY_CORPUS");
    assert!(!vault_dir.join("idx/kernel/CURRENT").exists());
}

#[test]
fn empty_graph_fails_closed_before_artifacts() {
    let (home, vault_dir) = seed_home("empty", GraphShape::Empty, true);

    let err = run_kernel_build_with_home(
        &home,
        KernelBuildArgs {
            vault: "empty".to_string(),
            held_out_fraction: 1.0,
            top_k: 1,
            min_recall: 0.0,
            ..KernelBuildArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("woven association graph"));
    assert!(!vault_dir.join("idx/kernel/CURRENT").exists());
}

#[test]
fn unanchored_graph_fails_closed_before_artifacts() {
    let (home, vault_dir) = seed_home("unanchored", GraphShape::Triangle, false);

    let err = run_kernel_build_with_home(
        &home,
        KernelBuildArgs {
            vault: "unanchored".to_string(),
            held_out_fraction: 1.0,
            top_k: 1,
            min_recall: 0.0,
            ..KernelBuildArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("no anchored nodes"));
    assert!(!vault_dir.join("idx/kernel/CURRENT").exists());
}

#[test]
fn acyclic_selected_graph_fails_closed_without_member_substitution() {
    let (home, vault_dir) = seed_home("acyclic", GraphShape::Acyclic, true);

    let err = run_kernel_build_with_home(
        &home,
        KernelBuildArgs {
            vault: "acyclic".to_string(),
            held_out_fraction: 1.0,
            top_k: 1,
            min_recall: 0.0,
            ..KernelBuildArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_EMPTY_RESULT");
    assert!(!vault_dir.join("idx/kernel/CURRENT").exists());
}

#[test]
fn rss_budget_parses_and_rejects_bad_values() {
    assert_eq!(super::parse_rss_budget(" 4096 ").unwrap(), 4096);
    assert_eq!(
        super::parse_rss_budget("0").unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        super::parse_rss_budget("not-a-number").unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn projected_peak_over_budget_is_saturating() {
    // No budget or within budget -> None.
    assert_eq!(super::projected_peak_over_budget(10, 5, None), None);
    assert_eq!(super::projected_peak_over_budget(10, 5, Some(100)), None);
    // Over budget -> the projected peak.
    assert_eq!(
        super::projected_peak_over_budget(80, 40, Some(100)),
        Some(120)
    );
    // Saturating add never wraps the comparison.
    assert_eq!(
        super::projected_peak_over_budget(u64::MAX, 1, Some(u64::MAX - 1)),
        Some(u64::MAX)
    );
}

#[test]
fn enforce_rss_budget_fails_closed_over_budget_and_passes_otherwise() {
    // No budget configured is a no-op.
    super::enforce_rss_budget("noop", None, 0).unwrap();
    // A 1-byte budget is impossible for a live process (real /proc VmRSS is
    // millions of bytes), so it must refuse fail-closed with backpressure.
    let refused = super::enforce_rss_budget("tiny", Some(1), 0).unwrap_err();
    assert_eq!(refused.code(), "CALYX_BACKPRESSURE");
    assert!(refused.message().contains("projected peak RSS"));
    // A huge budget passes.
    super::enforce_rss_budget("ample", Some(u64::MAX), 0).unwrap();
}

#[test]
fn rss_tracker_records_real_proc_samples() {
    let mut tracker = super::RssTracker::new();
    tracker.sample("phase_a");
    tracker.sample("phase_b");
    let report = tracker.report();
    let samples = report["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2);
    // The /proc probe is exercised: peak RSS of a running test process is > 0.
    assert!(report["peak_rss_bytes"].as_u64().unwrap() > 0);
    assert_eq!(samples[0]["phase"], "phase_a");
    assert!(samples[0]["rss_bytes"].as_u64().unwrap() > 0);
}

#[derive(Clone, Copy)]
enum GraphShape {
    Empty,
    Triangle,
    Acyclic,
}

fn seed_home(
    name: &str,
    shape: GraphShape,
    anchored: bool,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let home =
        std::env::temp_dir().join(format!("calyx-kernel-build-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("vaults")).unwrap();
    let vault_id = vault_id();
    let vault_dir = home.join("vaults").join(vault_id.to_string());
    std::fs::write(
        home.join("vaults").join("index.json"),
        serde_json::to_vec_pretty(&json!({
            "vaults": [{
                "name": name,
                "vault_id": vault_id.to_string(),
                "path": format!("vaults/{vault_id}"),
                "panel_template": "text-default"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let materialized = materialize_panel_template(&code_default(), 1).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(materialized.panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault_dir, &materialized.panel, &materialized.registry).unwrap();
    // Every active typed content lane (no-flatten panel contract), keyed by its
    // declared slot shape. This mirrors `weave::content_lens_slots` exactly
    // (state=Active, not retrieval_only, ALL shapes — not dense-only) so the
    // fixture carries the same ordered typed-block contract `kernel-build`
    // reconstructs from the live panel (kernel_build.rs:305-326).
    let mut content_slots = materialized
        .panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active && !slot.retrieval_only)
        .map(|slot| (slot.slot_id, slot.shape))
        .collect::<Vec<_>>();
    content_slots.sort_by_key(|(slot, _)| *slot);
    content_slots.dedup_by_key(|(slot, _)| *slot);
    assert!(
        content_slots.len() >= 2,
        "panel/RRF kernel-build needs >=2 typed content lanes"
    );
    let embedding_slots = content_slots
        .iter()
        .map(|(slot, _)| *slot)
        .collect::<Vec<_>>();
    let graph = PlainGraph::new(&vault, PANEL_ASTER_ASSOC_COLLECTION).unwrap();
    if !matches!(shape, GraphShape::Empty) {
        for seed in 1..=30u8 {
            // The no-flatten contract stores a complete typed panel per node in
            // `typed_embeddings`; the legacy flat `embedding`/`embeddings` fields
            // are forbidden and rejected by kernel-build (kernel_build.rs:378).
            let props = AsterAssocNodeProps {
                typed_embeddings: panel_typed_vectors(seed, &content_slots),
                anchors: anchored
                    .then(|| AnchorKind::Label("answer".to_string()))
                    .into_iter()
                    .collect(),
                ..Default::default()
            };
            graph
                .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
                .unwrap();
        }
    }
    match shape {
        GraphShape::Empty => {}
        GraphShape::Triangle => {
            graph.put_edge(cx(1), "assoc", cx(2), b"1").unwrap();
            graph.put_edge(cx(2), "assoc", cx(3), b"1").unwrap();
            graph.put_edge(cx(3), "assoc", cx(1), b"1").unwrap();
        }
        GraphShape::Acyclic => {
            graph.put_edge(cx(1), "assoc", cx(2), b"1").unwrap();
            graph.put_edge(cx(2), "assoc", cx(3), b"1").unwrap();
        }
    }
    let graph_source_seq = vault.latest_seq();
    write_assoc_metadata(
        &vault,
        PANEL_ASTER_ASSOC_COLLECTION,
        &AsterAssocMetadata {
            retention_horizon: None,
            embedding_slot: None,
            embedding_slots,
            // Match the real `weave-loom` no-flatten typed-block/RRF contract
            // (weave/graph_contract.rs); kernel-build fails closed on any other
            // fusion tag (kernel_build.rs:277).
            fusion: Some("typed_block_rrf".to_string()),
            rrf_k: Some(PANEL_RRF_K),
            panel_version: Some(u64::from(materialized.panel.version)),
            graph_source_seq: Some(graph_source_seq),
            knn: Some(16),
            edge_cos_threshold: None,
            edge_score_threshold: Some(0.5),
        },
    )
    .unwrap();
    graph.rebuild_csr(vault.snapshot()).unwrap();
    vault.flush().unwrap();
    (home, vault_dir)
}

/// Deterministic, finite, non-zero typed per-lane vectors for a synthetic panel
/// node — one `SlotVector` per contracted content slot, shaped exactly as the
/// slot declares (dense/sparse/multi). Distinct per node and per slot so the
/// corpus is not degenerate, yet a node's own vectors are its unique nearest
/// self (cosine 1.0), which is what the RRF recall/admission contract rides on.
/// No mock data: these are real f32 measurements matching each lane's schema.
fn panel_typed_vectors(seed: u8, content_slots: &[(SlotId, SlotShape)]) -> TypedPanelVectors {
    content_slots
        .iter()
        .map(|(slot, shape)| (*slot, typed_lane(seed, *slot, *shape)))
        .collect()
}

fn typed_lane(seed: u8, slot: SlotId, shape: SlotShape) -> SlotVector {
    let base =
        |k: u32| 0.5 + 0.1 * f32::from(seed) + 0.2 * f32::from(slot.get()) + 0.01 * (k as f32);
    match shape {
        SlotShape::Dense(dim) => SlotVector::Dense {
            dim,
            data: (0..dim).map(base).collect(),
        },
        // A fixed, strictly increasing support shared by every node (so cosine
        // is well-defined and non-degenerate) with per-node distinct finite
        // values. Eight entries is ample for the 30_522-dim lexical lane.
        SlotShape::Sparse(dim) => SlotVector::Sparse {
            dim,
            entries: (0..8u32)
                .filter(|idx| *idx < dim)
                .map(|idx| SparseEntry {
                    idx,
                    val: base(idx),
                })
                .collect(),
        },
        // Not produced by the default panels exercised here, but keep the
        // fixture total over the SlotVector contract so a new active Multi lane
        // fails loudly at the schema, not silently.
        SlotShape::Multi { token_dim } => SlotVector::Multi {
            token_dim,
            tokens: vec![(0..token_dim).map(base).collect()],
        },
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn only_kernel_id(vault_dir: &std::path::Path) -> CxId {
    super::super::kernel_generation::load_current_generation(vault_dir)
        .unwrap()
        .manifest
        .kernel_id
}
