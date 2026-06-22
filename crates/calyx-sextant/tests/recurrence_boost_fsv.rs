use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, recurrence_prefix_range};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence, read_series,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, BoostConfig, CxFlags, DecayFunction, FusionWeights, InputRef,
    LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_sextant::{
    CALYX_SEXTANT_RECURRENCE_READ_ERROR, CausalConfidence, FreshnessTag, Hit, ProvenanceSource,
    TemporalFixedClock as FixedClock, TemporalPolicy, TemporalSearchInput, TimeWindow,
    recurrence_boost_score, temporal_search_from_primary_with_recurrence,
};
use serde_json::json;
use sextant_support::cx_u8_fill as cx;

const QUERY_TIME: i64 = 1_000_000;
const CONTENT_SCORE: f32 = 0.70;
const CONTENT_SLOT: SlotId = SlotId::new(8);
const TEMPORAL_SLOT: SlotId = SlotId::new(20);
const CX_A: u8 = 0xA1;
const CX_B: u8 = 0xB2;

#[test]
#[ignore = "issue #391 manual FSV trigger; set CALYX_SEXTANT_ISSUE391_FSV_DIR"]
fn issue391_recurrence_boost_fsv_artifacts() {
    let root = fsv_root();
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue391-recurrence-boost",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    seed_ab(&vault);
    vault.flush().expect("flush recurrence vault");

    let result = temporal_search_from_primary_with_recurrence(
        TemporalSearchInput {
            primary_hits: vec![
                hit(CX_A, 1).with_explain("issue391-fsv"),
                hit(CX_B, 2).with_explain("issue391-fsv"),
            ],
            temporal_weight_used: 0.0,
            final_k: 2,
            window: Some(TimeWindow::all()),
            policy: &policy(),
            clock: &FixedClock::new(QUERY_TIME),
            tz_offset_secs: 0,
            primary_slots_used: vec![CONTENT_SLOT],
            temporal_slots_excluded: vec![TEMPORAL_SLOT],
            window_recall: Default::default(),
        },
        &vault,
    )
    .expect("temporal recurrence search");
    let a = result
        .hits
        .iter()
        .find(|hit| hit.cx_id == cx(CX_A))
        .unwrap();
    let b = result
        .hits
        .iter()
        .find(|hit| hit.cx_id == cx(CX_B))
        .unwrap();
    let boost_a = a.explain.as_ref().unwrap().recurrence_boost.unwrap().total;
    let boost_b = b.explain.as_ref().unwrap().recurrence_boost.unwrap().total;
    let expected_delta = CONTENT_SCORE * (boost_a - boost_b);
    let actual_delta = a.score - b.score;

    write_json(&root.join("temporal-search-result.json"), &result);
    write_json(
        &root.join("expected-arithmetic.json"),
        &json!({
            "content_score": CONTENT_SCORE,
            "boost_a": boost_a,
            "boost_b": boost_b,
            "expected_delta": expected_delta,
            "actual_delta": actual_delta,
            "delta_matches": (actual_delta - expected_delta).abs() <= 1.0e-5,
            "a_final_gt_b_final": a.score > b.score
        }),
    );
    write_json(
        &root.join("base-a-readback.json"),
        &base_readback(&vault, CX_A),
    );
    write_json(
        &root.join("base-b-readback.json"),
        &base_readback(&vault, CX_B),
    );
    write_json(
        &root.join("recurrence-series-readback.json"),
        &json!({
            "a": read_series(&vault, cx(CX_A)).unwrap(),
            "b": read_series(&vault, cx(CX_B)).unwrap(),
            "recurrence_cf_rows": recurrence_rows(&vault, CX_A)
        }),
    );
    edge_readbacks(&root, &vault);
    write_hashes(&root);
}

fn seed_ab(vault: &AsterVault) {
    vault.put(row(CX_A, None)).expect("put A");
    vault.put(row(CX_B, None)).expect("put B");
    for idx in 0..50 {
        append_occurrence_at(
            vault,
            CX_A,
            QUERY_TIME - (50 - idx) * 60,
            format!("A-{idx}"),
        );
    }
    append_occurrence_at(vault, CX_B, QUERY_TIME - 86_400, "B-singleton");
}

