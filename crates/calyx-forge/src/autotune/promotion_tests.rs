use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::FixedClock;
use proptest::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use super::{
    AbHook, AutotuneCache, AutotuneKey, PromotionAction, PromotionEvent, autotune, log_promotion,
    rollback_promotion, should_use_challenger,
};
use crate::{BackendKind, BestConfig, ForgeError, Result};

fn key(op: &str) -> AutotuneKey {
    AutotuneKey::default_for(op, &[1024, 1024, 1024], "f32", "cuda:0")
}

fn other_key() -> AutotuneKey {
    AutotuneKey::default_for("cosine", &[1024, 1024], "f32", "cuda:0")
}

fn config(tile: usize) -> BestConfig {
    BestConfig {
        backend: BackendKind::Cuda,
        tile_m: tile,
        tile_n: tile,
        tile_k: 32,
        extra: HashMap::from([("tile".to_string(), tile.to_string())]),
    }
}

fn promotion_event(
    key: AutotuneKey,
    old_config: BestConfig,
    new_config: BestConfig,
    timestamp_ns: u64,
) -> PromotionEvent {
    PromotionEvent {
        key,
        old_config,
        new_config,
        timestamp_ns,
        action: PromotionAction::Promoted,
    }
}

fn unique_path(name: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx_promotion_{}_{}_{}.{}",
        name,
        std::process::id(),
        nanos,
        ext
    ))
}

fn fsv_log_path() -> PathBuf {
    std::env::temp_dir().join("calyx_promotion_test.jsonl")
}

fn read_events(path: &Path) -> Vec<PromotionEvent> {
    fs::read_to_string(path)
        .expect("read promotion log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("deserialize promotion event"))
        .collect()
}

fn fsv_error(op: &str, path: &Path, detail: impl ToString) -> ForgeError {
    ForgeError::CacheError {
        op: op.to_string(),
        path: path.display().to_string(),
        detail: detail.to_string(),
        remediation: "repair CALYX_FSV_ROOT and rerun promotion provenance readback".to_string(),
    }
}

fn write_promotion_fsv_readbacks(
    log_path: &Path,
    events: &[PromotionEvent],
    demoted: Option<&BestConfig>,
    cache_config: Option<&BestConfig>,
) -> Result<()> {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root).map_err(|err| fsv_error("fsv_mkdir", &root, err))?;

    let log_dest = root.join("promotion-log-readback.jsonl");
    let raw_log = fs::read(log_path).map_err(|err| fsv_error("fsv_read", log_path, err))?;
    fs::write(&log_dest, &raw_log).map_err(|err| fsv_error("fsv_write", &log_dest, err))?;
    let copied_log =
        fs::read_to_string(&log_dest).map_err(|err| fsv_error("fsv_read", &log_dest, err))?;
    assert_eq!(copied_log.lines().count(), events.len());

    let summary = serde_json::json!({
        "issue": 338,
        "case": "promotion_logged_and_reversible",
        "provenance_surface": "local_jsonl_audit_stub",
        "ledger_chain_entry": false,
        "event_count": events.len(),
        "actions": events.iter().map(|event| format!("{:?}", event.action)).collect::<Vec<_>>(),
        "demoted_tile": demoted.map(|cfg| cfg.tile_m),
        "cache_tile": cache_config.map(|cfg| cfg.tile_m)
    });
    let summary_dest = root.join("promotion-provenance-summary-readback.json");
    let bytes = serde_json::to_vec_pretty(&summary)
        .map_err(|err| fsv_error("fsv_serialize", &summary_dest, err))?;
    fs::write(&summary_dest, &bytes).map_err(|err| fsv_error("fsv_write", &summary_dest, err))?;
    let readback =
        fs::read(&summary_dest).map_err(|err| fsv_error("fsv_read", &summary_dest, err))?;
    assert_eq!(readback, bytes);
    println!(
        "FORGE_PROMOTION_READBACK log_path={} log_bytes={} summary_path={} summary_bytes={}",
        log_dest.display(),
        raw_log.len(),
        summary_dest.display(),
        readback.len()
    );
    Ok(())
}

#[test]
fn promotion_logged_and_reversible() -> Result<()> {
    let log_path = fsv_log_path();
    let _ = fs::remove_file(&log_path);
    let cache_path = unique_path("cache", "json");
    let mut cache = AutotuneCache::load(&cache_path)?;
    let key = key("gemm");
    let old_config = config(64);
    let new_config = config(128);
    let event = promotion_event(key.clone(), old_config.clone(), new_config.clone(), 111);

    cache.insert(key.clone(), new_config.clone());
    log_promotion(&event, &log_path)?;
    let first_events = read_events(&log_path);

    assert_eq!(first_events, vec![event]);

    let demoted = rollback_promotion(&mut cache, &log_path, &key, &FixedClock::new(2_000))?;
    let events = read_events(&log_path);

    assert_eq!(demoted, Some(new_config.clone()));
    assert_eq!(cache.get(&key), Some(&old_config));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].action, PromotionAction::Promoted);
    assert_eq!(events[1].action, PromotionAction::RolledBack);
    assert_eq!(events[1].old_config, new_config);
    assert_eq!(events[1].new_config, old_config);
    assert_eq!(events[1].timestamp_ns, 2_000_000_000);
    write_promotion_fsv_readbacks(&log_path, &events, demoted.as_ref(), cache.get(&key))?;
    println!(
        "promotion_logged_and_reversible PASSED Promoted old_tile=64 new_tile=128 RolledBack demoted_tile={} cache_tile={} log_path={}",
        demoted.as_ref().map_or(0, |cfg| cfg.tile_m),
        cache.get(&key).map_or(0, |cfg| cfg.tile_m),
        log_path.display()
    );
    Ok(())
}

