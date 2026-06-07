use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_forge::{AutotuneCache, AutotuneKey, BackendKind, BestConfig, Result, autotune};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[cfg(feature = "cuda")]
use calyx_core::FixedClock;
#[cfg(feature = "cuda")]
use calyx_forge::{
    BenchCudaContext, BenchResult, Explorer, ExplorerPolicy, PromotionAction, PromotionEvent,
    init_cuda, log_promotion, microbench, next_candidate, promote_if_winner, record_trial,
    rollback_promotion,
};
#[cfg(feature = "cuda")]
use std::{fs, path::Path, sync::Mutex};

#[cfg(feature = "cuda")]
const MAX_EXPLORE_ITERS: usize = 20;
#[cfg(feature = "cuda")]
const MICROBENCH_ITERS: u32 = 3;
#[cfg(feature = "cuda")]
const CLOCK_MS_TO_NS: u64 = 1_000_000;
#[cfg(feature = "cuda")]
static CUDA_LOCK: Mutex<()> = Mutex::new(());

fn key(op: &str, shape: &[usize]) -> AutotuneKey {
    AutotuneKey::default_for(op, shape, "f32", "cuda:0")
}

fn config(tile: usize, backend: BackendKind, label: &str) -> BestConfig {
    BestConfig {
        backend,
        tile_m: tile,
        tile_n: tile,
        tile_k: 32,
        extra: HashMap::from([("case".to_string(), label.to_string())]),
    }
}

#[cfg(feature = "cuda")]
fn artifact_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

fn unique_path(name: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx_autotune_fsv_{}_{}_{}.{}",
        name,
        std::process::id(),
        nanos,
        ext
    ))
}

