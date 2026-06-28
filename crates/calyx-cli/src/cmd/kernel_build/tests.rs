use super::*;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, VaultId};
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, FsKernelStore, RecallPassMode,
    encode_assoc_node_props, kernel_health, load_kernel_index, read_kernel_artifact,
};
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
    ])
    .unwrap();
    assert!((args.held_out_fraction - 0.01).abs() < 1e-6);
    assert_eq!(args.top_k, 5);
    assert!((args.min_recall - 0.9).abs() < 1e-6);
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
        },
    )
    .unwrap();

    let store = FsKernelStore::new(&vault_dir);
    let kernel_id = only_kernel_id(&vault_dir);
    let kernel = read_kernel_artifact(kernel_id, &store).unwrap();
    let index = load_kernel_index(kernel_id, &store).unwrap();
    let health = kernel_health(kernel_id, &store).unwrap();

    assert_eq!(kernel.kernel_id, kernel_id);
    assert!(!kernel.members.is_empty());
    assert_eq!(index.rows().len(), kernel.members.len());
    assert_eq!(health.recall.pass_mode, RecallPassMode::Passed);
    assert_eq!(health.recall.min_recall_ratio, 0.0);
    assert!(store.kernel_file_path(kernel_id).exists());
    assert!(store.index_file_path(kernel_id).exists());
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
        },
    )
    .unwrap();

    let store = FsKernelStore::new(&vault_dir);
    let kernel_id = only_kernel_id(&vault_dir);
    let kernel = read_kernel_artifact(kernel_id, &store).unwrap();
    let index = load_kernel_index(kernel_id, &store).unwrap();

    assert_eq!(kernel.recall.ratio, 1.0);
    assert_eq!(kernel.recall.tau_star_estimate, 1);
    assert!(kernel.recall.tau_star_exact);
    assert_eq!(kernel.members.len(), 30);
    assert_eq!(index.rows().len(), 30);
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
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_RECALL_EMPTY_CORPUS");
    assert!(!vault_dir.join("idx").exists());
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
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("woven association graph"));
    assert!(!vault_dir.join("idx").exists());
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
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("no anchored nodes"));
    assert!(!vault_dir.join("idx").exists());
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
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_EMPTY_RESULT");
    assert!(!vault_dir.join("idx").exists());
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

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .unwrap();
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    if !matches!(shape, GraphShape::Empty) {
        for seed in 1..=30u8 {
            let props = AsterAssocNodeProps {
                embedding: Some(vec![seed as f32, (31 - seed) as f32]),
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
    vault.flush().unwrap();
    (home, vault_dir)
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn only_kernel_id(vault_dir: &std::path::Path) -> CxId {
    let kernel_root = vault_dir.join("idx").join("kernel");
    let dirs = std::fs::read_dir(&kernel_root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dirs.len(), 1);
    dirs[0].file_name().to_str().unwrap().parse().unwrap()
}
