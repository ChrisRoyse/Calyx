use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, PanelRef,
    RetentionPolicy, Schema, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use calyx_aster::layers::kv::kv_key;
use calyx_aster::layers::relational::record_key;
use calyx_aster::layers::{KvLayer, RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::txn::{CALYX_TXN_TIMEOUT, TxnHandle};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    CxFlags, CxId, FixedClock, InputRef, LedgerRef, LensId, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use calyx_sextant::query::{
    AskSpec, FieldOp, FieldPredicate, GraphHop, KvLookup, RelationalFilter, UniversalQuery,
    VectorQuery, execute, plan,
};
use calyx_sextant::{CALYX_INVALID_ARGUMENT, CALYX_PLANNER_COST_CAP};
use serde_json::{Value, json};

#[path = "../sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::hex;

pub(super) const FIXED_TS: u64 = 1_785_500_467;
const VAULT_SALT: &[u8] = b"issue467-ph55";

pub(super) struct Collections {
    orders: Collection,
    kv_state: Collection,
    cxs: Collection,
    lens_id: LensId,
}

pub(super) fn create_collections(vault: &AsterVault<FixedClock>) -> Collections {
    let lens_id = LensId::from_parts("issue467-stub", b"weights", b"corpus", b"2xf32");
    let collections = Collections {
        orders: orders_collection(),
        kv_state: collection("kv_state", CollectionMode::KV, None),
        cxs: collection(
            "cxs",
            CollectionMode::Constellations,
            Some(PanelRef::new(lens_id)),
        ),
        lens_id,
    };
    create_collection(vault, collections.orders.clone()).unwrap();
    create_collection(vault, collections.kv_state.clone()).unwrap();
    create_collection(vault, collections.cxs.clone()).unwrap();
    collections
}

pub(super) fn scenario_a(vault: Arc<AsterVault<FixedClock>>, cols: &Collections) -> Value {
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(1);
    let cx = constellation(vault.vault_id(), "order #1 placed", 1);
    let keys = ReadKeys::new(cols, &pk, cx.cx_id);
    let before_seq = vault.latest_seq();
    let before = row_state(vault.as_ref(), before_seq, &keys);
    let mut txn = handle
        .begin_on(
            vault.as_ref(),
            IsolationLevel::Serializable,
            Some(5_000),
            Duration::from_millis(100),
        )
        .unwrap();
    txn.put_record(vault.as_ref(), &cols.orders, &pk, &order_row())
        .unwrap();
    txn.kv_set(vault.as_ref(), &cols.kv_state, 1, b"last_order", b"1", None)
        .unwrap();
    txn.put_constellation(vault.as_ref(), &cx).unwrap();
    assert_eq!(
        txn.get_record(vault.as_ref(), &cols.orders, &pk).unwrap(),
        Some(order_row())
    );
    assert_eq!(
        txn.kv_get(vault.as_ref(), &cols.kv_state, 1, b"last_order")
            .unwrap(),
        Some(b"1".to_vec())
    );
    let before_commit_external = row_state(vault.as_ref(), before_seq, &keys);
    assert_eq!(before_commit_external["relational_present"], false);
    assert_eq!(before_commit_external["kv_present"], false);
    assert_eq!(before_commit_external["base_present"], false);
    assert_eq!(before_commit_external["slot_00_present"], false);

    let deadlock = active_timeout(&handle, Arc::clone(&vault));
    let expected_seq = before_seq + 1;
    let commit_seq = txn.commit(vault.as_ref()).unwrap();
    assert_eq!(commit_seq, expected_seq);
    handle
        .begin_on(
            vault.as_ref(),
            IsolationLevel::Serializable,
            Some(5_000),
            Duration::from_millis(50),
        )
        .unwrap()
        .rollback()
        .unwrap();
    let after = row_state(vault.as_ref(), commit_seq, &keys);
    assert_shared_seq(&after, commit_seq);
    assert_eq!(
        RelationalLayer::new(vault.as_ref())
            .get_record_at(commit_seq, &cols.orders, &pk)
            .unwrap(),
        Some(order_row())
    );
    assert_eq!(
        KvLayer::new(vault.as_ref())
            .kv_get_at(commit_seq, &cols.kv_state, 1, b"last_order")
            .unwrap(),
        Some(b"1".to_vec())
    );
    let stored_cx = vault.get(cx.cx_id, commit_seq).unwrap();
    assert_eq!(stored_cx.cx_id, cx.cx_id);
    assert_ne!(stored_cx.provenance.hash, [0; 32]);
    json!({
        "before_seq": before_seq,
        "expected_seq": expected_seq,
        "commit_seq": commit_seq,
        "cx_id": cx.cx_id.to_string(),
        "txn_constellation_ledger_ref": {
            "seq": stored_cx.provenance.seq,
            "hash": hex(&stored_cx.provenance.hash)
        },
        "before": before,
        "before_commit_external_read": before_commit_external,
        "after": after,
        "deadlock_check": deadlock
    })
}

pub(super) fn scenario_b(
    vault: &AsterVault<FixedClock>,
    cols: &Collections,
    cx_id: CxId,
    commit_seq: u64,
) -> Value {
    let query = UniversalQuery {
        relational: Some(RelationalFilter {
            collection: cols.orders.clone(),
            predicates: vec![FieldPredicate {
                field: "qty".to_string(),
                op: FieldOp::Gte,
                value: json!(1),
            }],
            estimated_rows: Some(1),
        }),
        kv: Some(KvLookup {
            ns: "kv_state:1".to_string(),
            key: b"last_order".to_vec(),
        }),
        graph_hop: Some(GraphHop {
            from_cx_ids: vec![cx_id],
            hop_kind: "related".to_string(),
            max_hops: 1,
        }),
        vector: Some(VectorQuery {
            lens_ids: vec![cols.lens_id],
            query_vec: vec![0.1, 0.2],
            limit: 5,
        }),
        cost_cap_ms: Some(10_000),
        explain: true,
        ..UniversalQuery::default()
    };
    let planned = plan(vault, &query).unwrap();
    assert!(planned.steps.len() >= 4);
    let explain = planned.explain.clone().unwrap();
    assert!(
        explain
            .steps
            .iter()
            .all(|step| step.estimated_cost_ms > 0.0)
    );
    let result = execute(vault, planned).unwrap();
    assert!(result.elapsed_ms <= 10_000);
    assert!(has_qty_row(&result.rows, 7));
    assert!(has_kv_row(&result.rows, b"1"));
    assert!(result.rows.iter().any(|row| {
        row.key.as_bytes() == cx_id.as_bytes() && row.score.is_some() && row.ledger_ref.is_some()
    }));
    let row_seqs = row_seq_summary(vault, cols, cx_id, commit_seq);
    json!({
        "planned_steps": query_steps(&explain.steps),
        "estimated_cost_ms": explain.total_cost_ms,
        "elapsed_ms": result.elapsed_ms,
        "total_scanned": result.total_scanned,
        "rows": serde_json::to_value(&result.rows).unwrap(),
        "row_seqs": row_seqs
    })
}

pub(super) fn scenario_c(vault: &AsterVault<FixedClock>, cols: &Collections) -> Value {
    let before_seq = vault.latest_seq();
    let mut unbounded = cols.orders.clone();
    unbounded.txn_policy.cost_cap_ms = None;
    let query = UniversalQuery {
        relational: Some(RelationalFilter {
            collection: unbounded,
            predicates: Vec::new(),
            estimated_rows: Some(1_000_000),
        }),
        cost_cap_ms: None,
        explain: true,
        ..UniversalQuery::default()
    };
    let err = plan(vault, &query).unwrap_err();
    let after_seq = vault.latest_seq();
    assert_eq!(err.code, CALYX_PLANNER_COST_CAP);
    assert_eq!(before_seq, after_seq);
    json!({
        "before_seq": before_seq,
        "after_seq": after_seq,
        "executor_called": false,
        "error_code": err.code,
        "message": err.message
    })
}

pub(super) fn scenario_d(vault: &AsterVault<FixedClock>) -> Value {
    let cx = constellation(vault.vault_id(), "ask provenanced target", 2);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    let stored = vault.get(cx_id, vault.latest_seq()).unwrap();
    assert_ne!(stored.provenance.hash, [0; 32]);
    let query = UniversalQuery {
        ask: Some(AskSpec {
            question: "test question".to_string(),
            context_cx_ids: vec![cx_id],
            top_k: 1,
            oracle: false,
        }),
        cost_cap_ms: Some(5_000),
        explain: true,
        ..UniversalQuery::default()
    };
    let result = execute(vault, plan(vault, &query).unwrap()).unwrap();
    assert_eq!(result.rows.len(), 1);
    let ledger = result.rows[0].ledger_ref.as_ref().unwrap();
    assert_eq!(ledger, &stored.provenance);
    let ledger_hex = hex(&ledger.hash);
    println!("PH55 ASK ledger_ref seq={} hash={ledger_hex}", ledger.seq);
    json!({
        "cx_id": cx_id.to_string(),
        "rows": serde_json::to_value(&result.rows).unwrap(),
        "ledger_ref_seq": ledger.seq,
        "ledger_ref_hash": ledger_hex
    })
}

pub(super) fn edge_empty_ask(vault: &AsterVault<FixedClock>, cx_id: CxId) -> Value {
    let before_seq = vault.latest_seq();
    let query = UniversalQuery {
        ask: Some(AskSpec {
            question: "   ".to_string(),
            context_cx_ids: vec![cx_id],
            top_k: 1,
            oracle: false,
        }),
        cost_cap_ms: Some(5_000),
        ..UniversalQuery::default()
    };
    let err = execute(vault, plan(vault, &query).unwrap()).unwrap_err();
    let after_seq = vault.latest_seq();
    assert_eq!(err.code, CALYX_INVALID_ARGUMENT);
    assert_eq!(before_seq, after_seq);
    json!({
        "before_seq": before_seq,
        "after_seq": after_seq,
        "error_code": err.code,
        "message": err.message
    })
}

struct ReadKeys {
    relational: Vec<u8>,
    kv: Vec<u8>,
    base: Vec<u8>,
    slot_00: Vec<u8>,
}

impl ReadKeys {
    fn new(cols: &Collections, pk: &RecordKey, cx_id: CxId) -> Self {
        Self {
            relational: record_key(&cols.orders, pk).unwrap(),
            kv: kv_key(&cols.kv_state, 1, b"last_order"),
            base: base_key(cx_id),
            slot_00: slot_key(cx_id),
        }
    }
}

fn row_state(vault: &AsterVault<FixedClock>, seq: u64, keys: &ReadKeys) -> Value {
    json!({
        "seq": seq,
        "relational_seq": vault.seq_for_key_at(seq, ColumnFamily::Relational, &keys.relational).unwrap(),
        "kv_seq": vault.seq_for_key_at(seq, ColumnFamily::Kv, &keys.kv).unwrap(),
        "base_seq": vault.seq_for_key_at(seq, ColumnFamily::Base, &keys.base).unwrap(),
        "slot_00_seq": vault.seq_for_key_at(seq, ColumnFamily::slot(SlotId::new(0)), &keys.slot_00).unwrap(),
        "relational_present": vault.read_cf_at(seq, ColumnFamily::Relational, &keys.relational).unwrap().is_some(),
        "kv_present": vault.read_cf_at(seq, ColumnFamily::Kv, &keys.kv).unwrap().is_some(),
        "base_present": vault.read_cf_at(seq, ColumnFamily::Base, &keys.base).unwrap().is_some(),
        "slot_00_present": vault.read_cf_at(seq, ColumnFamily::slot(SlotId::new(0)), &keys.slot_00).unwrap().is_some()
    })
}

fn active_timeout(handle: &TxnHandle, vault: Arc<AsterVault<FixedClock>>) -> Value {
    let cloned = handle.clone();
    let result = thread::spawn(move || {
        cloned
            .begin_on(
                vault.as_ref(),
                IsolationLevel::Serializable,
                Some(5_000),
                Duration::from_millis(50),
            )
            .map(|txn| {
                drop(txn);
                "UNEXPECTED_OK".to_string()
            })
            .unwrap_or_else(|error| error.code.to_string())
    })
    .join()
    .unwrap();
    assert_eq!(result, CALYX_TXN_TIMEOUT);
    json!({"timeout_ms": 50, "error_code": result})
}

fn row_seq_summary(
    vault: &AsterVault<FixedClock>,
    cols: &Collections,
    cx_id: CxId,
    commit_seq: u64,
) -> Value {
    let pk = RecordKey::from_u64(1);
    let keys = ReadKeys::new(cols, &pk, cx_id);
    let summary = row_state(vault, vault.latest_seq(), &keys);
    assert_shared_seq(&summary, commit_seq);
    summary
}

fn assert_shared_seq(state: &Value, seq: u64) {
    for key in ["relational_seq", "kv_seq", "base_seq", "slot_00_seq"] {
        assert_eq!(state[key], json!(seq), "{key}");
    }
}

fn has_qty_row(rows: &[calyx_sextant::query::ProvenancedRow], qty: i64) -> bool {
    rows.iter().any(|row| {
        row.value
            .as_ref()
            .and_then(|value| value.get("qty"))
            .is_some_and(|value| value == &RecordValue::I64(qty))
    })
}

fn has_kv_row(rows: &[calyx_sextant::query::ProvenancedRow], expected: &[u8]) -> bool {
    rows.iter().any(|row| {
        row.value
            .as_ref()
            .and_then(|value| value.get("__value"))
            .is_some_and(|value| value == &RecordValue::Bytes(expected.to_vec()))
    })
}

fn orders_collection() -> Collection {
    let mut collection = collection("orders", CollectionMode::Records, None);
    collection.schema = Some(Schema::SchemaFull(vec![
        FieldDef::new("item", FieldType::Text, false),
        FieldDef::new("qty", FieldType::I64, false),
    ]));
    collection
}

fn collection(name: &str, mode: CollectionMode, panel: Option<PanelRef>) -> Collection {
    Collection {
        name: name.to_string(),
        mode,
        schema: Some(Schema::SchemaLess),
        panel,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy {
            isolation: IsolationLevel::Serializable,
            cost_cap_ms: Some(5_000),
        },
        tenant: TenantId::default(),
    }
}

fn order_row() -> Row {
    Row::new([
        ("item", RecordValue::Text("order #1 placed".to_string())),
        ("qty", RecordValue::I64(7)),
    ])
}

fn constellation(vault_id: VaultId, input: &str, idx: u64) -> calyx_core::Constellation {
    let mut input_hash = [0_u8; 32];
    input_hash.copy_from_slice(blake3::hash(input.as_bytes()).as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![idx as f32, 0.5],
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_input(input.as_bytes(), 1, VAULT_SALT),
        vault_id,
        panel_version: 1,
        created_at: FIXED_TS,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://issue467/{idx}")),
            redacted: false,
        },
        modality: Modality::Structured,
        slots,
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

pub(super) fn durable_vault(root: &Path) -> AsterVault<FixedClock> {
    AsterVault::new_durable_with_clock(
        root,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
        FixedClock::new(FIXED_TS),
    )
    .unwrap()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub(super) fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph55-fsv-test"))
}

pub(super) fn reset_root(root: &Path) {
    let text = root.display().to_string();
    assert!(
        text.contains("calyx-ph55-fsv") || text.contains("fsv-issue467"),
        "refusing to reset unexpected FSV root: {text}"
    );
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).unwrap();
}

fn query_steps(steps: &[calyx_sextant::query::ExplainStep]) -> Vec<Value> {
    steps
        .iter()
        .map(|step| {
            json!({
                "ordinal": step.ordinal,
                "kind": format!("{:?}", step.kind),
                "estimated_cost_ms": step.estimated_cost_ms
            })
        })
        .collect()
}
