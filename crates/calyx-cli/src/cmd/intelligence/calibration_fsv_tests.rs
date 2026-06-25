use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_assay::AssayStore;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{AnchorKind, CxId, FixedClock, Panel, SlotId, VaultStore};
use calyx_loom::{CrossTermValue, LoomStore, agreement_scalar};
use calyx_oracle::{CALYX_ORACLE_INSUFFICIENT, DomainId, OracleError, check_sufficiency};
use serde_json::json;

use super::calibration_fsv_support::{
    DOMAIN, FsvPaths, KNOWN_PAIR_ANCHOR, MEDLINEPLUS_URL, assert_close_f32, assert_close_f64,
    durable_vault, panel_two_active, planted_known_pair_corpus, put_oracle_evidence, scan_count,
    slot, ungrounded_corpus, vault_id,
};
use super::core::{load_docs, read_json_row, write_json_row};
use super::model::{BitsOut, KernelOut, assay_key, kernel_key};
use super::{bits, kernel};

#[test]
fn planted_calibration_signals_roundtrip_from_durable_state() {
    let fsv = FsvPaths::new();
    let evidence_dir = fsv.case_dir.join("bits-kernel-vault");
    let vault = durable_vault(&evidence_dir, vault_id());
    let panel = panel_two_active();
    let anchor = AnchorKind::Label(KNOWN_PAIR_ANCHOR.to_string());
    let label = "label:metformin_type_2_diabetes";

    assert_eq!(scan_count(&vault, ColumnFamily::Base), 0);
    let ids = vault
        .put_batch(planted_known_pair_corpus(vault_id()))
        .expect("persist planted calibration corpus");
    vault.flush().expect("flush planted corpus");
    assert_eq!(ids.len(), 100);
    assert_eq!(scan_count(&vault, ColumnFamily::Base), 100);

    let docs = load_docs(&vault).expect("load durable planted docs");
    assert_eq!(docs.len(), 100);
    let first_positive = docs
        .values()
        .find(|cx| cx.metadata_value("drug") == Some("metformin"))
        .expect("metformin planted row exists");
    assert_eq!(
        first_positive.metadata_value("source_url"),
        Some(MEDLINEPLUS_URL)
    );

    let bits_key = assay_key(label);
    let bits_report = bits::calculate(&panel, &docs, &anchor, label, true, &bits_key)
        .expect("planted drug-disease bits should recover");
    let high = bits_report
        .per_slot
        .iter()
        .find(|slot| slot.slot == 0)
        .expect("slot 0 bits");
    let low = bits_report
        .per_slot
        .iter()
        .find(|slot| slot.slot == 1)
        .expect("slot 1 bits");
    assert_close_f64(high.bits, 0.5);
    assert_close_f64(low.bits, 0.0);
    let bits_bytes =
        write_json_row(&vault, ColumnFamily::Assay, bits_key.clone(), &bits_report).unwrap();
    let bits_raw = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Assay, &bits_key)
        .expect("read assay CF")
        .expect("bits row persisted");
    assert_eq!(bits_raw, bits_bytes);
    let bits_readback: BitsOut = read_json_row(&vault, ColumnFamily::Assay, &bits_key)
        .unwrap()
        .expect("bits JSON readback");
    assert_eq!(bits_readback, bits_report);

    let kernel_key = kernel_key(Some(label));
    let kernel_report =
        kernel::calculate(&docs, Some(&anchor)).expect("anchored kernel should ground");
    assert_eq!(kernel_report.total_cx, 100);
    assert_eq!(kernel_report.kernel_size, 1);
    assert_close_f32(kernel_report.recall, 1.0);
    assert!(kernel_report.grounding_gaps.is_empty());
    let kernel_bytes = write_json_row(
        &vault,
        ColumnFamily::Kernel,
        kernel_key.clone(),
        &kernel_report,
    )
    .unwrap();
    let kernel_raw = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Kernel, &kernel_key)
        .expect("read kernel CF")
        .expect("kernel row persisted");
    assert_eq!(kernel_raw, kernel_bytes);
    let kernel_readback: KernelOut = read_json_row(&vault, ColumnFamily::Kernel, &kernel_key)
        .unwrap()
        .expect("kernel JSON readback");
    assert_eq!(kernel_readback, kernel_report);

    let low_only_panel = Panel {
        slots: vec![slot(1)],
        ..panel.clone()
    };
    let assay_rows_before_low_edge = scan_count(&vault, ColumnFamily::Assay);
    let low_signal_error = bits::calculate(
        &low_only_panel,
        &docs,
        &anchor,
        label,
        false,
        b"bits\0low-only",
    )
    .expect_err("low-only planted panel must fail closed");
    assert_eq!(low_signal_error.code(), "CALYX_ASSAY_LOW_SIGNAL");
    assert_eq!(
        scan_count(&vault, ColumnFamily::Assay),
        assay_rows_before_low_edge
    );

    let insufficient = insufficient_sample_edge(&fsv.case_dir);
    let ungrounded = ungrounded_kernel_edge(&fsv.case_dir);
    let loom = loom_agreement_roundtrip(&fsv.case_dir);
    let oracle = oracle_gate_roundtrip(&fsv.case_dir, &panel);

    let artifact = json!({
        "schema": "calyx-medicalsearch-calibration-fsv-v1",
        "known_pair": {
            "drug": "metformin",
            "disease": "type_2_diabetes",
            "source_url": MEDLINEPLUS_URL,
            "anchor": label
        },
        "bits": {
            "vault_dir": evidence_dir.display().to_string(),
            "base_rows_before": 0,
            "base_rows_after": scan_count(&vault, ColumnFamily::Base),
            "assay_rows_after": scan_count(&vault, ColumnFamily::Assay),
            "n": bits_report.n,
            "slot0_bits": high.bits,
            "slot1_bits": low.bits,
            "panel_sufficiency": bits_report.panel_sufficiency,
            "assay_row_bytes": bits_raw.len()
        },
        "kernel": {
            "kernel_rows_after": scan_count(&vault, ColumnFamily::Kernel),
            "kernel_size": kernel_report.kernel_size,
            "recall": kernel_report.recall,
            "total_cx": kernel_report.total_cx,
            "kernel_row_bytes": kernel_raw.len()
        },
        "loom": loom,
        "oracle": oracle,
        "edge_cases": {
            "low_signal_code": low_signal_error.code(),
            "low_signal_assay_rows_before": assay_rows_before_low_edge,
            "low_signal_assay_rows_after": scan_count(&vault, ColumnFamily::Assay),
            "insufficient_samples": insufficient,
            "ungrounded_kernel": ungrounded
        }
    });
    let artifact_bytes = serde_json::to_vec_pretty(&artifact).expect("serialize artifact");
    fs::write(&fsv.artifact_path, artifact_bytes).expect("write FSV artifact");

    if !fsv.keep {
        fs::remove_dir_all(&fsv.case_dir).ok();
        fs::remove_file(&fsv.artifact_path).ok();
    }
}

