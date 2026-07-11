use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CalyxError, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_sextant::{HnswIndex, SearchEngine, SlotIndexMap, TemporalSearchResult};
use serde_json::json;

pub const SLOT_A: SlotId = SlotId::new(8);
pub const SLOT_B: SlotId = SlotId::new(9);
pub const GUARD_SLOT: SlotId = SlotId::new(7);
pub const QUERY_TIME: i64 = 1_000_000;
pub const IN_WINDOW_SEED: u8 = 15;
const FUSED_ORDER: [u8; 10] = [1, 11, 2, 12, 3, 13, 4, 14, 5, 15];
const VAULT_SALT: &[u8] = b"issue1382-window-recall-fsv";
const GRAPH_PREFIX: &str = "issue1382/window-recall/v1";

pub fn durable_fixture_engine(vault_dir: &Path) -> (SearchEngine, serde_json::Value) {
    assert!(!vault_dir.exists(), "FSV vault path must be new");
    let vault = open_vault(vault_dir);
    for (slot, seed, rank, created_at) in fixture_rows() {
        vault
            .put(row(slot, seed, rank, created_at))
            .expect("persist fixture constellation");
    }
    vault.flush().expect("flush fixture vault");
    drop(vault);

    let vault = open_vault(vault_dir);
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SLOT_A, 2, 42)).unwrap();
    map.register(HnswIndex::new(SLOT_B, 2, 43)).unwrap();
    let mut engine = SearchEngine::new(map);
    let snapshot = vault.snapshot();
    for (slot, seed, _, _) in fixture_rows() {
        let stored = vault
            .get(cx(seed), snapshot)
            .expect("read persisted fixture constellation");
        let vector = stored
            .slots
            .get(&slot)
            .cloned()
            .expect("persisted primary vector");
        engine
            .indexes
            .insert(slot, stored.cx_id, vector, u64::from(seed))
            .expect("rebuild primary index from Aster row");
        engine.put_constellation(stored);
    }
    let physical = json!({
        "base": cf_summary(&vault, ColumnFamily::Base),
        "slot_7_guard": cf_summary(&vault, ColumnFamily::slot(GUARD_SLOT)),
        "slot_8_primary": cf_summary(&vault, ColumnFamily::slot(SLOT_A)),
        "slot_9_primary": cf_summary(&vault, ColumnFamily::slot(SLOT_B)),
        "ledger": cf_summary(&vault, ColumnFamily::Ledger),
        "latest_seq": snapshot,
    });
    assert_eq!(physical["base"]["rows"], 10);
    assert_eq!(physical["slot_7_guard"]["rows"], 10);
    assert_eq!(physical["slot_8_primary"]["rows"], 5);
    assert_eq!(physical["slot_9_primary"]["rows"], 5);
    assert_eq!(physical["ledger"]["rows"], 10);
    (engine, physical)
}

pub fn authoritative_evidence(
    physical_fixture: serde_json::Value,
    filter_deepen: &TemporalSearchResult,
    guard_deepen: &TemporalSearchResult,
    true_exhaustion: &TemporalSearchResult,
    filter_cap: &CalyxError,
) -> Vec<(&'static str, serde_json::Value)> {
    let timeline = fixture_rows()
        .into_iter()
        .map(|(slot, seed, rank, created_at)| {
            json!({
                "slot": slot.get(),
                "seed": seed,
                "slot_rank": rank,
                "created_at": created_at,
                "in_window": seed == IN_WINDOW_SEED,
            })
        })
        .collect::<Vec<_>>();
    vec![
        (
            "plan",
            json!({
                "schema_version": 1,
                "issue": 1382,
                "source": "durable_aster_fixture_reopened_before_search",
                "primary_slots": [SLOT_A.get(), SLOT_B.get()],
                "guard_slot": GUARD_SLOT.get(),
                "requested_k": 1,
                "requested_recall_k": 2,
                "bounded_caps": [10, 4],
            }),
        ),
        ("timeline", json!({"schema_version": 1, "rows": timeline})),
        (
            "truth",
            json!({
                "schema_version": 1,
                "fused_order": FUSED_ORDER,
                "in_window_seeds": [IN_WINDOW_SEED],
                "filter_15_seeds": [IN_WINDOW_SEED],
                "guard_in_region_seeds": [IN_WINDOW_SEED],
                "union_bound": 10,
                "exhaustion_evidence": "pre_filter_pre_guard_fused_candidates",
            }),
        ),
        (
            "admission",
            json!({
                "schema_version": 1,
                "status": "deterministic_regression_fixture",
                "production_panel_gate": false,
                "reason": "validates bounded recall control flow, not learned-lens quality",
            }),
        ),
        (
            "roster/a35",
            json!({
                "status": "not_applicable",
                "production_panel_gate": false,
                "fixture_slots": [SLOT_A.get(), SLOT_B.get()],
            }),
        ),
        (
            "roster/a37",
            json!({
                "status": "not_applicable",
                "production_diversity_claim": false,
                "reason": "hand-known deterministic vectors",
            }),
        ),
        (
            "roster/a38",
            json!({
                "status": "not_applicable",
                "production_topology_claim": false,
                "reason": "two-slot bounded-recall regression",
            }),
        ),
        ("physical/fixture", physical_fixture),
        (
            "report",
            json!({
                "schema_version": 1,
                "filter_deepen": filter_deepen,
                "guard_deepen": guard_deepen,
                "true_exhaustion": true_exhaustion,
                "filter_cap": {
                    "code": filter_cap.code,
                    "message": filter_cap.message.as_str(),
                    "remediation": filter_cap.remediation,
                },
            }),
        ),
    ]
}

