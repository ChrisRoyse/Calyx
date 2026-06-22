use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    AnchorKind, Asymmetry, CxFlags, CxId, FixedClock, InputRef, LedgerRef, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, VaultId,
};
use calyx_lodestar::{
    GroundednessReport, InMemoryAnnIndex, InMemoryCorpus, Kernel, RecallQuery, RecallReport,
    RecallTestParams, build_kernel_index,
};
use calyx_oracle::{
    DomainId, HeldOutSplit, KernelRecallGate, ShortCircuit, TierMeasurementRequest,
    VaultSufficiencyAssay, measure_super_intelligence_tiers_1_to_3,
};
use serde_json::json;

const DOMAIN: &str = "ph50_t02_fsv";

#[test]
#[ignore = "manual FSV for issue #436 PH50 T02 tier 1-3 readbacks"]
fn issue436_super_intel_tiers_1_to_3_fsv_writes_readbacks() {
    let root = std::env::var_os("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .expect("set CALYX_FSV_ROOT");
    assert!(!root.exists(), "fresh FSV root required");
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let vault = durable_vault(&vault_dir, DOMAIN);
    let panel = panel();
    let clock = FixedClock::new(436);
    let (gate, held_out) = kernel_gate();

    put_sufficiency(&vault, &panel, DOMAIN, 0.46, 1.0, &[(SlotId::new(1), 0.04)]);
    let tier2_fails = measure(&vault, &gate, &held_out, &panel, DOMAIN, &clock);
    write_json(&root.join("tier2-form-panel-fails.json"), &tier2_fails);

    put_sufficiency(&vault, &panel, DOMAIN, 1.05, 1.0, &[(SlotId::new(1), 1.05)]);
    let all_pass = measure(&vault, &gate, &held_out, &panel, DOMAIN, &clock);
    write_json(&root.join("tiers-1-3-all-pass.json"), &all_pass);

    let empty_held_out = HeldOutSplit::new("empty-held-out", vec![cx(1)], Vec::new());
    let empty_edge = measure(&vault, &gate, &empty_held_out, &panel, DOMAIN, &clock);
    write_json(&root.join("edge-empty-held-out.json"), &empty_edge);

    let missing_domain = measure(
        &vault,
        &gate,
        &empty_held_out,
        &panel,
        "ph50_missing_domain",
        &clock,
    );
    write_json(&root.join("edge-domain-not-found.json"), &missing_domain);

    vault.flush().expect("flush FSV vault");
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "issue": 436,
            "vault": vault_dir,
            "vault_id": vault_id().to_string(),
            "domain": DOMAIN,
            "expected": {
                "tier2-form-panel-fails": {"failing_tier": "panel_sufficient", "overall": false},
                "tiers-1-3-all-pass": {"failing_tier": null, "overall": true},
                "edge-empty-held-out": {"failing_tier": "kernel_exists", "overall": false},
                "edge-domain-not-found": {"failing_tier": "oracle_clean", "overall": false}
            }
        }))
        .expect("manifest json"),
    )
    .expect("write manifest");
}

fn measure(
    vault: &AsterVault,
    gate: &KernelRecallGate<'_>,
    held_out: &HeldOutSplit,
    panel: &Panel,
    domain: &str,
    clock: &FixedClock,
) -> calyx_oracle::SuperIntelReport {
    let assay = VaultSufficiencyAssay::new(vault);
    measure_super_intelligence_tiers_1_to_3(TierMeasurementRequest {
        oracle: vault,
        assay: &assay,
        kernel: gate,
        panel,
        domain: DomainId::from(domain),
        held_out,
        clock,
        short_circuit: ShortCircuit::MeasureAll,
    })
}

fn durable_vault(vault_dir: &Path, domain: &str) -> AsterVault {
    let vault = AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue436-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let series = happy_series();
    for (cx_idx, rows) in series.iter().enumerate() {
        let cx_id = CxId::from_bytes([cx_idx as u8 + 1; 16]);
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx_id),
                encode::encode_constellation_base(&constellation(cx_id, domain))
                    .expect("encode base"),
            )
            .expect("write base");
        for (occ_idx, row) in rows.iter().enumerate() {
            let occurrence = Occurrence {
                id: OccurrenceId(occ_idx as u64),
                t_k: EpochSecs(1_000 + occ_idx as i64),
                context: OccurrenceContext::new(context(row)).expect("context"),
            };
            vault
                .write_cf(
                    ColumnFamily::Recurrence,
                    recurrence_key(cx_id, occ_idx as u64),
                    encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                        .expect("encode recurrence"),
                )
                .expect("write recurrence");
        }
    }
    vault
}

