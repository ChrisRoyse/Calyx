use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_INDEX_CACHE_WRITE_FAIL, IndexConfig, IndexPromotionRecord, IndexPromotionWriter,
    IndexScopeTuner, IndexSlotHealth, IndexTuneSkip, NoopIndexBanditStore, NoopIndexSlotHealth,
    QuantPromotionEvidence, index_candidate_configs, quant_win_check, slot_autotune_key,
};
use calyx_core::{Result, SlotId};
use calyx_forge::AutotuneCache;
use proptest::prelude::*;

#[test]
fn higher_p99_candidate_does_not_promote_even_with_better_recall() {
    let slot = SlotId::new(0);
    let cache = AutotuneCache::load(&temp_path("higher_p99")).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = IndexScopeTuner::with_parts(
        cache,
        writer.clone(),
        NoopIndexBanditStore,
        NoopIndexSlotHealth,
    );
    let configs = latency_configs();
    let incumbent = configs[0].clone();
    tuner.install_candidates(slot, configs).unwrap();
    tuner.on_search_for_arm(slot, 0, 700, 0.980, 12.0).unwrap();

    for _ in 0..3 {
        let decision = tuner.on_search_for_arm(slot, 1, 900, 0.995, 12.0).unwrap();
        assert!(decision.promoted.is_none());
    }

    assert_eq!(tuner.get_incumbent_config(slot).unwrap(), incumbent);
    assert!(writer.records().is_empty());
}

#[test]
fn quant_downgrade_with_bits_loss_is_rejected() {
    let slot = SlotId::new(0);
    let cache = AutotuneCache::load(&temp_path("bits_loss")).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = IndexScopeTuner::with_parts(
        cache,
        writer.clone(),
        NoopIndexBanditStore,
        NoopIndexSlotHealth,
    );
    let configs = quant_configs();
    let incumbent = configs[0].clone();
    assert!(!quant_win_check(&configs[1], &configs[0], 10.0, 9.5));
    tuner.install_candidates(slot, configs).unwrap();
    tuner
        .on_search_for_arm(slot, 0, 1_000, 0.990, 10.0)
        .unwrap();

    for _ in 0..3 {
        let decision = tuner.on_search_for_arm(slot, 1, 700, 0.990, 9.5).unwrap();
        assert!(decision.promoted.is_none());
    }

    assert_eq!(tuner.get_incumbent_config(slot).unwrap(), incumbent);
    assert!(writer.records().is_empty());
}

#[test]
fn quant_downgrade_with_equal_bits_and_lower_p99_promotes() {
    let slot = SlotId::new(0);
    let path = temp_path("promotes");
    let cache = AutotuneCache::load(&path).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = IndexScopeTuner::with_parts(
        cache,
        writer.clone(),
        NoopIndexBanditStore,
        NoopIndexSlotHealth,
    );
    let configs = quant_configs();
    let winner = configs[1].clone();
    assert!(quant_win_check(&winner, &configs[0], 10.0, 9.999_999_5));
    tuner.install_candidates(slot, configs).unwrap();
    tuner
        .on_search_for_arm(slot, 0, 1_000, 0.990, 10.0)
        .unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(
            slot,
            1,
            800,
            0.990,
            9.999_999_5,
            Some(quant_evidence()),
        )
        .unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(
            slot,
            1,
            790,
            0.990,
            9.999_999_5,
            Some(quant_evidence()),
        )
        .unwrap();
    let decision = tuner
        .on_search_for_arm_with_quant_evidence(
            slot,
            1,
            780,
            0.990,
            9.999_999_5,
            Some(quant_evidence()),
        )
        .unwrap();

    assert_eq!(decision.incumbent, winner);
    assert_eq!(writer.records().len(), 1);
    assert_eq!(writer.records()[0].quant_evidence, Some(quant_evidence()));
    let loaded = AutotuneCache::load(&path).unwrap();
    let persisted = loaded.get(&slot_autotune_key(slot, 0.99)).unwrap();
    assert_eq!(persisted.extra.get("quant_bits").unwrap(), "8");
}

