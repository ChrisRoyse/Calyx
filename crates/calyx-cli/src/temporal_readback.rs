use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, DecayFunction, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId,
};
use calyx_sextant::{
    HnswIndex, PeriodicOptions, Query, SearchEngine, SlotIndexMap,
    TemporalFixedClock as FixedClock, TemporalPolicy, temporal_search,
};

const CONTENT_SLOT: SlotId = SlotId::new(8);
const TEMPORAL_SLOT: SlotId = SlotId::new(20);

pub fn readback_temporal_search(clock_fixed: i64, tz_offset_secs: i32) -> Result<(), String> {
    let engine = sample_engine(clock_fixed)?;
    let policy = policy_step()?;
    let query = Query {
        k: 2,
        explain: true,
        ..Query::new("temporal readback")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![CONTENT_SLOT, TEMPORAL_SLOT])
            .with_recall_k(3)
    };
    let result = temporal_search(
        &engine,
        &query,
        None,
        &policy,
        &FixedClock::new(clock_fixed),
        tz_offset_secs,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn sample_engine(clock_fixed: i64) -> Result<SearchEngine, String> {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(CONTENT_SLOT, 2, 42))
        .map_err(|error| error.to_string())?;
    map.register(HnswIndex::new(TEMPORAL_SLOT, 2, 43))
        .map_err(|error| error.to_string())?;
    let mut engine = SearchEngine::new(map);
    let rows = [
        (1, vec![1.0, 0.0], vec![0.0, 1.0], 100_000),
        (2, vec![0.98, 0.2], vec![0.0, 1.0], 500),
        (3, vec![0.0, 1.0], vec![1.0, 0.0], 100),
    ];
    for (seed, content, temporal, age_secs) in rows {
        let id = cx(seed);
        let created_at = created_at_secs(clock_fixed, age_secs);
        engine
            .indexes
            .insert(CONTENT_SLOT, id, dense(content), seed as u64)
            .map_err(|error| error.to_string())?;
        engine
            .indexes
            .insert(TEMPORAL_SLOT, id, dense(temporal), seed as u64)
            .map_err(|error| error.to_string())?;
        engine.put_constellation(row(seed, created_at));
    }
    Ok(engine)
}

fn policy_step() -> Result<TemporalPolicy, String> {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(None, None).map_err(|error| error.to_string())?,
        Default::default(),
        Default::default(),
        Default::default(),
        true,
    )
    .map_err(|error| error.to_string())
}

fn row(seed: u8, created_at: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/temporal-readback/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("temporal-readback".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "calyx-cli".to_string(),
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

fn created_at_secs(clock_fixed: i64, age_secs: i64) -> u64 {
    u64::try_from(clock_fixed.saturating_sub(age_secs)).unwrap_or(0)
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