fn put_sufficiency(
    vault: &AsterVault,
    panel: &Panel,
    domain: &str,
    panel_bits: f32,
    entropy_bits: f32,
    slot_bits: &[(SlotId, f32)],
) {
    let key = AssayCacheKey::scoped(panel.version, domain, vault.vault_id(), AnchorKind::Reward);
    let mut store = AssayStore::default();
    store.put(
        key.clone(),
        AssaySubject::Panel,
        estimate(panel_bits, EstimatorKind::PanelSufficiency),
        "ph50 panel bits",
        1,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        estimate(entropy_bits, EstimatorKind::OutcomeEntropy),
        "ph50 entropy bits",
        1,
    );
    for (slot, bits) in slot_bits {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: *slot },
            estimate(*bits, EstimatorKind::Ksg),
            "ph50 lens bits",
            1,
        );
    }
    store.persist_to_vault(vault).expect("persist assay rows");
}

fn kernel_gate() -> (KernelRecallGate<'static>, HeldOutSplit) {
    let ids = [cx(101), cx(102)];
    let kernel = Box::leak(Box::new(kernel(&ids)));
    let mut embeddings = BTreeMap::new();
    embeddings.insert(ids[0], vec![1.0, 0.0]);
    embeddings.insert(ids[1], vec![0.0, 1.0]);
    let index = Box::leak(Box::new(
        build_kernel_index(kernel, &embeddings).expect("kernel index"),
    ));
    let rows = vec![
        RecallQuery {
            cx_id: ids[0],
            vector: vec![1.0, 0.0],
        },
        RecallQuery {
            cx_id: ids[1],
            vector: vec![0.0, 1.0],
        },
    ];
    let full = Box::leak(Box::new(InMemoryAnnIndex::new(rows.clone()).expect("full")));
    let corpus = Box::leak(Box::new(InMemoryCorpus::new("ph50-fsv-held-out", rows)));
    let gate = KernelRecallGate::new(
        index,
        full,
        corpus,
        RecallTestParams {
            held_out_fraction: 1.0,
            top_k: 1,
            rng_seed: 436,
            min_recall_ratio: 0.0,
        },
    );
    (
        gate,
        HeldOutSplit::new("held-out", vec![cx(1)], ids.to_vec()),
    )
}

fn kernel(members: &[CxId]) -> Kernel {
    Kernel {
        kernel_id: cx(200),
        panel_version: 50,
        anchor_kind: Some("reward".to_string()),
        corpus_shard_hash: [0; 32],
        members: members.to_vec(),
        kernel_graph: members.to_vec(),
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "ph50-fsv; trust=anchored".to_string(),
        warnings: Vec::new(),
    }
}

fn happy_series() -> Vec<Vec<Row>> {
    let mut series = vec![
        vec![v("pass", Some("pass")); 6],
        vec![
            v("pass", Some("pass")),
            v("pass", Some("pass")),
            v("fail", Some("fail")),
        ],
        vec![v("pass", Some("pass")); 2],
    ];
    for idx in 0..47 {
        let label = if idx % 2 == 0 { "pass" } else { "fail" };
        series.push(vec![v(label, Some(label))]);
    }
    series
}

#[derive(Clone)]
struct Row {
    verdict: &'static str,
    truth: Option<&'static str>,
}

fn v(verdict: &'static str, truth: Option<&'static str>) -> Row {
    Row { verdict, truth }
}

fn context(row: &Row) -> Vec<u8> {
    let mut value = json!({
        "oracle_verdict": { "value": { "text": row.verdict } },
        "outcome_anchor": { "value": { "text": row.verdict } }
    });
    if let Some(truth) = row.truth {
        value["ground_truth_anchor"] = json!({ "value": { "text": truth } });
    }
    serde_json::to_vec(&value).expect("context json")
}

fn constellation(cx_id: CxId, domain: &str) -> calyx_core::Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert("oracle.domain".to_string(), domain.to_string());
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn panel() -> Panel {
    Panel {
        version: 50,
        slots: vec![slot(1)],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Code,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("ph50-fsv".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 50,
    }
}

fn estimate(bits: f32, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::point(bits, 120, estimator, TrustTag::Trusted)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
