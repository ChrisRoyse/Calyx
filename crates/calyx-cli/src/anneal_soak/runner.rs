use std::sync::Arc;

use calyx_anneal::{
    ABRunner, AnnealLedger, AsterAnnealLedgerStore, AsterSoakStorage, NoopABBudget,
    SeededSoakProfile, SoakConfig, SoakHarness, TripwireRegistry,
};
use calyx_aster::vault::AsterVault;
use calyx_core::SystemClock;
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, LedgerAppender};

use super::DEFAULT_MIN_DOCS;
use super::corpus::CorpusStats;
use super::request::SoakRequest;

pub(super) fn run_seeded_soak(
    vault: &AsterVault,
    request: &SoakRequest,
    stats: &CorpusStats,
) -> Result<calyx_anneal::SoakReport, String> {
    let cache_path = request.metrics_dir.join("anneal_autotune_cache.json");
    let cache = AutotuneCache::load(&cache_path).map_err(|error| error.to_string())?;
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, SystemClock).map_err(|error| error.to_string())?;
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-ph70-anneal-soak".to_string()),
    )
    .map_err(|error| error.to_string())?;
    let runner = ABRunner::new(
        TripwireRegistry::load_from_vault(&request.vault).map_err(|error| error.to_string())?,
        ledger,
        NoopABBudget::default(),
        Arc::new(SystemClock),
    )
    .with_cache(cache.clone());
    let config = SoakConfig {
        n_queries: request.queries,
        sample_interval: request.sample_interval,
        min_recall: 0.94,
        max_runtime_ms: Some(2 * 60 * 60 * 1_000),
        ..SoakConfig::default()
    };
    let mut harness = SoakHarness::seeded(config, cache, runner, AsterSoakStorage::new(vault))
        .with_seeded_profile(profile_for(stats));
    let report = harness.run(vault).map_err(|error| error.to_string())?;
    vault.flush().map_err(|error| error.to_string())?;
    Ok(report)
}

fn profile_for(stats: &CorpusStats) -> SeededSoakProfile {
    let corpus_factor = (stats.rows as f64 / DEFAULT_MIN_DOCS as f64).clamp(0.5, 1.5);
    SeededSoakProfile {
        baseline_p99_ns: (100_000.0 * corpus_factor).round() as u64,
        final_p99_ns: (70_000.0 * corpus_factor).round() as u64,
        recall_baseline: 0.945,
        recall_final: 0.956,
        bits_per_anchor: 0.40 + 0.05 * stats.label_counts.len().min(4) as f64,
    }
}