pub fn persist_and_reopen_graph_evidence(
    vault_dir: &Path,
    evidence: Vec<(&str, serde_json::Value)>,
) -> serde_json::Value {
    let expected = evidence
        .into_iter()
        .map(|(suffix, value)| {
            (
                format!("{GRAPH_PREFIX}/{suffix}").into_bytes(),
                serde_json::to_vec(&value).expect("encode Graph evidence"),
            )
        })
        .collect::<Vec<_>>();
    let vault = open_vault(vault_dir);
    let ledger_payload = serde_json::to_vec(&json!({
        "event": "issue1382_window_recall_fsv",
        "graph_rows": expected.len(),
    }))
    .expect("encode ledger payload");
    let evidence_seq = vault
        .write_cf_batch_with_ledger_entry(
            expected
                .iter()
                .cloned()
                .map(|(key, value)| (ColumnFamily::Graph, key, value)),
            EntryKind::Measure,
            SubjectId::Query(blake3::hash(GRAPH_PREFIX.as_bytes()).as_bytes().to_vec()),
            ledger_payload,
            ActorId::Service("calyx-sextant-fsv".to_string()),
        )
        .expect("commit authoritative Graph evidence");
    vault.flush().expect("flush Graph evidence");
    drop(vault);

    let reopened = open_vault(vault_dir);
    let snapshot = reopened.snapshot();
    let mut readback = Vec::with_capacity(expected.len());
    for (key, expected_value) in expected {
        let actual = reopened
            .read_cf_at(snapshot, ColumnFamily::Graph, &key)
            .expect("read Graph evidence")
            .expect("Graph evidence row present");
        assert_eq!(actual, expected_value, "Graph evidence bytes changed");
        readback.push(json!({
            "key": String::from_utf8(key).expect("UTF-8 Graph key"),
            "bytes": actual.len(),
            "blake3": blake3::hash(&actual).to_hex().to_string(),
            "value": serde_json::from_slice::<serde_json::Value>(&actual)
                .expect("decode reopened Graph value"),
        }));
    }
    assert_eq!(
        reopened
            .scan_cf_at(snapshot, ColumnFamily::Graph)
            .expect("scan Graph evidence")
            .len(),
        readback.len()
    );
    assert_eq!(
        reopened
            .scan_cf_at(snapshot, ColumnFamily::Ledger)
            .expect("scan evidence ledger")
            .len(),
        11
    );
    println!(
        "ASTER_REOPEN_READBACK seq={evidence_seq} graph_rows={} ledger_rows=11",
        readback.len()
    );
    serde_json::Value::Array(readback)
}

pub fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

pub fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn fixture_rows() -> Vec<(SlotId, u8, u8, u64)> {
    let mut rows = Vec::with_capacity(10);
    for rank in 1..=5_u8 {
        rows.push((SLOT_A, rank, rank, out_of_window_created_at()));
        let b_seed = rank + 10;
        let created_at = if b_seed == IN_WINDOW_SEED {
            in_window_created_at()
        } else {
            out_of_window_created_at()
        };
        rows.push((SLOT_B, b_seed, rank, created_at));
    }
    rows
}

fn row(
    primary_slot: SlotId,
    seed: u8,
    slot_rank: u8,
    created_at: u64,
) -> calyx_core::Constellation {
    let guard_vector = if seed == IN_WINDOW_SEED {
        dense(vec![1.0, 0.0])
    } else {
        dense(vec![0.0, 1.0])
    };
    let mut slots = BTreeMap::new();
    slots.insert(primary_slot, dense(vec![1.0, 0.2 * f32::from(slot_rank)]));
    slots.insert(GUARD_SLOT, guard_vector);
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/window-recall-fsv/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("window-recall-fsv".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "issue-1382".to_string(),
            observed_at: created_at,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn cf_summary(vault: &AsterVault, cf: ColumnFamily) -> serde_json::Value {
    let mut rows = vault
        .scan_cf_at(vault.snapshot(), cf)
        .expect("scan physical CF rows");
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = blake3::Hasher::new();
    for (key, value) in &rows {
        hash.update(&(key.len() as u64).to_be_bytes());
        hash.update(key);
        hash.update(&(value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    json!({
        "rows": rows.len(),
        "bytes": rows.iter().map(|(key, value)| key.len() + value.len()).sum::<usize>(),
        "blake3": hash.finalize().to_hex().to_string(),
    })
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::open(
        vault_dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable issue1382 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid FSV vault id")
}

fn in_window_created_at() -> u64 {
    (QUERY_TIME - 600) as u64
}

fn out_of_window_created_at() -> u64 {
    (QUERY_TIME - 100_000) as u64
}
