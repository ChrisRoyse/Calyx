use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AnnealLedger, AnnealLedgerAction, CALYX_STORAGE_CACHE_WRITE_FAIL,
    CALYX_STORAGE_SCOPE_INVALID_CONFIG, ChangeId, NoopStorageBanditStore, StorageConfig,
    StorageMetrics, StoragePromotionRecord, StoragePromotionWriter, StorageScopeTuner,
    StorageShapeKey, candidate_storage_configs, shape_key_hash, storage_autotune_key,
    storage_win_check, validate_storage_config,
};
use calyx_core::{FixedClock, Result};
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, LedgerAppender, MemoryLedgerStore};
use serde_json::json;

#[test]
fn lower_latency_without_storage_regressions_promotes_and_persists() {
    let key = storage_key("promotes");
    let path = temp_path("promotes");
    let cache = AutotuneCache::load(&path).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = StorageScopeTuner::with_parts(cache, writer.clone(), NoopStorageBanditStore);
    let configs = tuned_configs();
    let winner = configs[1].clone();
    tuner.install_candidates(key.clone(), configs).unwrap();
    tuner
        .on_observation_for_arm(key.clone(), 0, baseline_metrics())
        .unwrap();

    for idx in 0..3 {
        let mut candidate = baseline_metrics();
        candidate.p99_read_ns = 800 - idx * 10;
        candidate.cache_miss_milli = 90;
        candidate.tier_hot_hit_milli = 930;
        candidate.codebook_staleness_secs = 300;
        candidate.prefetch_hit_milli = 840;
        let decision = tuner
            .on_observation_for_arm(key.clone(), 1, candidate)
            .unwrap();
        if idx < 2 {
            assert!(decision.promoted.is_none());
        } else {
            assert_eq!(decision.incumbent, winner);
            assert!(decision.promoted.is_some());
        }
    }

    assert_eq!(writer.records().len(), 1);
    let loaded = AutotuneCache::load(&path).unwrap();
    let persisted = loaded
        .get(&storage_autotune_key(
            &key,
            calyx_anneal::DEFAULT_STORAGE_RECALL_TARGET,
        ))
        .unwrap();
    assert_eq!(
        persisted.extra.get("compaction_interval_ms").unwrap(),
        "5000"
    );
    assert_eq!(persisted.extra.get("scope").unwrap(), "storage");
}

#[test]
fn write_amp_or_tier_regression_does_not_promote() {
    let key = storage_key("write_amp_regression");
    let cache = AutotuneCache::load(&temp_path("write_amp_regression")).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = StorageScopeTuner::with_parts(cache, writer.clone(), NoopStorageBanditStore);
    let configs = tuned_configs();
    let incumbent = configs[0].clone();
    tuner.install_candidates(key.clone(), configs).unwrap();
    tuner
        .on_observation_for_arm(key.clone(), 0, baseline_metrics())
        .unwrap();

    let mut regressed = baseline_metrics();
    regressed.p99_read_ns = 700;
    regressed.write_amp_milli = baseline_metrics().write_amp_milli + 1;
    assert!(!storage_win_check(&baseline_metrics(), &regressed));
    for _ in 0..3 {
        let decision = tuner
            .on_observation_for_arm(key.clone(), 1, regressed)
            .unwrap();
        assert!(decision.promoted.is_none());
    }

    assert_eq!(tuner.get_incumbent_config(&key).unwrap(), incumbent);
    assert!(writer.records().is_empty());
}

#[test]
fn invalid_storage_config_fails_closed() {
    let config = StorageConfig {
        prefetch_bytes: 123,
        ..StorageConfig::default()
    };
    let error = validate_storage_config(&config).unwrap_err();

    assert_eq!(error.code, CALYX_STORAGE_SCOPE_INVALID_CONFIG);
    assert!(error.message.contains("4096-byte multiple"));
}

#[test]
fn scheduler_options_mirror_compaction_knobs() {
    let config = StorageConfig {
        compaction_interval_ms: 5_000,
        debt_trigger_score_milli: 750,
        max_write_amp_milli: 1_500,
        ..StorageConfig::default()
    };
    let options = config
        .to_scheduler_options("/tmp/calyx-storage-autotune")
        .unwrap();

    assert_eq!(options.interval_ms, 5_000);
    assert_eq!(options.debt_trigger_score_milli, 750);
    assert_eq!(options.max_write_amp_milli, 1_500);
}

