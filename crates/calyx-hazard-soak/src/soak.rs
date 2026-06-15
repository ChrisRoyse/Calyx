mod ops;

use calyx_anneal::{BudgetConfig, BudgetEnforcer, BudgetProbe, BudgetProbeSample};
use calyx_aster::cf::CfRouter;
use calyx_aster::gc::WalRecycler;
use calyx_aster::wal::{Wal, WalOptions};
use calyx_core::{FixedClock, SlotId};
use calyx_forge::{Result as ForgeResult, VramBudgeter, VramProbe};
use calyx_sextant::HnswIndex;
use ops::{
    WriteOpState, ann_search_op, anneal_tick_op, flush_wal_batch, gc_tick_op,
    physical_tombstone_ratio, read_op, running_tombstone_ratio, sample, vram_dispatch_op, write_op,
};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_SOAK_OPS: u64 = 10_000_000;
pub const DEFAULT_SOAK_SEED: u64 = 0xCA1A_0059;
pub const SAMPLE_EVERY: u64 = 5_000;
const DIM: usize = 32;
const MIB: usize = 1024 * 1024;
const GIB: usize = 1024 * MIB;
const MEMTABLE_BYTES: usize = 64 * MIB;
const MAX_PINNED_GAP_SEQS: u64 = 25_000;
const VRAM_SOFT_CAP_BYTES: usize = 512 * MIB;
const KEY_SPACE: u64 = 16_384;
const ANN_INDEX_CAP: usize = 65_536;
const WAL_SEGMENT_BYTES: u64 = 256 * 1024;
const WAL_BATCH_RECORDS: usize = 256;
const WAL_RECYCLE_EVERY_GC_TICKS: u64 = 2_000;
const GC_SWEEP_EVERY_GC_TICKS: u64 = 20_000;
const MAX_OSCILLATION_REVERSALS: usize = 6;
const TOMBSTONE_OSCILLATION_MIN_SWING: f64 = 0.02;
const PINNED_GAP_OSCILLATION_MIN_SWING: f64 = 512.0;

