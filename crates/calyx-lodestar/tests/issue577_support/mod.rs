use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, EpochSecs, IngestInput, TauStrategy, TctCosineConfig,
};
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::stream::{BackpressureGuard, QuantizeOnlineConfig, StreamIngester};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, Clock, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, LensId,
    Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_forge::quant::QuantLevel;
use calyx_ledger::decode as decode_ledger;
use calyx_lodestar::{
    AsterAssocMetadata, AsterAssocNodeProps, AsterSummarizeRequest, CollectionId,
    DEFAULT_ASTER_ASSOC_COLLECTION, RecallTestParams, Scope, ScopeCache, SummarizeParams,
    SummarizeResult, encode_assoc_node_props, summarize_vault_as_of, summarize_vault_latest,
    write_assoc_metadata,
};
use calyx_loom::{
    AuditEntry, ReactiveRowKind, TriggerFired, decode_audit_entry, decode_trigger_fired,
    reactive_row_key,
};
use serde_json::{Value, json};

pub const PANEL_VERSION: u32 = 41;
pub const SERIES_RAW: &[u8] = b"issue577-recurring-stream";
pub const COLLECTION: &str = DEFAULT_ASTER_ASSOC_COLLECTION;
pub const SLOT_CONTENT: SlotId = SlotId::new(0);
pub const SLOT_TIME: SlotId = SlotId::new(20);
pub const SUMMARY_HORIZON: u64 = 300;
pub const NAMED_ARTIFACTS: &[&str] = &[
    "ph72_stream_stats.json",
    "ph72_backpressure.json",
    "ph72_trigger_fired.json",
    "ph72_trigger_audit.json",
    "ph72_asof_500.json",
    "ph72_asof_1000.json",
    "ph72_horizon_error.json",
    "ph72_summarize.json",
    "ph72_summarize_asof.json",
];

#[derive(Clone, Debug)]
pub struct StepClock(Arc<AtomicU64>);

impl StepClock {
    pub fn new(start: u64) -> Self {
        Self(Arc::new(AtomicU64::new(start)))
    }

    pub fn set(&self, value: u64) {
        self.0.store(value, Ordering::SeqCst);
    }
}

impl Clock for StepClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE577_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("calyx-issue577-ph72-{}", std::process::id()))
        })
}

pub fn clean_dir(dir: &Path) {
    fs::remove_dir_all(dir).ok();
    fs::create_dir_all(dir).unwrap();
}

pub fn write_json(root: &Path, name: &str, value: Value) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

pub fn assert_named_artifacts(root: &Path) {
    for name in NAMED_ARTIFACTS {
        let path = root.join(name);
        let bytes = fs::read(&path).unwrap_or_else(|_| panic!("missing {}", path.display()));
        let _: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("invalid JSON {}", path.display()));
    }
}

pub fn open_vault(dir: &Path, salt: &[u8]) -> (AsterVault<StepClock>, StepClock) {
    let clock = StepClock::new(1);
    let options = VaultOptions {
        dedup_policy: Some(recurrence_policy()),
        ..VaultOptions::default()
    };
    let vault =
        AsterVault::new_durable_with_clock(dir, vault_id(), salt.to_vec(), options, clock.clone())
            .unwrap();
    (vault, clock)
}

pub fn stream_ingester(
    vault: Arc<AsterVault<StepClock>>,
    capacity: usize,
) -> StreamIngester<StepClock> {
    StreamIngester::new(vault, stream_config(), BackpressureGuard::new(capacity, 0))
}

pub fn recurrence_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SLOT_CONTENT],
            TauStrategy::PerSlot(vec![(SLOT_CONTENT, 0.90)]),
            DedupAction::RecurrenceSeries,
        )
        .unwrap(),
    )
}

pub fn stream_config() -> QuantizeOnlineConfig {
    QuantizeOnlineConfig::new(LensId::from_bytes([0x77; 16]), QuantLevel::Bits3p5)
}

pub fn stream_input(raw: &[u8], index: usize) -> IngestInput {
    IngestInput::new(raw.to_vec(), PANEL_VERSION, Modality::Text)
        .with_slot(
            SLOT_CONTENT,
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        )
        .with_slot(
            SLOT_TIME,
            SlotVector::Dense {
                dim: 2,
                data: temporal_vec(index).to_vec(),
            },
        )
        .with_temporal_slot(SLOT_TIME)
}

pub fn put_time_cx(vault: &AsterVault<StepClock>, clock: &StepClock, raw: &[u8], at: u64) -> CxId {
    clock.set(at);
    let cx_id = vault.cx_id_for_input(raw, 1);
    let mut slots = BTreeMap::new();
    slots.insert(
        SLOT_CONTENT,
        SlotVector::Dense {
            dim: 2,
            data: vec![at as f32, at as f32 + 1.0],
        },
    );
    vault
        .put(Constellation {
            cx_id,
            vault_id: vault.vault_id(),
            panel_version: 1,
            created_at: at,
            input_ref: InputRef {
                hash: *blake3::hash(raw).as_bytes(),
                pointer: None,
                redacted: true,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                redacted_input: true,
                ..CxFlags::default()
            },
        })
        .unwrap();
    cx_id
}