#[test]
fn autotune_absent_returns_default_and_cached_returns_entry() -> Result<()> {
    let path = unique_path("autotune_default", "json");
    let mut cache = AutotuneCache::load(&path)?;
    let key = key("gemm");
    let default = autotune(&cache, &key);
    let expected_backend = if cfg!(feature = "cuda") {
        BackendKind::Cuda
    } else {
        BackendKind::Cpu
    };

    assert_eq!(default.backend, expected_backend);
    assert_eq!(default.tile_m, 64);
    assert_eq!(default.tile_k, 32);

    let cached = config(192);
    cache.insert(key.clone(), cached.clone());
    assert_eq!(autotune(&cache, &key), cached);
    println!(
        "autotune_absent_default PASSED backend={} default_tile={} cached_tile=192",
        default.backend, default.tile_m
    );
    Ok(())
}

#[test]
fn ab_hook_rate_prints_seeded_fraction() {
    let hook = AbHook { rate: 0.1 };
    let calls = 1_000;
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let uses = (0..calls)
        .filter(|_| should_use_challenger(&hook, &mut rng))
        .count();
    let fraction = uses as f64 / calls as f64;

    assert!((0.08..=0.12).contains(&fraction));
    println!("ab_hook_rate PASSED challenger_fraction={fraction:.3} rate=0.1");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn ab_hook_rate_seeded_proptest(seed in 0u64..8) {
        let hook = AbHook { rate: 0.1 };
        let calls = 10_000;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let uses = (0..calls)
            .filter(|_| should_use_challenger(&hook, &mut rng))
            .count();
        let fraction = uses as f64 / calls as f64;

        prop_assert!((0.08..=0.12).contains(&fraction));
    }
}

#[test]
fn rollback_without_prior_promotion_returns_none() -> Result<()> {
    let log_path = unique_path("empty_rollback", "jsonl");
    let cache_path = unique_path("empty_rollback_cache", "json");
    let mut cache = AutotuneCache::load(&cache_path)?;
    let key = key("gemm");
    let current = config(128);

    cache.insert(key.clone(), current.clone());
    let demoted = rollback_promotion(&mut cache, &log_path, &key, &FixedClock::new(3_000))?;

    assert_eq!(demoted, None);
    assert_eq!(cache.get(&key), Some(&current));
    println!(
        "promotion_no_prior PASSED RolledBack=false cache_tile={} log_exists={}",
        cache.get(&key).map_or(0, |cfg| cfg.tile_m),
        log_path.exists()
    );
    Ok(())
}

#[test]
fn log_promotion_missing_directory_fails_closed() {
    let dir = unique_path("missing_dir", "dir");
    let log_path = dir.join("promotion.jsonl");
    let event = promotion_event(key("gemm"), config(64), config(128), 111);

    let err = log_promotion(&event, &log_path).expect_err("missing parent must fail closed");

    assert!(matches!(err, ForgeError::CacheError { .. }));
    assert!(err.to_string().contains(&log_path.display().to_string()));
    println!("promotion_missing_dir PASSED {err}");
}

#[test]
fn rollback_uses_most_recent_promotion() -> Result<()> {
    let log_path = unique_path("two_promotions", "jsonl");
    let cache_path = unique_path("two_promotions_cache", "json");
    let mut cache = AutotuneCache::load(&cache_path)?;
    let key = key("gemm");
    let first = config(64);
    let second = config(128);
    let third = config(256);

    log_promotion(
        &promotion_event(key.clone(), first, second.clone(), 111),
        &log_path,
    )?;
    log_promotion(
        &promotion_event(key.clone(), second.clone(), third.clone(), 222),
        &log_path,
    )?;
    cache.insert(key.clone(), third.clone());

    let demoted = rollback_promotion(&mut cache, &log_path, &key, &FixedClock::new(4_000))?;
    let events = read_events(&log_path);

    assert_eq!(demoted, Some(third));
    assert_eq!(cache.get(&key), Some(&second));
    assert_eq!(
        events.last().map(|event| event.action),
        Some(PromotionAction::RolledBack)
    );
    println!(
        "promotion_two_promotions PASSED RolledBack=true demoted_tile=256 cache_tile={}",
        cache.get(&key).map_or(0, |cfg| cfg.tile_m)
    );
    Ok(())
}

#[test]
fn rollback_malformed_jsonl_fails_closed() {
    let log_path = unique_path("malformed", "jsonl");
    let valid = promotion_event(other_key(), config(64), config(128), 111);
    let valid_json = serde_json::to_string(&valid).expect("serialize valid event");
    fs::write(&log_path, format!("{valid_json}\n{{bad-json\n")).expect("write malformed log");
    let cache_path = unique_path("malformed_cache", "json");
    let mut cache = AutotuneCache::load(&cache_path).expect("load cache");

    let err = rollback_promotion(&mut cache, &log_path, &key("gemm"), &FixedClock::new(5_000))
        .expect_err("malformed JSONL must fail closed");

    assert!(matches!(err, ForgeError::CacheError { .. }));
    assert!(err.to_string().contains("line 2"));
    assert!(err.to_string().contains("{bad-json"));
    println!("promotion_malformed_jsonl PASSED {err}");
}