#[test]
fn cache_write_failure_is_fail_closed_but_incumbent_updates_in_memory() {
    let key = storage_key("missing_parent");
    let cache_path = temp_path("missing_parent").join("cache.json");
    let cache = AutotuneCache::load(&cache_path).unwrap();
    let mut tuner = StorageScopeTuner::new(cache);
    let configs = tuned_configs();
    let winner = configs[1].clone();
    tuner.install_candidates(key.clone(), configs).unwrap();
    tuner
        .on_observation_for_arm(key.clone(), 0, baseline_metrics())
        .unwrap();

    let candidate = winning_metrics();
    tuner
        .on_observation_for_arm(key.clone(), 1, candidate)
        .unwrap();
    tuner
        .on_observation_for_arm(key.clone(), 1, candidate)
        .unwrap();
    let error = tuner
        .on_observation_for_arm(key.clone(), 1, candidate)
        .unwrap_err();

    assert_eq!(error.code, CALYX_STORAGE_CACHE_WRITE_FAIL);
    assert_eq!(tuner.get_incumbent_config(&key).unwrap(), winner);
}

#[test]
fn generated_candidates_cover_all_storage_knobs() {
    let configs = candidate_storage_configs(&storage_key("candidate_coverage")).unwrap();

    assert_eq!(configs.len(), 8);
    assert!(
        configs
            .iter()
            .any(|cfg| cfg.compaction_interval_ms != 10_000)
    );
    assert!(configs.iter().any(|cfg| cfg.hot_tier_min_hits != 8));
    assert!(configs.iter().any(|cfg| cfg.codebook_refresh_secs != 3_600));
    assert!(configs.iter().any(|cfg| cfg.prefetch_bytes != 64 * 1024));
}

#[test]
fn ledger_promotion_payload_uses_redaction_safe_storage_artifact() {
    let key = StorageShapeKey::new("v".repeat(48), "w".repeat(48), &[257, 65, 17]);
    let record = StoragePromotionRecord {
        key: key.clone(),
        change_id: ChangeId(58_300),
        old_config: tuned_configs()[0].clone(),
        new_config: tuned_configs()[1].clone(),
        metrics_before: baseline_metrics(),
        metrics_after: winning_metrics(),
        key_hash: shape_key_hash(&key.label()),
        old_config_hash: [1; 32],
        new_config_hash: [2; 32],
    };
    let mut ledger = memory_ledger();

    ledger.write_autotune_promote(&record).unwrap();
    let entries = ledger.read_recent(1).unwrap();
    let entry = entries.first().unwrap();
    let details = entry.details.as_ref().unwrap();

    assert_eq!(entry.action, AnnealLedgerAction::AutotunePromote);
    assert!(entry.artifact_id.starts_with("storage:"));
    assert!(entry.artifact_id.len() < 40);
    assert_ne!(entry.artifact_id, key.label());
    assert_eq!(
        details.get("tag"),
        Some(&json!("storage_autotune_promotion_v1"))
    );
    assert_eq!(details.get("scope"), Some(&json!("storage")));
    assert!(details.get("shape_key").is_none());
    assert!(details.get("vault_id").is_none());
    assert!(details.get("workload_id").is_none());
    assert_eq!(details.get("shape_bucketed"), Some(&json!([512, 128, 32])));
}

#[derive(Clone, Default)]
struct RecordingWriter {
    records: Arc<Mutex<Vec<StoragePromotionRecord>>>,
}

impl RecordingWriter {
    fn records(&self) -> Vec<StoragePromotionRecord> {
        self.records.lock().unwrap().clone()
    }
}

impl StoragePromotionWriter for RecordingWriter {
    fn write_autotune_promote(&mut self, event: &StoragePromotionRecord) -> Result<()> {
        self.records.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn tuned_configs() -> Vec<StorageConfig> {
    vec![
        StorageConfig::default(),
        StorageConfig {
            compaction_interval_ms: 5_000,
            debt_trigger_score_milli: 750,
            prefetch_bytes: 128 * 1024,
            ..StorageConfig::default()
        },
    ]
}

fn baseline_metrics() -> StorageMetrics {
    StorageMetrics {
        p99_read_ns: 1_000,
        write_amp_milli: 1_500,
        cache_miss_milli: 100,
        tier_hot_hit_milli: 900,
        codebook_staleness_secs: 600,
        prefetch_hit_milli: 800,
    }
}

fn winning_metrics() -> StorageMetrics {
    StorageMetrics {
        p99_read_ns: 780,
        write_amp_milli: 1_500,
        cache_miss_milli: 90,
        tier_hot_hit_milli: 930,
        codebook_staleness_secs: 300,
        prefetch_hit_milli: 840,
    }
}

fn storage_key(label: &str) -> StorageShapeKey {
    StorageShapeKey::new("issue583-vault", label, &[257, 65, 17])
}

fn temp_path(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "calyx_scope_storage_{label}_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_file(&path);
    path
}

fn memory_ledger() -> AnnealLedger<MemoryLedgerStore, FixedClock> {
    let appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(58_300)).unwrap();
    AnnealLedger::new(
        appender,
        ActorId::Service("calyx-storage-scope-test".to_string()),
    )
    .unwrap()
}
