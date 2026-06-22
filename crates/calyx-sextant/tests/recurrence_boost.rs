use std::collections::BTreeMap;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, InputRef, LedgerRef, Modality, RecurrenceBoostConfig,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_sextant::{
    CALYX_SEXTANT_RECURRENCE_READ_ERROR, CausalConfidence, FreshnessTag, Hit, ProvenanceSource,
    TemporalPolicy, TemporalScores, apply_temporal_boost, apply_temporal_boost_with_recurrence,
    frequency_kernel_bonus, recurrence_boost_evidence, recurrence_boost_score,
};
use sextant_support::cx_u8_fill as cx;

const QUERY_TIME: i64 = 1_000_000;
const EPSILON: f32 = 1.0e-5;

#[test]
fn zero_frequency_and_no_occurrence_returns_zero() {
    let vault = vault();
    vault.put(row(1, Some(0.0))).expect("put base");

    let score =
        recurrence_boost_score(cx(1), &vault, QUERY_TIME, &Default::default()).expect("score");

    assert_eq!(score, 0.0);
}

#[test]
fn frequency_ten_and_thirty_minute_recency_match_formula() {
    let vault = vault();
    vault.put(row(2, None)).expect("put base");
    append_n(&vault, 2, 10, QUERY_TIME - 1_800);

    let evidence =
        recurrence_boost_evidence(cx(2), &vault, QUERY_TIME, &Default::default()).expect("score");
    let recency = (-1_800.0_f32 * 0.693 / 3_600.0).exp();
    let expected = frequency_kernel_bonus(10) * 0.05 + recency * 0.05;

    assert_close(evidence.total, expected);
    assert_eq!(evidence.frequency, 10);
    assert_eq!(evidence.last_occurrence_t, Some(QUERY_TIME - 1_800));
}

#[test]
fn max_recurrence_boost_caps_high_frequency_recent_hit() {
    let vault = vault();
    vault.put(row(3, Some(10_000.0))).expect("put base");
    append_occurrence_at(&vault, 3, QUERY_TIME);

    let score =
        recurrence_boost_score(cx(3), &vault, QUERY_TIME, &Default::default()).expect("score");

    assert_close(score, 0.10);
}

#[test]
fn zero_content_score_remains_zero_after_recurrence_boost() {
    let vault = vault();
    vault.put(row(4, Some(10_000.0))).expect("put base");
    append_occurrence_at(&vault, 4, QUERY_TIME);
    let hit = hit(4, 0.0, 1).with_explain("recurrence-test");

    let boosted = apply_temporal_boost_with_recurrence(
        vec![hit],
        &TemporalPolicy::default(),
        QUERY_TIME,
        0,
        &vault,
    )
    .expect("boost");

    assert_eq!(boosted[0].score, 0.0);
    assert_eq!(boosted[0].temporal_scores, Some(TemporalScores::zero()));
    assert!(
        boosted[0]
            .explain
            .as_ref()
            .and_then(|explain| explain.recurrence_boost)
            .is_some_and(|evidence| evidence.total > 0.0)
    );
}

#[test]
fn disabled_recurrence_config_does_not_read_base_cf() {
    let vault = vault();
    let policy = TemporalPolicy {
        recurrence_boost: None,
        ..TemporalPolicy::default()
    };
    let hits = vec![hit(5, 0.70, 1)];

    let with_vault =
        apply_temporal_boost_with_recurrence(hits.clone(), &policy, QUERY_TIME, 0, &vault)
            .expect("disabled recurrence");
    let without_vault = apply_temporal_boost(hits, &policy, QUERY_TIME, 0).expect("plain boost");

    assert_eq!(with_vault, without_vault);
}

#[test]
fn u64_max_frequency_caps_frequency_component_without_overflow() {
    let vault = vault();
    vault.put(row(6, Some(u64::MAX as f64))).expect("put base");
    let config = RecurrenceBoostConfig::default();

    let evidence = recurrence_boost_evidence(cx(6), &vault, QUERY_TIME, &config).expect("score");

    assert_eq!(evidence.frequency, u64::MAX);
    assert_close(evidence.frequency_bonus, 1.0);
    assert_close(evidence.frequency_component, config.frequency_weight);
}

#[test]
fn missing_base_cf_fails_closed_with_sextant_code() {
    let vault = vault();

    let error = recurrence_boost_score(cx(7), &vault, QUERY_TIME, &Default::default())
        .expect_err("missing base row");

    assert_eq!(error.code, CALYX_SEXTANT_RECURRENCE_READ_ERROR);
}

fn append_n(vault: &AsterVault, seed: u8, count: usize, last_time: i64) {
    for idx in 0..count {
        append_occurrence(
            vault,
            cx(seed),
            EpochSecs(last_time - (count - idx - 1) as i64),
            OccurrenceContext::new(format!("{seed}-{idx}")).expect("context"),
            EpochSecs(QUERY_TIME),
            RetentionPolicy::default(),
        )
        .expect("append recurrence");
    }
}

fn append_occurrence_at(vault: &AsterVault, seed: u8, time: i64) {
    append_occurrence(
        vault,
        cx(seed),
        EpochSecs(time),
        OccurrenceContext::new(format!("{seed}-recent")).expect("context"),
        EpochSecs(time),
        RetentionPolicy::default(),
    )
    .expect("append recurrence");
}

fn hit(seed: u8, score: f32, rank: usize) -> Hit {
    Hit {
        cx_id: cx(seed),
        score,
        rank,
        event_time_secs: Some(QUERY_TIME - 60),
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
            pointer: Some(format!("zfs://calyx/sextant-recurrence/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::<SlotId, SlotVector>::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("sextant-recurrence".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "calyx-sextant-test".to_string(),
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

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"sextant-recurrence")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "actual {actual} expected {expected}"
    );
}