fn insufficient_sample_edge(root: &Path) -> serde_json::Value {
    let dir = root.join("insufficient-samples-vault");
    let vault = durable_vault(&dir, vault_id());
    let anchor = AnchorKind::Label(KNOWN_PAIR_ANCHOR.to_string());
    vault
        .put_batch(planted_known_pair_corpus(vault_id()).into_iter().take(10))
        .expect("persist insufficient-sample corpus");
    vault.flush().expect("flush insufficient edge");
    let docs = load_docs(&vault).expect("load insufficient edge docs");
    let before = scan_count(&vault, ColumnFamily::Assay);
    let err = bits::calculate(
        &panel_two_active(),
        &docs,
        &anchor,
        "label:metformin_type_2_diabetes",
        false,
        b"bits\0insufficient",
    )
    .expect_err("under-50 planted outcomes must fail closed");
    let after = scan_count(&vault, ColumnFamily::Assay);
    assert_eq!(err.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(before, after);
    json!({
        "base_rows": scan_count(&vault, ColumnFamily::Base),
        "assay_rows_before": before,
        "assay_rows_after": after,
        "code": err.code()
    })
}

fn ungrounded_kernel_edge(root: &Path) -> serde_json::Value {
    let dir = root.join("ungrounded-kernel-vault");
    let vault = durable_vault(&dir, vault_id());
    vault
        .put_batch(ungrounded_corpus(vault_id()))
        .expect("persist ungrounded corpus");
    vault.flush().expect("flush ungrounded edge");
    let docs = load_docs(&vault).expect("load ungrounded docs");
    let before = scan_count(&vault, ColumnFamily::Kernel);
    let err = kernel::calculate(&docs, Some(&AnchorKind::TestPass))
        .expect_err("kernel with no anchors must fail closed");
    let after = scan_count(&vault, ColumnFamily::Kernel);
    assert_eq!(err.code(), "CALYX_KERNEL_UNGROUNDED");
    assert_eq!(before, after);
    json!({
        "base_rows": scan_count(&vault, ColumnFamily::Base),
        "kernel_rows_before": before,
        "kernel_rows_after": after,
        "code": err.code()
    })
}

fn loom_agreement_roundtrip(root: &Path) -> serde_json::Value {
    let dir = root.join("loom-xterm-cf");
    fs::create_dir_all(&dir).expect("create loom CF dir");
    let mut store = LoomStore::new(8);
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), vec![1.0, 0.0, 0.0]);
    slots.insert(SlotId::new(1), vec![1.0, 0.0, 0.0]);
    let direct_agreement = agreement_scalar(&slots[&SlotId::new(0)], &slots[&SlotId::new(1)])
        .expect("identical vectors should have agreement");
    assert_close_f32(direct_agreement, 1.0);
    assert_eq!(
        store
            .weave(CxId::from_bytes([77; 16]), &slots)
            .expect("weave agreement xterm"),
        1
    );
    let mut router = CfRouter::open(&dir, 1024).expect("open loom CF");
    assert_eq!(
        store
            .persist_xterms_to_aster(&mut router)
            .expect("persist xterm CF"),
        1
    );
    drop(router);
    let reopened = CfRouter::open(&dir, 1024).expect("reopen loom CF");
    let loaded = LoomStore::load_xterms_from_aster(&reopened, 8).expect("load xterm CF");
    let rows = loaded.xterm_rows();
    assert_eq!(rows.len(), 1);
    let CrossTermValue::Scalar(persisted) = rows[0].value.clone() else {
        panic!("agreement xterm must persist a scalar");
    };
    assert_close_f32(persisted, 1.0);
    let graph = loaded.agreement_graph();
    assert_eq!(graph.len(), 1);
    assert_close_f32(graph[0].raw_mean_agreement, 1.0);
    json!({
        "xterm_rows_readback": rows.len(),
        "direct_agreement": direct_agreement,
        "persisted_agreement": persisted,
        "agreement_weight": graph[0].agreement_weight,
        "graph_n": graph[0].n
    })
}

