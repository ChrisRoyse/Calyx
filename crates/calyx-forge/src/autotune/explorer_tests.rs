use std::collections::HashMap;
use std::path::Path;

use calyx_core::FixedClock;

use super::{
    AutotuneCache, AutotuneKey, BenchResult, EPSILON, Explorer, ExplorerPolicy, MIN_PROMOTE_TRIALS,
    next_candidate, promote_if_winner, record_trial, should_promote,
};
use crate::{BackendKind, BestConfig, Result};

fn key() -> AutotuneKey {
    AutotuneKey::default_for("gemm", &[1024, 1024, 1024], "f32", "cuda:0")
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

fn result(gflops: f64) -> BenchResult {
    BenchResult {
        gflops,
        elapsed_ms: 1.0,
        cv_pct: 1.0,
    }
}

fn record_many(explorer: &mut Explorer, key: &AutotuneKey, cfg: &BestConfig, values: &[f64]) {
    for value in values {
        record_trial(explorer, key, cfg, result(*value));
    }
}

#[test]
fn explorer_should_promote_three_trials_three_pct() {
    let key = key();
    let incumbent = config(64);
    let challenger = config(128);
    let mut explorer = Explorer::new(ExplorerPolicy::EpsilonGreedy, 7);

    record_many(&mut explorer, &key, &incumbent, &[100.0, 100.0, 100.0]);
    record_many(&mut explorer, &key, &challenger, &[103.0, 103.0, 103.0]);

    assert!(should_promote(&explorer, &key, &challenger, &incumbent));
    println!(
        "explorer_should_promote_3pct PASSED promote=true trials={} challenger_mean=103 incumbent_mean=100",
        MIN_PROMOTE_TRIALS
    );
}

#[test]
fn explorer_should_not_promote_with_two_trials() {
    let key = key();
    let incumbent = config(64);
    let challenger = config(128);
    let mut explorer = Explorer::new(ExplorerPolicy::EpsilonGreedy, 7);

    record_many(&mut explorer, &key, &incumbent, &[100.0, 100.0]);
    record_many(&mut explorer, &key, &challenger, &[110.0, 110.0]);

    assert!(!should_promote(&explorer, &key, &challenger, &incumbent));
    println!("explorer_not_enough_trials PASSED promote=false trials=2");
}

#[test]
fn explorer_should_not_promote_below_margin() {
    let key = key();
    let incumbent = config(64);
    let challenger = config(128);
    let mut explorer = Explorer::new(ExplorerPolicy::EpsilonGreedy, 7);

    record_many(&mut explorer, &key, &incumbent, &[100.0, 100.0, 100.0]);
    record_many(&mut explorer, &key, &challenger, &[101.0, 101.0, 101.0]);

    assert!(!should_promote(&explorer, &key, &challenger, &incumbent));
    println!("explorer_below_margin PASSED promote=false margin=0.01");
}

#[test]
fn epsilon_greedy_exploit_fraction() {
    let key = key();
    let incumbent = config(64);
    let pool: Vec<_> = (0..10).map(|idx| config(128 + idx)).collect();
    let mut explorer = Explorer::new(ExplorerPolicy::EpsilonGreedy, 42);
    let calls = 1_000;
    let mut exploit = 0;

    for _ in 0..calls {
        let candidate = next_candidate(&mut explorer, &key, &incumbent, &pool);
        if candidate == incumbent {
            exploit += 1;
        }
    }

    let exploit_fraction = exploit as f64 / calls as f64;
    assert!((0.85..=0.95).contains(&exploit_fraction));
    println!(
        "epsilon_greedy_exploit_fraction PASSED exploit_fraction={exploit_fraction:.3} epsilon={EPSILON}"
    );
}

#[test]
fn explorer_edges_empty_single_and_thompson_equal() {
    let key = key();
    let incumbent = config(64);
    let mut epsilon = Explorer::new(ExplorerPolicy::EpsilonGreedy, 5);
    let mut thompson = Explorer::new(ExplorerPolicy::Thompson, 9);

    assert_eq!(
        next_candidate(&mut epsilon, &key, &incumbent, &[]),
        incumbent
    );
    assert_eq!(
        next_candidate(
            &mut epsilon,
            &key,
            &incumbent,
            std::slice::from_ref(&incumbent)
        ),
        incumbent
    );

    let pool = [config(128), config(192), config(256)];
    let chosen = next_candidate(&mut thompson, &key, &incumbent, &pool);
    assert!(pool.contains(&chosen));
    println!(
        "explorer_edges PASSED empty_pool=incumbent single_pool=incumbent thompson_tile={}",
        chosen.tile_m
    );
}

#[test]
fn promote_if_winner_fixed_clock_returns_old_incumbent() -> Result<()> {
    let key = key();
    let incumbent = config(64);
    let challenger = config(128);
    let mut explorer = Explorer::new(ExplorerPolicy::EpsilonGreedy, 7);
    let cache_path =
        std::env::temp_dir().join(format!("calyx_explorer_unused_{}.json", std::process::id()));
    let mut cache = AutotuneCache::load(Path::new(&cache_path))?;
    let clock = FixedClock::new(1_785_555_000);

    record_many(&mut explorer, &key, &incumbent, &[100.0, 100.0, 100.0]);
    record_many(&mut explorer, &key, &challenger, &[103.0, 103.0, 103.0]);

    let old = promote_if_winner(
        &mut explorer,
        &mut cache,
        key.clone(),
        challenger.clone(),
        incumbent.clone(),
        &clock,
    );

    assert_eq!(old, Some(incumbent.clone()));
    assert_eq!(cache.get(&key), Some(&challenger));
    assert_eq!(explorer.last_promotion_ts(), Some(1_785_555_000));
    println!(
        "explorer_promote_fixed_clock PASSED promote=true old_tile={} new_tile={} ts={}",
        incumbent.tile_m,
        challenger.tile_m,
        explorer.last_promotion_ts().unwrap_or_default()
    );
    Ok(())
}