pub fn seed_summary_graph(vault: &AsterVault<StepClock>, clock: &StepClock, nodes: usize) {
    let graph = PlainGraph::new(vault, COLLECTION).unwrap();
    clock.set(0);
    write_assoc_metadata(
        vault,
        COLLECTION,
        &AsterAssocMetadata {
            retention_horizon: Some(SUMMARY_HORIZON),
        },
    )
    .unwrap();
    for i in 1..=nodes {
        let t = (i as u64) * 10;
        clock.set(t);
        let cx = summary_cx(i);
        let props = AsterAssocNodeProps {
            embedding: Some(vec![i as f32, 1.0, (i % 7) as f32]),
            ts: Some(t),
            anchors: ((i - 1) % 5 == 0)
                .then(|| AnchorKind::Label("ph72".to_string()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        graph
            .put_node(cx, &encode_assoc_node_props(&props).unwrap())
            .unwrap();
        if i % 5 != 1 {
            graph
                .put_edge(summary_cx(i - 1), "assoc", cx, b"1")
                .unwrap();
        }
        if i % 5 == 0 {
            graph
                .put_edge(cx, "assoc", summary_cx(i - 4), b"1")
                .unwrap();
        }
    }
}

pub fn summarize_latest(vault: &AsterVault<StepClock>, max: usize) -> SummarizeResult {
    let mut cache = ScopeCache::new(32);
    summarize_vault_latest(vault, request(max), &mut cache, &FixedClock::new(7_000)).unwrap()
}

pub fn summarize_asof(vault: &AsterVault<StepClock>, t: u64, max: usize) -> SummarizeResult {
    let mut cache = ScopeCache::new(32);
    summarize_vault_as_of(vault, request(max), t, &mut cache, &FixedClock::new(7_000)).unwrap()
}

pub fn request(max: usize) -> AsterSummarizeRequest<'static> {
    AsterSummarizeRequest {
        collection: COLLECTION,
        scope: Scope::Collection {
            id: CollectionId::from(COLLECTION),
        },
        params: Some(SummarizeParams {
            max_kernel_size: Some(max),
            anchor_kind: Some(AnchorKind::Label("ph72".to_string())),
            ..Default::default()
        }),
        recall_params: RecallTestParams {
            held_out_fraction: 1.0,
            top_k: max.max(1),
            rng_seed: 57,
            min_recall_ratio: 0.0,
        },
    }
}

pub fn audit_entries(vault: &AsterVault<StepClock>) -> Vec<AuditEntry> {
    reactive_rows(vault)
        .into_iter()
        .filter_map(|(key, value)| {
            let parts = reactive_row_key(&key).unwrap();
            (parts.kind == ReactiveRowKind::Audit).then(|| decode_audit_entry(&value).unwrap())
        })
        .collect()
}

pub fn fired_events(vault: &AsterVault<StepClock>) -> Vec<TriggerFired> {
    reactive_rows(vault)
        .into_iter()
        .filter_map(|(key, value)| {
            let parts = reactive_row_key(&key).unwrap();
            (parts.kind == ReactiveRowKind::Fired).then(|| decode_trigger_fired(&value).unwrap())
        })
        .collect()
}

pub fn ledger_hash(vault: &AsterVault<StepClock>, seq: u64) -> [u8; 32] {
    let bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .unwrap()
        .expect("ledger row");
    decode_ledger(&bytes).unwrap().entry_hash
}

pub fn ledger_payloads(vault: &AsterVault<StepClock>) -> Vec<Value> {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .filter_map(|(_, bytes)| {
            serde_json::from_slice(&decode_ledger(&bytes).unwrap().payload).ok()
        })
        .collect()
}

pub fn stream_stats_json(stats: &calyx_aster::stream::StreamStats) -> Value {
    json!({
        "ingested": stats.ingested,
        "backpressured": stats.backpressured,
        "quantized": stats.quantized,
        "batches": stats.batches,
    })
}

pub fn summary_json(result: &SummarizeResult) -> Value {
    json!({
        "scope_hash": hex_bytes(&result.scope_hash),
        "kernel_ids": result.kernel_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "kernel_size": result.kernel_size,
        "kernel_only_recall": result.kernel_only_recall,
        "grounded_fraction": result.grounded_fraction,
        "approx_factor": result.approx_factor,
        "ledger_ref": {
            "seq": result.ledger_ref.seq,
            "hash": hex_bytes(&result.ledger_ref.hash),
        },
    })
}

pub fn summary_cx(index: usize) -> CxId {
    CxId::from_bytes([index as u8; 16])
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn reactive_rows(vault: &AsterVault<StepClock>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Reactive)
        .unwrap()
}

fn temporal_vec(index: usize) -> [f32; 2] {
    let phase = (index % 4) as f32;
    [phase, 3.0 - phase]
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub fn epoch(index: usize) -> EpochSecs {
    EpochSecs((index as i64) + 1)
}