fn edge_readbacks(root: &Path, vault: &AsterVault) {
    let zero = 0xC3;
    let missing = 0xD4;
    let invalid = 0xE5;
    vault.put(row(zero, Some(0.0))).expect("put zero edge");
    vault
        .put(row(invalid, Some(1.5)))
        .expect("put invalid edge");
    let zero_before = base_readback(vault, zero);
    let zero_score = recurrence_boost_score(cx(zero), vault, QUERY_TIME, &Default::default())
        .expect("zero score");
    let missing_error = recurrence_boost_score(cx(missing), vault, QUERY_TIME, &Default::default())
        .expect_err("missing base");
    let invalid_before = base_readback(vault, invalid);
    let invalid_error = recurrence_boost_score(cx(invalid), vault, QUERY_TIME, &Default::default())
        .expect_err("invalid scalar");
    write_json(
        &root.join("edge-readbacks.json"),
        &json!({
            "zero_frequency": {
                "before": zero_before,
                "after": base_readback(vault, zero),
                "score": zero_score
            },
            "missing_base": {
                "before_base_present": false,
                "after_base_present": false,
                "actual_code": missing_error.code,
                "expected_code": CALYX_SEXTANT_RECURRENCE_READ_ERROR
            },
            "invalid_frequency": {
                "before": invalid_before,
                "after": base_readback(vault, invalid),
                "actual_code": invalid_error.code,
                "expected_code": CALYX_SEXTANT_RECURRENCE_READ_ERROR
            }
        }),
    );
}

fn base_readback(vault: &AsterVault, seed: u8) -> serde_json::Value {
    let bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx(seed)))
        .expect("read base")
        .expect("base present");
    let decoded = encode::decode_constellation_base(&bytes).expect("decode base");
    json!({
        "cx_id": cx(seed),
        "raw_blake3": blake3::hash(&bytes).to_string(),
        "raw_hex_prefix": hex_prefix(&bytes),
        "frequency": decoded.scalars.get(FREQUENCY_SCALAR).copied()
    })
}

fn recurrence_rows(vault: &AsterVault, seed: u8) -> Vec<serde_json::Value> {
    let range = recurrence_prefix_range(cx(seed));
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Recurrence)
        .expect("scan recurrence")
        .into_iter()
        .filter(|(key, _)| range.contains(key))
        .map(|(key, value)| json!({"key": hex_prefix(&key), "value_hash": blake3::hash(&value).to_string()}))
        .collect()
}

fn policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        Default::default(),
        Default::default(),
        FusionWeights::new(1.0, 0.0, 0.0).unwrap(),
        BoostConfig {
            post_retrieval_alpha: 0.0,
            ..BoostConfig::default()
        },
        true,
    )
    .unwrap()
}

fn append_occurrence_at(vault: &AsterVault, seed: u8, time: i64, context: impl Into<String>) {
    append_occurrence(
        vault,
        cx(seed),
        EpochSecs(time),
        OccurrenceContext::new(context.into()).expect("context"),
        EpochSecs(QUERY_TIME),
        RetentionPolicy::default(),
    )
    .expect("append occurrence");
}

fn hit(seed: u8, rank: usize) -> Hit {
    Hit {
        cx_id: cx(seed),
        score: CONTENT_SCORE,
        rank,
        event_time_secs: Some(QUERY_TIME - 600),
        temporal_scores: None,
        causal_confidence: CausalConfidence::Absent,
        causal_gate: None,
        per_lens: Vec::new(),
        cross_terms_used: false,
        guard: None,
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        provenance_source: ProvenanceSource::Stub,
        freshness: FreshnessTag::fresh(0),
        explain: None,
    }
}

fn row(seed: u8, frequency: Option<f64>) -> calyx_core::Constellation {
    let mut scalars = BTreeMap::new();
    if let Some(frequency) = frequency {
        scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: QUERY_TIME as u64,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/issue391/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::<SlotId, SlotVector>::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("issue391".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "calyx-sextant-fsv".to_string(),
            observed_at: QUERY_TIME as u64,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write json");
}

fn write_hashes(root: &Path) {
    let mut rows = Vec::new();
    for entry in fs::read_dir(root).expect("read fsv root") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
            let bytes = fs::read(entry.path()).expect("read artifact");
            rows.push(format!(
                "{}  {}",
                blake3::hash(&bytes),
                entry.file_name().to_string_lossy()
            ));
        }
    }
    rows.sort();
    fs::write(root.join("BLAKE3SUMS.txt"), rows.join("\n")).expect("write sums");
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old fsv root");
    }
    fs::create_dir_all(path).expect("create fsv root");
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_SEXTANT_ISSUE391_FSV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("calyx-issue391-fsv-{}", std::process::id()))
        })
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(32)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap()
}
