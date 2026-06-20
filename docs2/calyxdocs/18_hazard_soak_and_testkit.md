# Hazard Soak & Testkit (calyx-hazard-soak + calyx-testkit)

**Source files covered:**

- `crates/calyx-hazard-soak/Cargo.toml`
- `crates/calyx-hazard-soak/src/lib.rs`
- `crates/calyx-hazard-soak/src/main.rs`
- `crates/calyx-hazard-soak/src/cli.rs`
- `crates/calyx-hazard-soak/src/soak.rs`
- `crates/calyx-hazard-soak/src/soak/ops.rs`
- `crates/calyx-hazard-soak/src/hazards/mod.rs`
- `crates/calyx-hazard-soak/src/hazards/resource.rs`
- `crates/calyx-hazard-soak/src/hazards/resource_hazards_6_8.rs`
- `crates/calyx-hazard-soak/src/hazards/heap_soak.rs`
- `crates/calyx-hazard-soak/src/hazards/resource_support.rs`
- `crates/calyx-hazard-soak/src/hazards/numerical.rs`
- `crates/calyx-hazard-soak/src/hazards/numerical_support.rs`
- `crates/calyx-hazard-soak/src/hazards/operational.rs`
- `crates/calyx-hazard-soak/src/hazards/operational_h13_14.rs`
- `crates/calyx-hazard-soak/src/hazards/operational_h15_16.rs`
- `crates/calyx-hazard-soak/src/hazards/operational_h17_19.rs`
- `crates/calyx-hazard-soak/src/hazards/operational_h20_21.rs`
- `crates/calyx-hazard-soak/src/hazards/operational_support.rs`
- `crates/calyx-hazard-soak/src/hazards/security.rs`
- `crates/calyx-hazard-soak/src/hazards/security_support.rs`
- `crates/calyx-hazard-soak/benches/bench_hazard_soak_throughput.rs`
- `crates/calyx-testkit/Cargo.toml`
- `crates/calyx-testkit/src/lib.rs`

This document covers two support crates. `calyx-hazard-soak` is a standalone
fault-injection / resilience-probe harness and binary that runs the **PH59**
hazard suite (25 hazard probes, H1–H25) plus a long-running integrated soak.
`calyx-testkit` is a tiny reusable library of deterministic test scaffolding
(seeds, fixed clock, proptest strategies). They are unrelated except that both
serve testing. For the math/storage crates they exercise, see
[06_aster_storage_engine.md](06_aster_storage_engine.md),
[07_forge_math_runtime.md](07_forge_math_runtime.md),
[08_registry_lenses.md](08_registry_lenses.md),
[09_sextant_search.md](09_sextant_search.md), and
[15_anneal_optimization.md](15_anneal_optimization.md).

---

## Part A — calyx-hazard-soak

### A.1 What it is and how it is structured

`lib.rs` has no `//!` doc comment; it consists of a single line:
`pub mod soak;`. Everything else (`cli`, `hazards`) is private to the binary
(`main.rs` declares `mod cli; mod hazards;`). So the crate exposes exactly one
public module — `soak` — as a library; the hazard probes are reachable only
through the binary.