fn oracle_gate_roundtrip(root: &Path, panel: &Panel) -> serde_json::Value {
    let sufficient_dir = root.join("oracle-sufficient-vault");
    let sufficient_vault = durable_vault(&sufficient_dir, vault_id());
    put_oracle_evidence(&sufficient_vault, panel, 1.05, 1.0, &[(0, 0.50), (1, 0.55)]);
    let sufficient_rows = AssayStore::load_from_vault(&sufficient_vault)
        .expect("load sufficient assay rows")
        .len();
    let sufficient = check_sufficiency(
        &sufficient_vault,
        panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(41),
    )
    .expect("planted sufficient gate should pass");
    assert!(sufficient.sufficient);

    let insufficient_dir = root.join("oracle-insufficient-vault");
    let insufficient_vault = durable_vault(&insufficient_dir, vault_id());
    put_oracle_evidence(
        &insufficient_vault,
        panel,
        0.46,
        1.0,
        &[(0, 0.04), (1, 0.42)],
    );
    let insufficient_rows = AssayStore::load_from_vault(&insufficient_vault)
        .expect("load insufficient assay rows")
        .len();
    let err = check_sufficiency(
        &insufficient_vault,
        panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(51),
    )
    .expect_err("planted insufficient gate should refuse");
    assert_eq!(err.code(), CALYX_ORACLE_INSUFFICIENT);
    let OracleError::Insufficient { bound } = err else {
        panic!("expected insufficient bound");
    };
    assert!(!bound.sufficient);
    assert!(!bound.per_sensor_deficit.is_empty());

    json!({
        "sufficient": {
            "assay_rows_readback": sufficient_rows,
            "I_panel_oracle": sufficient.i_panel_oracle,
            "dpi_ceiling": sufficient.dpi_ceiling,
            "sufficient": sufficient.sufficient,
            "per_sensor_deficit_count": sufficient.per_sensor_deficit.len()
        },
        "insufficient": {
            "assay_rows_readback": insufficient_rows,
            "code": CALYX_ORACLE_INSUFFICIENT,
            "I_panel_oracle": bound.i_panel_oracle,
            "dpi_ceiling": bound.dpi_ceiling,
            "sufficient": bound.sufficient,
            "per_sensor_deficit_count": bound.per_sensor_deficit.len()
        }
    })
}
