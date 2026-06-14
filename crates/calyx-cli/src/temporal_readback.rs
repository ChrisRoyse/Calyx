use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{OccurrenceContext, RetentionPolicy, append_occurrence};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, BoostConfig, CxFlags, CxId, DecayFunction, FusionWeights,
    InputRef, LedgerRef, Modality, SlotId, VaultId, VaultStore,
};
use calyx_sextant::{
    CausalConfidence, FreshnessTag, Hit, ProvenanceSource, TemporalFixedClock as FixedClock,
    TemporalPolicy, TemporalSearchInput, TimeWindow, temporal_search_from_primary_with_recurrence,
};

const CONTENT_SLOT: SlotId = SlotId::new(8);
const TEMPORAL_SLOT: SlotId = SlotId::new(20);
const QUERY_SCORE: f32 = 0.70;
const CX_A: u8 = 0xA1;
const CX_B: u8 = 0xB2;

pub fn readback_temporal_search(clock_fixed: i64, tz_offset_secs: i32) -> crate::error::CliResult {
    let root = std::env::temp_dir().join(format!(
        "calyx-temporal-search-recurrence-readback-{}",
        std::process::id()
    ));
    reset_dir(&root)?;
    let vault = AsterVault::new_durable(
        &root,
        vault_id(),
        b"temporal-readback-recurrence",
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    seed_recurrence(&vault, clock_fixed)?;

    let policy = policy_recurrence_only()?;
    let result = temporal_search_from_primary_with_recurrence(
        TemporalSearchInput {
            primary_hits: vec![
                hit(CX_A, 1, event_time(clock_fixed, 600)).with_explain("temporal_readback"),
                hit(CX_B, 2, event_time(clock_fixed, 600)).with_explain("temporal_readback"),
            ],
            temporal_weight_used: 0.0,
            final_k: 2,
            window: Some(TimeWindow::all()),
            policy: &policy,
            clock: &FixedClock::new(clock_fixed),
            tz_offset_secs,
            primary_slots_used: vec![CONTENT_SLOT],
            temporal_slots_excluded: vec![TEMPORAL_SLOT],
            window_recall: Default::default(),
        },
        &vault,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn seed_recurrence(vault: &AsterVault, clock_fixed: i64) -> std::result::Result<(), String> {
    vault
        .put(row(CX_A, created_at_secs(clock_fixed, 3_000)))
        .map_err(|error| error.to_string())?;
    vault
        .put(row(CX_B, created_at_secs(clock_fixed, 86_400)))
        .map_err(|error| error.to_string())?;
    for idx in 0..50 {
        append_occurrence(
            vault,
            cx(CX_A),
            EpochSecs(event_time(clock_fixed, (50 - idx) * 60)),
            OccurrenceContext::new(format!("A-{idx}")).map_err(|error| error.to_string())?,
            EpochSecs(event_time(clock_fixed, 0)),
            RetentionPolicy::default(),
        )
        .map_err(|error| error.to_string())?;
    }
    append_occurrence(
        vault,
        cx(CX_B),
        EpochSecs(event_time(clock_fixed, 86_400)),
        OccurrenceContext::new("B-singleton").map_err(|error| error.to_string())?,
        EpochSecs(event_time(clock_fixed, 0)),
        RetentionPolicy::default(),
    )
    .map_err(|error| error.to_string())?;
    vault.flush().map_err(|error| error.to_string())
}

fn policy_recurrence_only() -> Result<TemporalPolicy, String> {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        Default::default(),
        Default::default(),
        FusionWeights::new(1.0, 0.0, 0.0).map_err(|error| error.to_string())?,
        BoostConfig {
            post_retrieval_alpha: 0.0,
            ..BoostConfig::default()
        },
        true,
    )
    .map_err(|error| error.to_string())
}

fn hit(seed: u8, rank: usize, event_time_secs: i64) -> Hit {
    Hit {
        cx_id: cx(seed),
        score: QUERY_SCORE,
        rank,
        event_time_secs: Some(event_time_secs),
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

fn row(seed: u8, created_at: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
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
        metadata: BTreeMap::new(),
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

fn reset_dir(path: &Path) -> std::result::Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn event_time(clock_fixed: i64, age_secs: i64) -> i64 {
    clock_fixed.saturating_sub(age_secs).max(0)
}

fn created_at_secs(clock_fixed: i64, age_secs: i64) -> u64 {
    u64::try_from(event_time(clock_fixed, age_secs)).unwrap_or(0)
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap()
}