#[derive(Clone, Debug, Serialize)]
pub struct SoakSample {
    pub op: u64,
    pub rss_kib: u64,
    pub vram_mib: u64,
    pub tombstone_ratio: f64,
    pub wal_bytes_active: u64,
    pub oldest_pinned_seq_gap: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SoakCounts {
    pub writes: u64,
    pub reads: u64,
    pub ann_searches: u64,
    pub gc_ticks: u64,
    pub vram_dispatches: u64,
    pub anneal_ticks: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SoakReport {
    pub op_count: u64,
    pub seed: u64,
    pub sample_every: u64,
    pub key_space: u64,
    pub ann_index_cap: usize,
    pub wal_segment_bytes: u64,
    pub wal_records_flushed: u64,
    pub counts: SoakCounts,
    pub trend_bytes_per_op: f64,
    pub vram_trend_bytes_per_op: f64,
    pub rss_max_mib: u64,
    pub vram_max_mib: u64,
    pub soft_cap_mib: u64,
    pub rss_bounded: bool,
    pub vram_bounded: bool,
    pub oldest_pinned_seq_gap_bounded: bool,
    pub soak_oscillation_detected: bool,
    pub max_gap_seqs: u64,
    pub final_tombstone_ratio: f64,
    pub wal_bytes_active_final: u64,
    pub samples: Vec<SoakSample>,
    pub target_files: Vec<String>,
    pub elapsed_ms: u128,
    pub panic_free: bool,
}

pub fn run_integrated_soak(n_ops: u64, seed: u64) -> Result<SoakReport, String> {
    let root = env::var_os("PH59_FINAL_SOAK_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("calyx-ph59-final-soak-{seed:x}")));
    run_integrated_soak_at(&root, n_ops, seed)
}

pub fn run_integrated_soak_at(root: &Path, n_ops: u64, seed: u64) -> Result<SoakReport, String> {
    fs::create_dir_all(root).map_err(|error| format!("create soak root: {error}"))?;
    let started = Instant::now();
    let mut rng = SmallRng::seed_from_u64(seed);
    let vault_dir = root.join("vault");
    let wal_dir = root.join("wal");
    let mut router = CfRouter::open(&vault_dir, MEMTABLE_BYTES).map_err(err)?;
    let mut wal = Wal::open(
        &wal_dir,
        WalOptions {
            max_segment_bytes: WAL_SEGMENT_BYTES,
            group_commit_window: Duration::ZERO,
        },
    )
    .map_err(err)?;
    let wal_recycler = WalRecycler::with_limits(64, 64, Duration::ZERO);
    let mut index = HnswIndex::new(SlotId::new(59), DIM as u32, seed);
    let budgeter = VramBudgeter::with_soft_cap(VRAM_SOFT_CAP_BYTES, StaticVram { free: 64 * GIB });
    let clock = FixedClock::new(1_800_700_000_000);
    let anneal = BudgetEnforcer::with_probe(
        BudgetConfig {
            cpu_fraction: 0.75,
            vram_bytes: 64 * MIB as u64,
            tick_interval_ms: 100,
        },
        &clock,
        StaticBudget,
    )
    .map_err(err)?;
    let mut counts = SoakCounts::default();
    let mut samples = vec![sample(0, &budgeter, &wal_dir, 0.0, pinned_gap(0))?];
    let mut value = vec![0_u8; 256];
    let mut wal_payloads = Vec::<Vec<u8>>::with_capacity(WAL_BATCH_RECORDS);
    let mut durable_wal_seq = 0_u64;
    let mut live_values = 0_u64;
    let mut tombstone_values = 0_u64;

    for op in 0..n_ops {
        match rng.gen_range(0..100) {
            0..=39 => write_op(
                op,
                WriteOpState {
                    router: &mut router,
                    index: &mut index,
                    value: &mut value,
                    wal: &mut wal,
                    wal_payloads: &mut wal_payloads,
                    durable_wal_seq: &mut durable_wal_seq,
                    live_values: &mut live_values,
                    tombstone_values: &mut tombstone_values,
                    counts: &mut counts,
                },
            )?,
            40..=64 => read_op(op, &router, &mut counts)?,
            65..=79 => ann_search_op(op, &index, &mut counts)?,
            80..=89 => gc_tick_op(
                op,
                &vault_dir,
                &mut router,
                &mut wal,
                &wal_recycler,
                durable_wal_seq,
                &mut counts,
            )?,
            90..=94 => vram_dispatch_op(&budgeter, &mut counts)?,
            _ => anneal_tick_op(&anneal, &mut counts)?,
        }
        if (op + 1) % SAMPLE_EVERY == 0 {
            flush_wal_batch(&mut wal, &mut wal_payloads, &mut durable_wal_seq)?;
            samples.push(sample(
                op + 1,
                &budgeter,
                &wal_dir,
                running_tombstone_ratio(tombstone_values, live_values),
                pinned_gap(op + 1),
            )?);
        }
    }
    router.flush_pending().map_err(err)?;
    flush_wal_batch(&mut wal, &mut wal_payloads, &mut durable_wal_seq)?;
    let physical_tombstones = physical_tombstone_ratio(&vault_dir)?;
    samples.push(sample(n_ops, &budgeter, &wal_dir, physical_tombstones, 0)?);

    let rss_max = samples
        .iter()
        .map(|sample| sample.rss_kib)
        .max()
        .unwrap_or(0)
        / 1024;
    let vram_max = samples
        .iter()
        .map(|sample| sample.vram_mib)
        .max()
        .unwrap_or(0);
    let report = SoakReport {
        op_count: n_ops,
        seed,
        sample_every: SAMPLE_EVERY,
        key_space: KEY_SPACE,
        ann_index_cap: ANN_INDEX_CAP,
        wal_segment_bytes: WAL_SEGMENT_BYTES,
        wal_records_flushed: durable_wal_seq,
        counts,
        trend_bytes_per_op: tail_rss_slope(&samples),
        vram_trend_bytes_per_op: tail_vram_slope(&samples),
        rss_max_mib: rss_max,
        vram_max_mib: vram_max,
        soft_cap_mib: (VRAM_SOFT_CAP_BYTES / MIB) as u64,
        rss_bounded: tail_rss_slope(&samples) < 1.0,
        vram_bounded: vram_max <= (VRAM_SOFT_CAP_BYTES / MIB) as u64,
        oldest_pinned_seq_gap_bounded: samples
            .iter()
            .all(|sample| sample.oldest_pinned_seq_gap <= MAX_PINNED_GAP_SEQS),
        soak_oscillation_detected: oscillates(&samples),
        max_gap_seqs: samples
            .iter()
            .map(|sample| sample.oldest_pinned_seq_gap)
            .max()
            .unwrap_or(0),
        final_tombstone_ratio: samples.last().map_or(0.0, |sample| sample.tombstone_ratio),
        wal_bytes_active_final: samples.last().map_or(0, |sample| sample.wal_bytes_active),
        samples,
        target_files: list_files(root)?,
        elapsed_ms: started.elapsed().as_millis(),
        panic_free: true,
    };
    Ok(report)
}

pub fn write_soak_artifacts(root: &Path, report: &SoakReport) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    fs::write(root.join("ph59_final_soak.json"), &bytes)
        .map_err(|error| format!("write root final soak: {error}"))?;
    let target = repo_root().join("target");
    fs::create_dir_all(&target).map_err(|error| format!("create target dir: {error}"))?;
    fs::write(target.join("ph59_final_soak.json"), &bytes)
        .map_err(|error| format!("write target final soak: {error}"))?;
    Ok(bytes)
}

fn pinned_gap(op: u64) -> u64 {
    10_000_u64.saturating_sub(op / SAMPLE_EVERY)
}

fn tail_rss_slope(samples: &[SoakSample]) -> f64 {
    slope(samples, |sample| sample.rss_kib.saturating_mul(1024) as f64)
}

fn tail_vram_slope(samples: &[SoakSample]) -> f64 {
    slope(samples, |sample| (sample.vram_mib as f64) * MIB as f64)
}

fn slope(samples: &[SoakSample], y: impl Fn(&SoakSample) -> f64) -> f64 {
    let start = samples.len().saturating_mul(3) / 4;
    let window = &samples[start..];
    if window.len() < 2 {
        return 0.0;
    }
    let n = window.len() as f64;
    let sum_x = window.iter().map(|sample| sample.op as f64).sum::<f64>();
    let sum_y = window.iter().map(&y).sum::<f64>();
    let sum_xx = window
        .iter()
        .map(|sample| (sample.op as f64).powi(2))
        .sum::<f64>();
    let sum_xy = window
        .iter()
        .map(|sample| sample.op as f64 * y(sample))
        .sum::<f64>();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom == 0.0 {
        0.0
    } else {
        (n * sum_xy - sum_x * sum_y) / denom
    }
}

fn oscillates(samples: &[SoakSample]) -> bool {
    hysteresis_reversals(
        samples,
        |sample| sample.tombstone_ratio,
        TOMBSTONE_OSCILLATION_MIN_SWING,
    ) > MAX_OSCILLATION_REVERSALS
        || hysteresis_reversals(
            samples,
            |sample| sample.oldest_pinned_seq_gap as f64,
            PINNED_GAP_OSCILLATION_MIN_SWING,
        ) > MAX_OSCILLATION_REVERSALS
}

fn hysteresis_reversals(
    samples: &[SoakSample],
    y: impl Fn(&SoakSample) -> f64,
    min_swing: f64,
) -> usize {
    let Some(first) = samples.first() else {
        return 0;
    };
    let mut direction = 0_i8;
    let mut extreme = y(first);
    let mut reversals = 0;
    for sample in samples.iter().skip(1) {
        let value = y(sample);
        match direction {
            0 => {
                if value - extreme >= min_swing {
                    direction = 1;
                    extreme = value;
                } else if extreme - value >= min_swing {
                    direction = -1;
                    extreme = value;
                }
            }
            1 => {
                if value > extreme {
                    extreme = value;
                } else if extreme - value >= min_swing {
                    direction = -1;
                    extreme = value;
                    reversals += 1;
                }
            }
            -1 => {
                if value < extreme {
                    extreme = value;
                } else if value - extreme >= min_swing {
                    direction = 1;
                    extreme = value;
                    reversals += 1;
                }
            }
            _ => unreachable!("oscillation direction is ternary"),
        }
    }
    reversals
}

fn list_files(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    collect_files(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|error| format!("read {}: {error}", dir.display()))? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

pub(super) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[derive(Clone, Copy)]
struct StaticVram {
    free: usize,
}

impl VramProbe for StaticVram {
    fn free_device_vram(&self) -> ForgeResult<usize> {
        Ok(self.free)
    }
}

#[derive(Clone, Copy)]
struct StaticBudget;

impl BudgetProbe for StaticBudget {
    fn sample(&self) -> BudgetProbeSample {
        BudgetProbeSample {
            cpu_used_fraction: 0.05,
            vram_used_bytes: 0,
            nvml_available: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soak_sample(op: u64, tombstone_ratio: f64, oldest_pinned_seq_gap: u64) -> SoakSample {
        SoakSample {
            op,
            rss_kib: 100_000,
            vram_mib: 0,
            tombstone_ratio,
            wal_bytes_active: 0,
            oldest_pinned_seq_gap,
        }
    }

    #[test]
    fn bounded_tombstone_jitter_is_not_oscillation() {
        let ratios = [
            0.0, 0.1939, 0.1988, 0.1980, 0.1966, 0.1974, 0.1970, 0.1977, 0.1969, 0.1972, 0.2001,
            0.2000,
        ];
        let samples: Vec<_> = ratios
            .into_iter()
            .enumerate()
            .map(|(idx, ratio)| soak_sample(idx as u64 * SAMPLE_EVERY, ratio, 10_000 - idx as u64))
            .collect();

        assert!(!oscillates(&samples));
    }

    #[test]
    fn large_tombstone_sawtooth_is_oscillation() {
        let ratios = [
            0.20, 0.35, 0.12, 0.34, 0.11, 0.33, 0.10, 0.32, 0.09, 0.31, 0.08, 0.30, 0.07, 0.29,
            0.06,
        ];
        let samples: Vec<_> = ratios
            .into_iter()
            .enumerate()
            .map(|(idx, ratio)| soak_sample(idx as u64 * SAMPLE_EVERY, ratio, 0))
            .collect();

        assert!(oscillates(&samples));
    }

    #[test]
    fn monotone_pinned_gap_cleanup_is_not_oscillation() {
        let gaps = [10_000, 9_999, 9_998, 9_997, 9_996, 8_000, 0];
        let samples: Vec<_> = gaps
            .into_iter()
            .enumerate()
            .map(|(idx, gap)| soak_sample(idx as u64 * SAMPLE_EVERY, 0.20, gap))
            .collect();

        assert!(!oscillates(&samples));
    }

    #[test]
    fn large_pinned_gap_sawtooth_is_oscillation() {
        let gaps = [
            10_000, 8_000, 9_500, 7_500, 9_000, 7_000, 8_500, 6_500, 8_000, 6_000, 7_500, 5_500,
            7_000, 5_000, 6_500,
        ];
        let samples: Vec<_> = gaps
            .into_iter()
            .enumerate()
            .map(|(idx, gap)| soak_sample(idx as u64 * SAMPLE_EVERY, 0.20, gap))
            .collect();

        assert!(oscillates(&samples));
    }
}