The crate is a **fault-injection and resource-leak test harness** ("hazard
soak"). It does two things:

1. **Hazard probes (H1–H25):** 25 short, self-contained probe functions, each
   one injecting a specific failure/stress condition against a real Calyx
   subsystem (Aster vault/WAL/MVCC/GC, Forge VRAM/quantization, Sextant
   ANN/rerank, Registry lenses, Anneal bandit, Ledger). Each probe asserts the
   system fails closed and/or stays bounded, and emits a structured JSON
   evidence object plus Prometheus-style metrics text.
2. **Integrated soak (`soak.rs`):** a single long-running loop (default
   10,000,000 mixed operations) against one live vault/WAL/HNSW/VRAM-budgeter,
   sampling RSS and VRAM to detect unbounded memory growth and oscillation.

Dependencies (`Cargo.toml`): `calyx-anneal`, `calyx-aster`, `calyx-core`,
`calyx-forge`, `calyx-ledger`, `calyx-registry`, `calyx-sextant`, `blake3`,
`rand` (with `small_rng`), `serde`, `serde_json`. Dev: `criterion`. One feature:
`cuda = ["calyx-forge/cuda"]`. One binary: `calyx-hazard-soak` (`src/main.rs`).
One bench: `bench_hazard_soak_throughput` (harness = false).

### A.2 Binary entry point and CLI (`main.rs`, `cli.rs`)

`main()` calls `run()`, prints `calyx-hazard-soak: {error}` and exits `1` on
error. `run()` parses args into `RunConfig`, picks a `Suite`, creates an FSV
root directory, runs the suite, optionally runs the final soak, writes
artifacts, and computes a pass/fail exit gate.

#### A.2.1 `RunConfig` (`cli.rs`, `pub(crate)`)

| Field | Type | Meaning |
|---|---|---|
| `suite` | `Suite` | which hazard set to run |
| `seed_input` | `String` | raw `--seed` text (default `"0xCALYX59"`) |
| `seed` | `u64` | parsed seed |
| `soak_ops` | `u64` | final-soak op count |

`RunConfig::parse(args)` flags:

| Arg | Effect |
|---|---|
| `--all-hazards` | `suite = Stage13Exit` (runs all + final soak) |
| `--hazards <range>` | `Suite::from_hazards_range(range)` |
| `--seed <value>` | sets `seed_input` |
| `--ops <n>` / `--soak-ops <n>` | sets `soak_ops` (parsed `u64`) |
| (none) | defaults to `Suite::Hazards1To5` |

Unknown args return `Err("unsupported arg …")`.

**Seed parsing** (`parse_seed`): the literal `"0xCALYX59"` (case-insensitive)
maps to `DEFAULT_SOAK_SEED` (`0xCA1A_0059`). Otherwise a `0x`/`0X` hex prefix is
parsed as hex `u64`; else a decimal parse is attempted; else the input is
BLAKE3-hashed and the first 8 bytes (big-endian) become the seed.

`soak_ops` defaults to env `PH59_FINAL_SOAK_OPS` if set, else `DEFAULT_SOAK_OPS`
(10,000,000).

`dmesg_oom_count()` shells out to `sh -lc "dmesg 2>/dev/null | grep -ci oom ||
true"` and parses the count (returns `None` if unavailable — non-Linux).

#### A.2.2 Hazard range → `Suite` mapping (`from_hazards_range`)

| `--hazards` value | Suite | Runs |
|---|---|---|
| `1-5` | `Hazards1To5` | H1–H5 |
| `6-8` | `Hazards6To8` | H6–H8 |
| `9-12` | `Hazards9To12` | H9–H12 |
| `13-16` | `Hazards13To16` | H13–H16 |
| `17-21` | `Hazards17To21` | H17–H21 |
| `22-25` | `Hazards22To25` | H22–H25 |
| `1-8` | `Hazards1To8` | H1–H8 |
| `1-12` | `Hazards1To12` | H1–H12 |
| `1-16`,`1-21`,`1-25` | `AllImplemented` | H1–H25 |

`--all-hazards` selects `Stage13Exit`, which runs `AllImplemented` **plus** the
final soak. Any other range string returns an error.

#### A.2.3 `Suite` metadata (per-variant `pub(crate)` accessors)

Each variant carries a GitHub issue number, a task code, a suite name, a metrics
prefix, JSON/Prometheus artifact filenames, and a root env-var name:

| Suite | issue | task | metrics_suite | json_artifact |
|---|---|---|---|---|
| Hazards1To5 | 488 | T01 | ph59_t01 | ph59_hazards_1_5.json |
| Hazards6To8 | 489 | T02 | ph59_t02 | ph59_hazards_6_8.json |
| Hazards9To12 | 490 | T03 | ph59_t03 | ph59_hazards_9_12.json |
| Hazards13To16 | 491 | T04 | ph59_t04 | ph59_hazards_13_16.json |
| Hazards17To21 | 492 | T05 | ph59_t05 | ph59_hazards_17_21.json |
| Hazards22To25 | 493 | T06 | ph59_t06 | ph59_hazards_22_25.json |
| Hazards1To8 | [488,489] | T01_T02 | ph59_t01_t02 | ph59_hazards_1_8.json |
| Hazards1To12 | [488,489,490] | T01_T03 | ph59_t01_t03 | ph59_hazards_1_12.json |
| AllImplemented | [488..493] | T01_T06 | ph59_t01_t06 | ph59_hazards_1_25.json |
| Stage13Exit | 494 | T07 | ph59_t07 | ph59_hazard_results.json |

`runs_final_soak()` is `true` only for `Stage13Exit`. The `.prom` artifact name
mirrors the JSON name with a `.prom` extension. `root_env_name()` returns names
like `PH59_HAZARDS_1_5_ROOT`, `PH59_STAGE13_ROOT`, etc.

#### A.2.4 FSV root selection and artifacts

`fsv_root(suite)` chooses the output directory: env `<suite root env>`, else
env `CALYX_FSV_ROOT`, else `temp_dir()/calyx-ph59-<task>-<pid>`. A
`cleanup-tag.txt` (`"issue{…} PH59 {task} synthetic FSV data\n"`) is written
there.

`write_artifacts` writes the JSON evidence to both the FSV root and
`<repo_root>/target/<json_artifact>`, plus a `.prom` metrics file.
`metrics_text` emits `calyx_hazard_pass_count{suite=…}`, one
`calyx_hazard_pass{suite,hazard="H<id>"}` per probe (0/1), inlines each probe's
own `metrics_text`, and (for the soak) Stage-13 gauges.

`repo_root()` = manifest dir's grandparent (`crates/calyx-hazard-soak` → repo).

#### A.2.5 Exit gate (`stage_passed`)

- Without a soak: pass iff **all** probes passed.
- With a soak (Stage13Exit): pass iff all probes passed **AND**
  `soak.rss_bounded` **AND** `soak.vram_bounded` **AND NOT**
  `soak.soak_oscillation_detected` **AND** `dmesg_oom_count == 0`.

On Stage13Exit it prints `STAGE13 EXIT GATE: hazard_pass_count=… rss_bounded=…
vram_bounded=… oscillation=…`. The final soak runs inside
`std::panic::catch_unwind`; a panic becomes the error `"PH59 final soak
panicked"`.

### A.3 Hazard probe machinery (`hazards/resource.rs`)

Every probe has signature `fn(&Path) -> ProbeResult` where
`pub(super) type ProbeResult = Result<(bool, serde_json::Value), String>` — the
`bool` is pass/fail, the `Value` is JSON evidence.

`run_probe(root, hazard_id, name, probe)` wraps the probe in
`catch_unwind(AssertUnwindSafe(...))` and produces a `HazardResult`:

```rust
pub struct HazardResult {           // hazards/resource.rs
    pub hazard_id: u8,
    pub name: &'static str,
    pub passed: bool,
    pub evidence: serde_json::Value,
}
```

- `Ok(Ok((passed, evidence)))` → that result.
- `Ok(Err(error))` → `passed=false`, evidence `{"error":…,"panic_free":true}`.
- `Err(payload)` (panic) → `passed=false`, evidence
  `{"panic":<text>,"panic_free":false}` (panic payload downcast to
  `&str`/`String`).

So a probe never aborts the suite; a panic is recorded as a failure.

### A.4 The 25 hazards

Each probe builds its own case directory under the FSV root via
`case_dir(root, name)` (which removes any prior dir first), opens real Aster
vaults/routers, injects a fault, and checks invariants. The evidence JSON always
includes a `"trigger"` string, an `"expected"` block, an `"actual"` block (with
`"panic_free": true`), and a `"metrics_text"` string. Hazard groups and the
fault each injects:

#### A.4.1 H1–H5 — resource / storage (`resource.rs`)

| ID | Name | Fault injected | Key invariants checked |
|---|---|---|---|
| 1 | write amplification / compaction storm | 30k write-heavy durable rows then base-CF compaction | `compacted` true, `write_amp ≤ 10.0`, serving p99 ≤ 2× baseline, compaction debt bounded |
| 2 | memtable flush stall | write flood into a 4 KiB-cap CF router + one oversized (8 KiB) row | every bucket acks > 0, memtable used ≤ cap, oversized row rejected with `CALYX_BACKPRESSURE`, RSS bounded |
| 3 | tombstone buildup | 100k rows, 70k MVCC tombstones, 5 compaction-GC sweeps (trigger ratio 0.4) | ratio before > 0.4, ratio after ≤ 0.1 |
| 4 | fsync latency spike | 100 durable writes + injected `fsync_p99` guard spike on `WalRecycler` | all 100 readable before/after reopen, recycler skips on `fsync_p99_guard` then `fsync_backoff_active`, recovers `< 10000 us`, no data loss |
| 5 | WAL bloat | 10k rows held before flush, flush, WAL recycle, reopen | WAL grew before flush, recycle triggered & shrank WAL, bounded ≤ 2× segment, 10k rows readable |

#### A.4.2 H6–H8 — MVCC / VRAM / heap (`resource_hazards_6_8.rs`, `heap_soak.rs`)

| ID | Name | Fault injected | Key invariants |
|---|---|---|---|
| 6 | MVCC version pile-up / long reader | reader pinned at seq 5000, 10k newer versions, lease expiry, snapshot GC | `oldest_pinned_seq_gap ≥ 9999` while pinned; expired read → `CALYX_READER_LEASE_EXPIRED`; `reader_lease_expired_total == 1`; post-GC gap < 10; bytes freed grew; on-disk SST bytes flat or smaller |
| 7 | VRAM OOM admission | **20 concurrent threads** each requesting 200 MiB Forge admission against a 2 GiB soft cap (plus a zero-budget probe) | ≥1 `CALYX_FORGE_VRAM_BUDGET` error, 0 panics, 0 other errors, `failed_total ≥ 1`, `nvidia-smi` delta ≤ 2560 MiB, no `dmesg` OOM lines |
| 8 | heap OOM bounded soak | 10M-op bounded allocator/router soak (env `PH59_H8_OPS`/`PH59_H8_FLOOD_ROWS`) + 100k max-size write burst at the midpoint | ≥1 backpressure event, RSS max ≤ budget, RSS trend < 1.0 B/op, cache/arena within caps, fail-closed code `CALYX_BACKPRESSURE` |

H8 (`heap_soak.rs`) exercises `Arena` (4 MiB cap), `SlabPool<256>` (1024 slots),
`PageAlignedSlabPool`, `LruTtlCache` (16 MiB), and a `CfRouter` (32 MiB
memtable). RSS budget = initial RSS + 1.20·(cap sum) + 64 MiB headroom; the trend
is a least-squares slope over the last quarter of samples (sample every 10,000
ops). H7 uses `GpuBlockRegistry` + `AdmissionController` from calyx-forge with a
`StaticProbe` (64 GiB free) and a `NoopDealloc`.

#### A.4.3 H9–H12 — numerical / index (`numerical.rs`, `numerical_support.rs`)

DIM=128, ROWS=1000, K=10, distortion bound 0.12, fail-closed code
`CALYX_QUANT_DRIFT_EXCEEDED`.

| ID | Name | Fault injected | Key invariants |
|---|---|---|---|
| 9 | NaN/Inf propagation guard | one single-NaN vector and one all-NaN vector through the Forge numerical boundary | both rejected with `CALYX_FORGE_NUMERICAL_INVARIANT` (CUDA `topk` if `cuda` feature, else CPU `TurboQuantCodec`); slot CF stays at 0 rows; no NaN f32 pattern persisted on disk |
| 10 | TurboQuant drift and recall | 1000 paired vectors encoded `Bits3p5`, decoded, searched via HNSW | max relative IP error ≤ 0.12; quantized recall@10 ≥ 0.95× full recall; min-bitwidth contract trips `CALYX_QUANT_DRIFT_EXCEEDED` |
| 11 | QJL seed/codebook staleness | re-quantize one vector with same vs. different rotation seed | same seed → bit-identical bytes; different seed → different bytes + different `seed_id` (data-oblivious rotation, no codebook) |
| 12 | ANN graph corruption rebuild | flip 8 bytes in a DiskANN graph header, fall back to base-CF HNSW, rebuild | open of corrupt graph → `CALYX_INDEX_CORRUPT`; fallback HNSW returns correct top hit; rebuild restores top hit; base-CF bytes unchanged |

#### A.4.4 H13–H16 — operational concurrency (`operational_h13_14.rs`, `operational_h15_16.rs`)

| ID | Name | Fault injected | Key invariants |
|---|---|---|---|
| 13 | hot-shard tenant skew | 10 vaults, 1000 writes routed ~90% to vault 0, per-vault `QuotaGuard` (120 cx/s) | hot vault rate-limited (`CALYX_QUOTA_EXCEEDED`); cool vaults keep ≥ 0.5× read throughput; all vaults still readable |
| 14 | lock contention / deadlock | **64 concurrent writer clients** funneled through a single-writer-per-vault thread; readers pin an MVCC snapshot; synthetic `try_lock_for` timeout | all 64 writers complete 8 rows each, all rows readback, read throughput ≥ 0.8× baseline, lock timeout → `CALYX_LOCK_TIMEOUT` |
| 15 | cache stampede single-flight | expired hot kernel entry + **100 concurrent identical misses** on a `SingleFlightCache` | exactly 1 recompute, all 100 callers get `kernel-v2:42`, plus a zero-jitter edge case (20 callers → 1 recompute) |
| 16 | slow-lens head-of-line | one 50 ms `TimeoutLens` + 2 fast lenses in a registry microbatch; breaker (1 trip / 10000 ms) | returns within timeout+100 ms; slow lens → `CALYX_LENS_UNREACHABLE`; fast lenses present; breaker trips then recovers; open breaker skips the slow lens (no extra calls) |

#### A.4.5 H17–H21 — operational resilience (`operational_h17_19.rs`, `operational_h20_21.rs`)

| ID | Name | Fault injected | Key invariants |
|---|---|---|---|
| 17 | disk pressure fail-closed | `AtomicDiskProbe` drops available blocks (admit @80% used, reject @90%, recover @84%) with a `DiskPressureGuard` + `SpillTrigger` | reject → `CALYX_DISK_PRESSURE`, seq not advanced, rejected key absent, spill requested, recovered write present, boundary check rejected |
| 18 | ARC/read-thrash graceful degradation | 8 MiB thrash file + deterministic file churn between Aster read loops | all DB reads hit before/after; recovery ops ratio ≥ 0.05; read p99 not unbounded (≤ 100× before) |
| 19 | clock skew monotonic sequence | durable writes with forward clock, then backwards clock, then zero clock; reopen | MVCC seq strictly increases regardless of wall clock; 41 rows survive reopen; latest seq survives reopen; time-index rows ≥ 41 |
| 20 | Anneal thrash hysteresis | oscillating bandit candidate wins then stable wins; persist to `anneal_bandit` CF | oscillation does NOT promote (incumbent stays 0); stable candidate promotes after hysteresis (3); bandit row persisted & decodes; `check_oscillation` detects rising p99 but not stable |
| 21 | panel-version / cross-term explosion | 12 panel files, 3 live panel-version rows, capped (4/cx) xterm materialization, then panel GC | 9 unreferenced versions moved to cold then pruned; live versions {10,11,12} preserved; xterm rows ≤ cap/cx; over-cap pairs skipped & absent; 0 temporal xterm rows |

#### A.4.6 H22–H25 — security / upgrade (`security.rs`, `security_support.rs`)

Secret token `CALYX_TEST_SECRET_ABCD1234`, DIM=32.

| ID | Name | Fault injected | Key invariants |
|---|---|---|---|
| 22 | secret leakage / request-text non-persistence | inject the secret into rerank, embed, and search request text; append hash-only ledger entries | secret never on disk (scan before/after empty); ledger payloads hash-only (no secret); raw-secret payload rejected `CALYX_LEDGER_SECRET_IN_PAYLOAD`; `RerankRequest` Debug redacts the secret; rerank score 0.42 |
| 23 | deterministic replay parity | `CALYX_DETERMINISM=1` replay of identical Forge quant + Sextant HNSW query (vs `=0`) | two `=1` runs byte-identical, max decoded delta ≤ 1e-3; `=0` does not claim determinism |
| 24 | whole-host loss DR drill | synthetic DR vault + ledger CF read via `AsterLedgerCfStore`; restic gated behind `CALYX_PH59_RESTIC_DR=1` | base row byte-exact, ledger chain `Intact`, restic drill skipped pending PH66 (see Gaps) |
| 25 | upgrade / format skew | open a major-1 vault, read old shard, append new shard, then craft an unknown major-99 manifest | old shards readable, new shard current format, same major after reopen, unknown major rejected `CALYX_FORMAT_VERSION_UNSUPPORTED` |

H22 spins up an in-process HTTP reranker test server (`TestServer`,
`spawn_reranker`) on `127.0.0.1:0` returning `{"scores":[0.42]}`.

### A.5 The integrated soak (`soak.rs`, `soak/ops.rs`) — the public library

This is the only `pub` surface of the crate.

#### A.5.1 Public constants (`soak.rs`)

| Constant | Value | Meaning |
|---|---|---|
| `DEFAULT_SOAK_OPS` | `10_000_000` | default op count |
| `DEFAULT_SOAK_SEED` | `0xCA1A_0059` | default RNG seed |
| `SAMPLE_EVERY` | `5_000` | sample RSS/VRAM every N ops |

Internal (`const`) tuning: `DIM=32`, `MEMTABLE_BYTES=64 MiB`,
`MAX_PINNED_GAP_SEQS=25_000`, `VRAM_SOFT_CAP_BYTES=512 MiB`, `KEY_SPACE=16_384`,
`ANN_INDEX_CAP=65_536`, `WAL_SEGMENT_BYTES=256 KiB`, `WAL_BATCH_RECORDS=256`,
`WAL_RECYCLE_EVERY_GC_TICKS=2_000`, `GC_SWEEP_EVERY_GC_TICKS=20_000`,
`MAX_OSCILLATION_REVERSALS=6`, `TOMBSTONE_OSCILLATION_MIN_SWING=0.02`,
`PINNED_GAP_OSCILLATION_MIN_SWING=512.0`.

#### A.5.2 Public functions

| Function | Signature | Notes |
|---|---|---|
| `run_integrated_soak` | `(n_ops: u64, seed: u64) -> Result<SoakReport, String>` | chooses root from env `PH59_FINAL_SOAK_ROOT` or `temp_dir()/calyx-ph59-final-soak-<seed hex>`, then delegates |
| `run_integrated_soak_at` | `(root: &Path, n_ops: u64, seed: u64) -> Result<SoakReport, String>` | the real driver |
| `write_soak_artifacts` | `(root: &Path, report: &SoakReport) -> Result<Vec<u8>, String>` | writes `ph59_final_soak.json` to `root` and `<repo>/target` |

#### A.5.3 Public types

`SoakSample` (serialize): `op: u64`, `rss_kib: u64`, `vram_mib: u64`,
`tombstone_ratio: f64`, `wal_bytes_active: u64`, `oldest_pinned_seq_gap: u64`.

`SoakCounts` (default/serialize): `writes`, `reads`, `ann_searches`,
`gc_ticks`, `vram_dispatches`, `anneal_ticks` (all `u64`).

`SoakReport` (serialize) fields:

| Field | Type | Meaning |
|---|---|---|
| `op_count` | `u64` | ops executed |
| `seed` | `u64` | RNG seed |
| `sample_every` | `u64` | = `SAMPLE_EVERY` |
| `key_space` | `u64` | = `KEY_SPACE` |
| `ann_index_cap` | `usize` | = `ANN_INDEX_CAP` |
| `wal_segment_bytes` | `u64` | = `WAL_SEGMENT_BYTES` |
| `wal_records_flushed` | `u64` | final durable WAL seq |
| `counts` | `SoakCounts` | per-op-type tallies |
| `trend_bytes_per_op` | `f64` | RSS slope over last quarter of samples |
| `vram_trend_bytes_per_op` | `f64` | VRAM slope (same window) |
| `rss_max_mib` | `u64` | max sampled RSS (KiB→MiB) |
| `vram_max_mib` | `u64` | max sampled VRAM |
| `soft_cap_mib` | `u64` | = 512 |
| `rss_bounded` | `bool` | `trend_bytes_per_op < 1.0` |
| `vram_bounded` | `bool` | `vram_max ≤ soft cap` |
| `oldest_pinned_seq_gap_bounded` | `bool` | all samples ≤ `MAX_PINNED_GAP_SEQS` |
| `soak_oscillation_detected` | `bool` | hysteresis reversals on tombstone-ratio or pinned-gap exceed 6 |
| `max_gap_seqs` | `u64` | max pinned-gap seen |
| `final_tombstone_ratio` | `f64` | last sample ratio |
| `wal_bytes_active_final` | `u64` | last sample WAL bytes |
| `samples` | `Vec<SoakSample>` | full sample series |
| `target_files` | `Vec<String>` | sorted relative file list under root |
| `elapsed_ms` | `u128` | wall time |
| `panic_free` | `bool` | always `true` if it returned |

#### A.5.4 The soak loop algorithm (`run_integrated_soak_at`)

1. Create `root`, seed a `SmallRng` from `seed`, open a `CfRouter` (vault) with a
   64 MiB memtable, a `Wal` (256 KiB segments, zero group-commit window), a
   `WalRecycler`, an `HnswIndex` (slot 59, dim 32), a `VramBudgeter` (512 MiB
   soft cap, `StaticVram` reporting 64 GiB free), a `FixedClock`, and a
   `BudgetEnforcer` (`StaticBudget`: 5% CPU, 0 VRAM, NVML "available").
2. Take an initial sample (op 0).
3. For each op `0..n_ops`, draw `rng.gen_range(0..100)` and dispatch by band:

   | Range | Op | Weight |
   |---|---|---|
   | 0–39 | `write_op` | 40% |
   | 40–64 | `read_op` | 25% |
   | 65–79 | `ann_search_op` | 15% |
   | 80–89 | `gc_tick_op` | 10% |
   | 90–94 | `vram_dispatch_op` | 5% |
   | 95–99 | `anneal_tick_op` | 5% |

4. Every `SAMPLE_EVERY` ops: flush the WAL batch and push a sample (RSS via
   `heap_rss_bytes`, VRAM via budgeter stats, running tombstone ratio, and a
   synthetic decreasing pinned gap `10_000 - op/SAMPLE_EVERY`).
5. After the loop: flush pending, flush WAL, compute the **physical** tombstone
   ratio by scanning the on-disk inventory, push a final sample, and assemble
   the report.

Per-op details (`soak/ops.rs`): `write_op` puts a base row (length
`64 + op%128`), a tombstone every 4th op, inserts into the HNSW index while
`live_len < ANN_INDEX_CAP`, and batches WAL payloads (flush at 256). `gc_tick_op`
recycles WAL every 2000 GC ticks and runs a compaction-GC sweep every 20000.
`anneal_tick_op` returns `Err("anneal budget handle leak detected")` if any
budget handle stays active after a tick (leak invariant).

**Oscillation detection** (`oscillates`): counts hysteresis reversals (direction
flips exceeding a min swing) in the tombstone-ratio series (min swing 0.02) and
the pinned-gap series (min swing 512.0); oscillation is declared if either count
exceeds `MAX_OSCILLATION_REVERSALS` (6). Unit tests in `soak.rs` confirm bounded
jitter and monotone cleanup are NOT flagged, while sawtooths ARE.

**Trend/slope** (`slope`): ordinary least-squares slope of y vs. op over the last
quarter of samples; `rss_bounded` requires the RSS slope `< 1.0` bytes/op.

### A.6 Benchmark (`benches/bench_hazard_soak_throughput.rs`)

Criterion bench `bench_hazard_soak_throughput` runs `run_integrated_soak_at`
with `BENCH_OPS = 10_000` and `DEFAULT_SOAK_SEED`, throughput measured in
elements (ops). It **returns immediately (no-op) on non-Linux** (`cfg!(target_os
= "linux")` guard). Criterion config: 1 s warm-up, 10 s measurement, sample size
10.

### A.7 How it is run

```
calyx-hazard-soak [--hazards <range> | --all-hazards] [--seed <value>] [--ops <n>]
```

Exit code 0 on pass, 1 on fail. Output directory comes from env
(`PH59_*_ROOT` / `CALYX_FSV_ROOT`) or a temp dir. Several probes depend on Linux
tools (`dmesg`, `nvidia-smi`, `restic`, `sh`); these degrade to
"unavailable"/`None` off-Linux rather than failing hard, except where a probe
asserts on their output.

---

## Part B — calyx-testkit

### B.1 Overview

`calyx-testkit/src/lib.rs` (217 LOC, the crate's only source file) has the
module doc `//! Reusable deterministic test scaffolding for Calyx crates.` It
provides deterministic seeds/clock builders and a set of `proptest` strategies
for `calyx-core` types. Dependencies: `calyx-core`, `proptest`, `rand`, `serde`,
`serde_json`. No binary, no features.

### B.2 Public constants

| Constant | Type | Exact value |
|---|---|---|
| `DEFAULT_TEST_SEED` | `u64` | `0xCA1A_CAFE_D15C_1A11` |
| `DEFAULT_TEST_TS` | `Ts` (`u64`) | `1_785_500_000` |

### B.3 Public builder functions

| Function | Signature | Behavior |
|---|---|---|
| `seeded_rng` | `(seed: u64) -> StdRng` | `StdRng::seed_from_u64(seed)` — deterministic RNG |
| `fixed_clock` | `() -> FixedClock` | `FixedClock::new(DEFAULT_TEST_TS)` — the standard fixed test clock |

The seed and timestamp helpers are the requested constants: default seed
`0xCA1A_CAFE_D15C_1A11`, fixed timestamp `1_785_500_000`. `seeded_rng` builds an
RNG; `fixed_clock` builds the clock.

### B.4 Public proptest strategies

All return `BoxedStrategy<…>`:

| Function | Strategy yields |
|---|---|
| `slot_id_strategy()` | `SlotId` from `any::<u16>()` via `SlotId::new` |
| `cx_id_strategy()` | `CxId` from 16 arbitrary bytes via `CxId::from_bytes` |
| `modality_strategy()` | one of all 10 `Modality` variants: Text, Code, Image, Audio, Video, Protein, Dna, Molecule, Structured, Mixed |
| `anchor_kind_strategy()` | `AnchorKind`: TestPass, TieFormed, Thumbs, `Label("[a-z]{1,8}")`, Reward, SpeakerMatch, StyleHold, Recurrence |
| `absent_reason_strategy()` | `AbsentReason`: NotApplicable, Redacted, LensUnavailable, Deferred, LensInactive, `Error("[A-Z_]{1,16}")` |
| `slot_vector_strategy()` | `SlotVector::Dense{dim, data}` (0–3 values, each `u8 0..=10` scaled `/10.0`) or `SlotVector::Absent{reason}` |
| `small_constellation_strategy()` | a full `Constellation` (see below) |

`small_constellation_strategy()` builds a `Constellation` from a `cx_id`,
modality, `panel_version` (1..16), a `redacted: bool`, and a slot vector. When
not redacted it inserts one slot (id 1), a `Reward` anchor (value `1.0`,
source `"testkit"`, confidence `1.0`), and an input pointer
`"zfs://calyx/testkit/input"`; `created_at`/`observed_at` use `DEFAULT_TEST_TS`;
`input_ref.hash = [3;32]`, `provenance = LedgerRef{seq:1, hash:[4;32]}`, and
`flags.ungrounded = flags.redacted_input = redacted`. The private helper
`test_vault_id()` parses the fixed ULID `01ARZ3NDEKTSV4RRFFQ69G5FAV` into a
`VaultId`.

### B.5 Tests in the crate

The `#[cfg(test)] mod tests` block asserts the contracts: `seeded_rng` replays
identical bytes for the same seed; `fixed_clock()` equals
`FixedClock::new(DEFAULT_TEST_TS)`; and proptest roundtrips for `SlotId`
display/parse, `Constellation` serde, and `SlotVector::Absent` staying absent.

---

## Gaps / not covered

- **`calyx-hazard-soak/src/lib.rs` has no module doc**; the crate's purpose is
  inferred from `soak.rs`, the probes, and PH59 issue/task naming in `main.rs`.
- **H24 (DR drill) is intentionally incomplete**: the restic restore is gated
  behind `CALYX_PH59_RESTIC_DR=1` and "skipped pending PH66"; the probe passes
  on the skip path (`dr_restore_verified: false`). The full restore drill is
  not implemented here.
- **Platform dependence**: the benchmark is a no-op off Linux; `dmesg`,
  `nvidia-smi`, and `restic` invocations degrade to "unavailable" rather than
  exercising the real tools on non-Linux hosts.
- The hazard probes (H1–H25) are reachable **only via the binary**, not the
  library API; only the `soak` module is `pub`.
- The CUDA path in H9 (`nan_guard_code`) is only compiled with the `cuda`
  feature; the default build uses the CPU TurboQuant guard.
