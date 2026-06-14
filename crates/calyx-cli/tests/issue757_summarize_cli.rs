use std::process::Command;

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, VaultId};
use calyx_lodestar::{
    AsterAssocMetadata, AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION,
    encode_assoc_node_props, write_assoc_metadata,
};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn temp_root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("calyx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn seed_vault(dir: &std::path::Path) {
    let vault = AsterVault::new_durable(
        dir,
        vault_id(),
        b"calyx-summarize-cli".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    write_assoc_metadata(
        &vault,
        DEFAULT_ASTER_ASSOC_COLLECTION,
        &AsterAssocMetadata {
            retention_horizon: Some(1),
        },
    )
    .unwrap();
    for seed in 1..=6u8 {
        let props = AsterAssocNodeProps {
            embedding: Some(vec![seed as f32, 1.0]),
            ts: Some(u64::from(seed)),
            anchors: (seed == 1)
                .then(|| AnchorKind::Label("domain".to_string()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        graph
            .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
            .unwrap();
    }
    for (src, dst) in [(1, 2), (2, 3), (3, 1), (4, 5), (5, 6), (6, 4), (3, 4)] {
        graph.put_edge(cx(src), "assoc", cx(dst), b"1").unwrap();
    }
    vault.flush().unwrap();
}

#[test]
fn summarize_cli_writes_json_and_aster_ledger_tail() {
    let root = temp_root("issue757-summarize-cli");
    let vault = root.join("vault.calyx");
    let out = root.join("summary.json");
    seed_vault(&vault);

    let output = Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args([
            "summarize",
            "--vault",
            vault.to_str().unwrap(),
            "--scope",
            r#"{"kind":"all_associations"}"#,
            "--out",
            out.to_str().unwrap(),
            "--anchor-label",
            "domain",
            "--max-kernel-size",
            "6",
            "--recall-held-out",
            "1.0",
            "--recall-top-k",
            "6",
            "--recall-min-ratio",
            "0.0",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("kernel_size"));

    let summary: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert!(summary["kernel_only_recall"].as_f64().unwrap() > 0.0);
    assert!(summary["kernel_size"].as_u64().unwrap() > 0);

    let tail = Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args([
            "ledger-tail",
            "--vault",
            vault.to_str().unwrap(),
            "--last",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        tail.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&tail.stderr)
    );
    let tail_text = String::from_utf8_lossy(&tail.stdout);
    assert!(tail_text.contains("SUMMARIZE_INVOKED"));
    assert!(tail_text.contains("kernel_only_recall"));

    let _ = std::fs::remove_dir_all(root);
}