#[test]
fn quant_downgrade_missing_evidence_fails_closed() {
    let slot = SlotId::new(0);
    let cache = AutotuneCache::load(&temp_path("missing_evidence")).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = IndexScopeTuner::with_parts(
        cache,
        writer.clone(),
        NoopIndexBanditStore,
        NoopIndexSlotHealth,
    );
    let configs = quant_configs();
    let incumbent = configs[0].clone();
    tuner.install_candidates(slot, configs).unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(slot, 0, 1_000, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 800, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 790, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();

    let error = tuner
        .on_search_for_arm(slot, 1, 780, 0.990, 10.0)
        .unwrap_err();

    assert_eq!(error.code, calyx_anneal::CALYX_INDEX_SCOPE_INVALID_CONFIG);
    assert!(error.message.contains("requires measured cosine/FAR"));
    assert_eq!(tuner.get_incumbent_config(slot).unwrap(), incumbent);
    assert!(writer.records().is_empty());
}

#[test]
fn parked_slot_is_noop() {
    let slot = SlotId::new(0);
    let cache = AutotuneCache::load(&temp_path("parked")).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner =
        IndexScopeTuner::with_parts(cache, writer.clone(), NoopIndexBanditStore, Parked);

    let decision = tuner.on_search(slot, 700, 0.99, 0.01).unwrap();

    assert_eq!(decision.skipped, Some(IndexTuneSkip::ParkedSlot));
    assert!(decision.promoted.is_none());
    assert!(decision.shadow_arm.is_none());
    assert!(tuner.bandits.is_empty());
    assert!(writer.records().is_empty());
}

#[test]
fn cache_write_failure_is_fail_closed_but_in_memory_incumbent_survives() {
    let slot = SlotId::new(0);
    let cache_path = temp_path("missing_parent").join("cache.json");
    let cache = AutotuneCache::load(&cache_path).unwrap();
    let mut tuner = IndexScopeTuner::new(cache);
    let configs = quant_configs();
    let winner = configs[1].clone();
    tuner.install_candidates(slot, configs).unwrap();
    tuner
        .on_search_for_arm(slot, 0, 1_000, 0.990, 10.0)
        .unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 800, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();
    tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 790, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();

    let error = tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 780, 0.990, 10.0, Some(quant_evidence()))
        .unwrap_err();

    assert_eq!(error.code, CALYX_INDEX_CACHE_WRITE_FAIL);
    assert_eq!(tuner.get_incumbent_config(slot).unwrap(), winner);
}

proptest! {
    #[test]
    fn incumbent_quant_bits_are_valid(arms in prop::collection::vec(0_usize..8, 1..40)) {
        let slot = SlotId::new(0);
        let cache = AutotuneCache::load(&temp_path("proptest")).unwrap();
        let mut tuner = IndexScopeTuner::new(cache);
        let configs = index_candidate_configs(slot).unwrap();
        tuner.install_candidates(slot, configs).unwrap();
        for arm in arms {
            let _ = tuner.on_search_for_arm(slot, arm % 8, 1_000 - (arm as u64 * 10), 0.99, 10.0);
            let incumbent = tuner.get_incumbent_config(slot).unwrap();
            prop_assert!([4, 8, 16, 32].contains(&incumbent.quant_bits));
        }
    }
}

#[derive(Clone, Default)]
struct RecordingWriter {
    records: Arc<Mutex<Vec<IndexPromotionRecord>>>,
}

impl RecordingWriter {
    fn records(&self) -> Vec<IndexPromotionRecord> {
        self.records.lock().unwrap().clone()
    }
}

impl IndexPromotionWriter for RecordingWriter {
    fn write_autotune_promote(&mut self, event: &IndexPromotionRecord) -> Result<()> {
        self.records.lock().unwrap().push(event.clone());
        Ok(())
    }
}

struct Parked;

impl IndexSlotHealth for Parked {
    fn is_slot_parked(&self, _slot_id: SlotId) -> bool {
        true
    }
}

fn latency_configs() -> Vec<IndexConfig> {
    vec![
        IndexConfig {
            hnsw_ef: 128,
            ..IndexConfig::default()
        },
        IndexConfig {
            hnsw_ef: 256,
            ..IndexConfig::default()
        },
    ]
}

fn quant_configs() -> Vec<IndexConfig> {
    vec![
        IndexConfig::default(),
        IndexConfig {
            hnsw_ef: 128,
            quant_bits: 8,
            ..IndexConfig::default()
        },
    ]
}

fn quant_evidence() -> QuantPromotionEvidence {
    QuantPromotionEvidence {
        cosine_error_before: 0.0,
        cosine_error_after: 0.000_4,
        max_cosine_error: 0.001,
        guard_far_before: 0.001,
        guard_far_after: 0.001,
    }
}

fn temp_path(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "calyx_scope_index_{label}_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_file(&path);
    path
}
