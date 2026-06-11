use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_FORGE_CACHE_WRITE_FAIL, DType, ForgeConfig, ForgePromotionRecord, ForgePromotionWriter,
    ForgeScopeTuner, MAX_BUCKETED_DIM, NoopForgeBanditStore, ShapeKey, bucket_dim,
};
use calyx_core::Result;
use calyx_forge::{AutotuneCache, autotune};
use proptest::prelude::*;

#[test]
fn faster_same_recall_promotes_after_hysteresis() {
    let key = forge_key();
    let path = temp_path("promotes");
    let cache = AutotuneCache::load(&path).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = ForgeScopeTuner::with_parts(cache, writer.clone(), NoopForgeBanditStore);
    let configs = two_configs(&key);
    let winner = configs[1].clone();
    tuner.install_candidates(key.clone(), configs).unwrap();

    tuner.on_op_for_arm(key.clone(), 0, 1_000, 0.99).unwrap();
    assert!(
        tuner
            .on_op_for_arm(key.clone(), 1, 800, 0.99)
            .unwrap()
            .promoted
            .is_none()
    );
    assert!(
        tuner
            .on_op_for_arm(key.clone(), 1, 790, 0.99)
            .unwrap()
            .promoted
            .is_none()
    );
    let decision = tuner.on_op_for_arm(key.clone(), 1, 780, 0.99).unwrap();

    assert_eq!(decision.incumbent, winner);
    assert_eq!(tuner.get_incumbent(&key).unwrap(), winner);
    assert_eq!(writer.records().len(), 1);
    let loaded = AutotuneCache::load(&path).unwrap();
    assert_eq!(
        autotune(&loaded, &key.autotune_key(0.99)).tile_m,
        winner.tile_m as usize
    );
}

#[test]
fn lower_recall_candidate_does_not_promote() {
    let key = forge_key();
    let cache = AutotuneCache::load(&temp_path("recall_reject")).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = ForgeScopeTuner::with_parts(cache, writer.clone(), NoopForgeBanditStore);
    let configs = two_configs(&key);
    let incumbent = configs[0].clone();
    tuner.install_candidates(key.clone(), configs).unwrap();
    tuner.on_op_for_arm(key.clone(), 0, 1_000, 0.99).unwrap();

    for _ in 0..3 {
        let decision = tuner.on_op_for_arm(key.clone(), 1, 700, 0.98).unwrap();
        assert!(decision.promoted.is_none());
    }

    assert_eq!(tuner.get_incumbent(&key).unwrap(), incumbent);
    assert!(writer.records().is_empty());
}

#[test]
fn shape_bucketing_caps_large_dimensions_without_overflow() {
    assert_eq!(bucket_dim(0), 1);
    assert_eq!(bucket_dim(769), 1024);
    assert_eq!(bucket_dim(MAX_BUCKETED_DIM - 1), MAX_BUCKETED_DIM);
    assert_eq!(bucket_dim(MAX_BUCKETED_DIM), MAX_BUCKETED_DIM);
    assert_eq!(bucket_dim(u32::MAX), MAX_BUCKETED_DIM);

    let key = ShapeKey::new("gemm", &[u32::MAX, 1], DType::Fp16, "cuda");
    assert_eq!(key.shape_bucketed, vec![MAX_BUCKETED_DIM, 1]);
}

#[test]
fn first_on_op_creates_bandit_and_returns_default() {
    let key = forge_key();
    let cache = AutotuneCache::load(&temp_path("first")).unwrap();
    let mut tuner = ForgeScopeTuner::new(cache);

    let decision = tuner.on_op(key.clone(), 1_000, 0.99).unwrap();

    assert_eq!(decision.incumbent, ForgeConfig::default_for(&key));
    assert!(decision.promoted.is_none());
    assert!(!tuner.bandits[&key].arms.is_empty());
}

#[test]
fn cache_write_failure_is_fail_closed_but_bandit_state_survives() {
    let key = forge_key();
    let cache_path = temp_path("missing_parent").join("cache.json");
    let cache = AutotuneCache::load(&cache_path).unwrap();
    let writer = RecordingWriter::default();
    let mut tuner = ForgeScopeTuner::with_parts(cache, writer.clone(), NoopForgeBanditStore);
    let configs = two_configs(&key);
    let winner = configs[1].clone();
    tuner.install_candidates(key.clone(), configs).unwrap();
    tuner.on_op_for_arm(key.clone(), 0, 1_000, 0.99).unwrap();
    tuner.on_op_for_arm(key.clone(), 1, 800, 0.99).unwrap();
    tuner.on_op_for_arm(key.clone(), 1, 790, 0.99).unwrap();

    let err = tuner.on_op_for_arm(key.clone(), 1, 780, 0.99).unwrap_err();

    assert_eq!(err.code, CALYX_FORGE_CACHE_WRITE_FAIL);
    assert_eq!(tuner.get_incumbent(&key).unwrap(), winner);
    assert!(writer.records().is_empty());
}

proptest! {
    #[test]
    fn incumbent_is_always_a_valid_config(latencies in prop::collection::vec(1_u64..10_000, 1..40)) {
        let key = forge_key();
        let cache = AutotuneCache::load(&temp_path("proptest")).unwrap();
        let mut tuner = ForgeScopeTuner::new(cache);
        let configs = two_configs(&key);
        tuner.install_candidates(key.clone(), configs.clone()).unwrap();
        for (idx, latency) in latencies.into_iter().enumerate() {
            let arm = idx % configs.len();
            let _ = tuner.on_op_for_arm(key.clone(), arm, latency, 0.99);
            prop_assert!(configs.contains(&tuner.get_incumbent(&key).unwrap()));
        }
    }
}

#[derive(Clone, Default)]
struct RecordingWriter {
    records: Arc<Mutex<Vec<ForgePromotionRecord>>>,
}

impl RecordingWriter {
    fn records(&self) -> Vec<ForgePromotionRecord> {
        self.records.lock().unwrap().clone()
    }
}

impl ForgePromotionWriter for RecordingWriter {
    fn write_autotune_promote(&mut self, event: &ForgePromotionRecord) -> Result<()> {
        self.records.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn forge_key() -> ShapeKey {
    ShapeKey::new("gemm", &[768, 768], DType::Fp16, "cuda")
}

fn two_configs(key: &ShapeKey) -> Vec<ForgeConfig> {
    vec![
        ForgeConfig::default_for(key),
        ForgeConfig {
            tile_m: 128,
            tile_n: 128,
            tile_k: 64,
            dtype: DType::Fp16,
            batch_size: 2,
        },
    ]
}

fn temp_path(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "calyx_scope_forge_{label}_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_file(&path);
    path
}