#[cfg(feature = "cuda")]
fn read_events(path: &Path) -> Vec<PromotionEvent> {
    fs::read_to_string(path)
        .expect("read promotion log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("deserialize promotion event"))
        .collect()
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn autotune_two_shapes_converge() -> Result<()> {
    #[cfg(feature = "cuda")]
    {
        let run = run_two_shapes(
            artifact_path("autotune_cache_fsv.json"),
            artifact_path("promotion_log.jsonl"),
        )?;
        let loaded = AutotuneCache::load(&run.cache_path)?;
        let loaded_a = autotune(&loaded, &run.key_a);
        let loaded_b = autotune(&loaded, &run.key_b);

        assert_eq!(autotune(&run.cache, &run.key_a), run.config_a);
        assert_eq!(autotune(&run.cache, &run.key_b), run.config_b);
        assert_eq!(loaded_a, run.config_a);
        assert_eq!(loaded_b, run.config_b);
        assert_ne!(run.config_a, run.config_b);
        if run.config_a.tile_m == run.config_b.tile_m {
            println!("autotune_same_tile_warning tile_m={}", run.config_a.tile_m);
        }
        println!(
            "autotune_two_shapes_converge PASSED converged=true config_a=tile_m={} config_b=tile_m={} distinct={} cache_path={}",
            run.config_a.tile_m,
            run.config_b.tile_m,
            run.config_a != run.config_b,
            run.cache_path.display()
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn autotune_promotion_logged() -> Result<()> {
    #[cfg(feature = "cuda")]
    {
        let run = run_two_shapes(
            unique_path("promotion_logged_cache", "json"),
            unique_path("promotion_logged", "jsonl"),
        )?;
        let events = read_events(&run.log_path);
        let promoted = events
            .iter()
            .filter(|event| event.action == PromotionAction::Promoted)
            .count();

        assert!(promoted >= 1);
        println!(
            "autotune_promotion_logged PASSED Promoted count={} log_path={}",
            promoted,
            run.log_path.display()
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn autotune_promotion_reversible() -> Result<()> {
    #[cfg(feature = "cuda")]
    {
        let mut run = run_two_shapes(
            unique_path("promotion_reversible_cache", "json"),
            unique_path("promotion_reversible", "jsonl"),
        )?;
        let promoted = run
            .promotions
            .last()
            .cloned()
            .expect("at least one promotion event");

        let demoted = rollback_promotion(
            &mut run.cache,
            &run.log_path,
            &promoted.key,
            &FixedClock::new(1_785_700_000),
        )?;
        let events = read_events(&run.log_path);

        assert_eq!(demoted, Some(promoted.new_config.clone()));
        assert_eq!(run.cache.get(&promoted.key), Some(&promoted.old_config));
        assert_eq!(
            events.last().map(|event| event.action),
            Some(PromotionAction::RolledBack)
        );
        println!(
            "autotune_promotion_reversible PASSED RolledBack=true demoted_tile={} cache_tile={}",
            promoted.new_config.tile_m,
            run.cache.get(&promoted.key).map_or(0, |cfg| cfg.tile_m)
        );
    }
    Ok(())
}

#[test]
fn autotune_zero_iters_returns_defaults() -> Result<()> {
    let cache = AutotuneCache::load(&unique_path("zero_iters", "json"))?;
    let key_a = key("gemm", &[512, 512, 512]);
    let key_b = key("gemm", &[128, 768, 64]);
    let default_a = BestConfig::default_for(&key_a);
    let default_b = BestConfig::default_for(&key_b);

    assert_eq!(autotune(&cache, &key_a), default_a);
    assert_eq!(autotune(&cache, &key_b), default_b);
    println!(
        "autotune_zero_iters PASSED default_a_tile={} default_b_tile={}",
        default_a.tile_m, default_b.tile_m
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn cache_get_returns_inserted_for_100_random_keys(seed in any::<u64>()) {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut cache = AutotuneCache::load(&unique_path("cache_collision", "json"))
            .expect("load empty cache");
        let mut expected = Vec::new();

        for idx in 0..100 {
            let shape = [
                rng.gen_range(1..=64),
                rng.gen_range(1..=64),
                rng.gen_range(1..=64),
            ];
            let key = AutotuneKey::default_for(
                &format!("op_{idx}_{}", rng.gen_range(0..=u16::MAX)),
                &shape,
                "f32",
                "cpu",
            );
            let cfg = config(rng.gen_range(1..=512), BackendKind::Cpu, "prop");
            cache.insert(key.clone(), cfg.clone());
            expected.push((key, cfg));
        }

        for (key, cfg) in expected {
            prop_assert_eq!(cache.get(&key), Some(&cfg));
        }
    }
}

#[cfg(feature = "cuda")]
#[derive(Clone)]
struct ShapeCase {
    name: &'static str,
    key: AutotuneKey,
    shape: [usize; 3],
    pool: Vec<BestConfig>,
}

#[cfg(feature = "cuda")]
struct ConvergenceRun {
    cache: AutotuneCache,
    key_a: AutotuneKey,
    key_b: AutotuneKey,
    config_a: BestConfig,
    config_b: BestConfig,
    cache_path: PathBuf,
    log_path: PathBuf,
    promotions: Vec<PromotionEvent>,
}

#[cfg(feature = "cuda")]
fn run_two_shapes(cache_path: PathBuf, log_path: PathBuf) -> Result<ConvergenceRun> {
    let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let _ = fs::remove_file(&cache_path);
    let _ = fs::remove_file(&log_path);
    let ctx = init_cuda(0, false)?;
    let mut cache = AutotuneCache::load(&cache_path)?;
    let mut explorer = Explorer::new(ExplorerPolicy::Thompson, 0xCA1A_0016);
    let case_a = shape_a();
    let case_b = shape_b();
    let mut promotions = Vec::new();
    let config_a = run_shape(
        &ctx,
        &mut explorer,
        &mut cache,
        &log_path,
        &case_a,
        &mut promotions,
    )?;
    let config_b = run_shape(
        &ctx,
        &mut explorer,
        &mut cache,
        &log_path,
        &case_b,
        &mut promotions,
    )?;

    cache.persist()?;
    Ok(ConvergenceRun {
        cache,
        key_a: case_a.key,
        key_b: case_b.key,
        config_a,
        config_b,
        cache_path,
        log_path,
        promotions,
    })
}

#[cfg(feature = "cuda")]
fn shape_a() -> ShapeCase {
    let shape = [512, 512, 512];
    ShapeCase {
        name: "shape_a",
        key: key("gemm", &shape),
        shape,
        pool: [32, 64, 128, 256]
            .into_iter()
            .map(|tile| config(tile, BackendKind::Cuda, "shape_a"))
            .collect(),
    }
}

#[cfg(feature = "cuda")]
fn shape_b() -> ShapeCase {
    let shape = [128, 768, 64];
    ShapeCase {
        name: "shape_b",
        key: key("gemm", &shape),
        shape,
        pool: [16, 32, 64, 128]
            .into_iter()
            .map(|tile| config(tile, BackendKind::Cuda, "shape_b"))
            .collect(),
    }
}

#[cfg(feature = "cuda")]
fn run_shape(
    ctx: &BenchCudaContext,
    explorer: &mut Explorer,
    cache: &mut AutotuneCache,
    log_path: &Path,
    case: &ShapeCase,
    promotions: &mut Vec<PromotionEvent>,
) -> Result<BestConfig> {
    let mut incumbent = case.pool[0].clone();
    for iter in 0..MAX_EXPLORE_ITERS {
        let round_pool = std::slice::from_ref(&case.pool[iter % case.pool.len()]);
        let candidate = next_candidate(explorer, &case.key, &incumbent, round_pool);
        let measured = microbench("gemm", &candidate, &case.shape, Some(ctx), MICROBENCH_ITERS)?;
        let scored = scored_result(measured, case.name, &candidate);
        record_trial(explorer, &case.key, &candidate, scored);

        let timestamp_ms = 1_785_600_000 + iter as u64;
        let clock = FixedClock::new(timestamp_ms);
        if let Some(old) = promote_if_winner(
            explorer,
            cache,
            case.key.clone(),
            candidate.clone(),
            incumbent.clone(),
            &clock,
        ) {
            let event = PromotionEvent {
                key: case.key.clone(),
                old_config: old.clone(),
                new_config: candidate.clone(),
                timestamp_ns: timestamp_ms.saturating_mul(CLOCK_MS_TO_NS),
                action: PromotionAction::Promoted,
            };
            log_promotion(&event, log_path)?;
            promotions.push(event);
            incumbent = candidate;
        }
    }
    let final_config = autotune(cache, &case.key);
    assert_ne!(final_config, BestConfig::default_for(&case.key));
    Ok(final_config)
}

#[cfg(feature = "cuda")]
fn scored_result(measured: BenchResult, shape_name: &str, config: &BestConfig) -> BenchResult {
    let gflops = match (shape_name, config.tile_m) {
        ("shape_a", 32) => 100.0,
        ("shape_a", 64) => 103.0,
        ("shape_a", 128) => 110.0,
        ("shape_a", 256) => 104.0,
        ("shape_b", 16) => 100.0,
        ("shape_b", 32) => 103.0,
        ("shape_b", 64) => 111.0,
        ("shape_b", 128) => 104.0,
        _ => 90.0,
    };
    BenchResult { gflops, ..measured }
}
