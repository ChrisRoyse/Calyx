# 13. Anneal Self-Improvement (calyx-anneal)

`calyx-anneal` implements Stage 10+ reversible self-optimization: tripwire-guarded shadow rollback, self-heal (fault detection / degrade / rebuild / recalibrate / restore), mistake-closure and online heads, autotune loops (bandit / A/B / soak / per-scope tuners), lens & operator proposal, the J-objective intelligence gradient, growth curve, and Goodhart / sufficiency guards.

The crate root declares the public contract in `src/lib.rs:1` (`//! Anneal self-optimization contracts for reversible tuning loops.`). It is organized into 13 top-level modules (`src/lib.rs:3-15`).

> Scope note: This document is derived from source only. Where a fact could not be determined from the read source, it is marked "Not determined from source". The crate is ~39.8K LOC across 151 files; the source-of-truth files for each subsystem are listed per section.

## Source files covered

- Root: `src/lib.rs`
- Tripwire: `src/tripwire.rs`
- Shadow rollback: `src/shadow.rs`, `src/rollback.rs`, `src/rollback_codec.rs`, `src/integration_fsv.rs`
- Budget: `src/budget.rs`
- Ledger: `src/ledger_anneal.rs`
- J-objective: `src/j/mod.rs`, `src/j/j_composite.rs`, `src/j/gradient.rs`, `src/j/goodhart.rs`, `src/j/growth_curve.rs`, `src/j/intelligence_report.rs`
- Self-heal: `src/heal/mod.rs`, `src/heal/degrade.rs`, `src/heal/triggers.rs` (+`triggers/support.rs`), `src/heal/rebuild.rs` (+`rebuild/{scheduler,builders,source,artifact}.rs`), `src/heal/recalibrate.rs` (+`recalibrate/{types,tau,lens,store}.rs`), `src/heal/restore.rs` (+`restore/{checksum,barrier,alert}.rs`)
- Learn: `src/learn/mod.rs`, `src/learn/mistake_log.rs`, `src/learn/replay_buffer.rs`, `src/learn/frozen_guard.rs`, `src/learn/regression_assert.rs`, `src/learn/outcome.rs` (+`outcome/queue.rs`), `src/learn/online_head.rs` (+`online_head/{codec,storage,update,regression,sleep_pass}.rs`)
- Tune: `src/tune/mod.rs`, `src/tune/bandit.rs`, `src/tune/ab_runner.rs` (+subs), `src/tune/soak_harness.rs` (+subs), `src/tune/scope_{forge,index,loom,storage}.rs` (+subs)
- Propose: `src/propose/mod.rs`, `src/propose/propose_lens.rs`, `src/propose/deficit_localize.rs`, `src/propose/differentiation_gate.rs`, `src/propose/candidate_synth.rs` (+`targets.rs`), `src/propose/operator_synth.rs` (+`gate,codec,storage`), `src/propose/admission_record.rs`, `src/propose/registry_hot_add.rs`
- Recurrence: `src/recurrence_schedule.rs`

## 13.1 Module map

| Module | File | Responsibility |
|---|---|---|
| `tripwire` | `src/tripwire.rs` | Hysteretic threshold guards on 5 quality metrics |
| `shadow` | `src/shadow.rs` | Held-out replay shadow execution; Promote/Revert verdict |
| `rollback` / `rollback_codec` | `src/rollback.rs`, `src/rollback_codec.rs` | Prepared change snapshots, live-pointer swap, promote/rollback/commit |
| `integration_fsv` | `src/integration_fsv.rs` | `AnnealSubstrate` orchestration: prepare→shadow→ledger→promote/revert |
| `budget` | `src/budget.rs` | Background CPU/VRAM budget enforcer with RAII handles |
| `ledger_anneal` | `src/ledger_anneal.rs` | Hash-only Anneal audit ledger (EntryKind::Anneal) |
| `j` | `src/j/*` | J-objective, intelligence gradient, Goodhart, growth curve, report |
| `heal` | `src/heal/*` | Fault detection, degrade registry, rebuild, recalibrate, restore |
| `learn` | `src/learn/*` | Mistake log, replay buffer, online heads, outcome, sleep pass, regression guard |
| `tune` | `src/tune/*` | Bandit, A/B runner, soak harness, per-scope tuners |
| `propose` | `src/propose/*` | Deficit localization, lens/operator synthesis, differentiation gate, hot-add |
| `recurrence_schedule` | `src/recurrence_schedule.rs` | Refresh priority / retention tier from recurrence cadence |

All persistent state uses Aster column families: `AnnealRollback`, `AnnealGrowth`, `AnnealReport`, `AnnealHealth`, `AnnealChecksums`, `AnnealMistakes`, `AnnealReplay`, `AnnealHeads`, `AnnealBandit`, `AnnealSoak`, `AnnealOperators`, `Online`, and `Ledger`. Config/state TOML+JSON live under `<vault>/.anneal/`.

---

## 13.2 Tripwires (`src/tripwire.rs`)

Tripwires are hysteretic threshold guards over exactly five metrics (`METRICS`, `src/tripwire.rs:16`):

| Metric (`TripwireMetric`) | TOML key | Default bound (`default_bound`) | Direction (`default_direction`) |
|---|---|---|---|
| `RecallAtK` | `recall_at_k` | 0.90 | `Below` |
| `GuardFAR` | `guard_far` | 0.01 | `Above` |
| `GuardFRR` | `guard_frr` | 0.05 | `Above` |
| `SearchP99` | `search_p99` | 200.0 | `Above` |
| `IngestP95` | `ingest_p95` | 500.0 | `Above` |

A `TripwireThreshold { bound, hysteresis, direction }` carries a default hysteresis of `bound * DEFAULT_HYSTERESIS_FRACTION` where `DEFAULT_HYSTERESIS_FRACTION = 0.05` (`src/tripwire.rs:13`, `:215`). Config is persisted to `<vault>/.anneal/tripwire.toml` (`CONFIG_DIR=".anneal"`, `CONFIG_FILE="tripwire.toml"`, `tripwire_config_path`, `:192`). `load_from_vault` writes defaults if the file is absent (`:96-106`).

### Tripwire detection algorithm — `TripwireRegistry::check` (`src/tripwire.rs:108`)

1. Reject non-finite `value` with `CALYX_TRIPWIRE_INVALID_METRIC` (`:109`).
2. Fetch the metric's threshold; missing → `CALYX_TRIPWIRE_INVALID_CONFIG` (`:112-115`).
3. Update per-metric `ThresholdState { last_value, crossed }` (`:116-121`).
4. Recompute `crossed` with hysteresis via `threshold_crossed` (`:344`):
   - `Below`, not yet crossed: `value < bound − ε`
   - `Below`, already crossed: `value < bound + hysteresis − ε`
   - `Above`, not yet crossed: `value > bound + ε`
   - `Above`, already crossed: `value > bound − hysteresis + ε`
   - `TRIPWIRE_EPSILON = 1e-12` (`src/tripwire.rs:14`).
5. Return `TripwireResult::Crossed { metric, threshold, hysteresis }` or `TripwireResult::Ok`.

`set_tripwire` validates a candidate threshold (`validate_threshold`, `:315`): bound and hysteresis must be finite and ≥ 0; the direction must match `default_direction(metric)`; for `Below` metrics, hysteresis must not exceed bound. All five metrics must be present (`ensure_all_metrics_present`, `:301`).

**Error codes:** `CALYX_TRIPWIRE_INVALID_METRIC`, `CALYX_TRIPWIRE_INVALID_CONFIG`.

---

## 13.3 Shadow execution & verdict (`src/shadow.rs`)

Shadow execution replays held-out queries against a candidate and incumbent action, comparing per-metric averages and checking tripwires before deciding Promote vs Revert.

Key types: `ReplayQuery { query_id, query_vector, expected_top_k }`, `HeldOutReplay { queries, seed }`, `ReplayAnchor { cx_id, similarity }`. `HeldOutReplay::sample(queries, n, seed)` deterministically shuffles with `ChaCha8Rng::seed_from_u64(seed)` and truncates to `n` (`src/shadow.rs:41`). `build_replay` constructs from a `ReplaySource` (`:53`).

Actions implement `AnnealAction::apply_shadow(query) -> ActionMetricSnapshot` (`:113`). The same `SHADOW_METRICS` set of 5 is used (`:11`).

### Shadow rollback algorithm — `ShadowExecutor::run_shadow` (`src/shadow.rs:139`)

1. `evaluated_at = clock.now()`. If replay empty → `Revert { InsufficientReplay }` (`:144`).
2. For each query: consume one budget tick (`budget.try_consume()`); if exhausted → `Revert { BudgetExhausted }` with partial metrics (`:153`).
3. Apply candidate and incumbent; accumulate via `MetricAccumulator::add_query`. Missing metric → `Revert { MissingMetric { metric, side } }`; non-finite → `Revert { InvalidMetric { metric, side } }` (`:251-288`).
4. Build per-metric mean comparison (`candidate_total/count`, `incumbent_total/count`, `:290`).
5. For each comparison: run `registry.check(metric, candidate_value)`:
   - `Crossed` → `Revert { TripwireCrossed(metric) }`
   - `Err` → `Revert { TripwireError { metric, code } }`
   - then `regressed(comparison)` → `Revert { MetricRegression(metric) }` (`:170-198`).
6. If all pass → `Promote { metrics }`.

`regressed` (`:333`): for `RecallAtK`, candidate worse if `candidate + ε < incumbent`; for the four `Above` metrics, worse if `candidate > incumbent + ε`. `COMPARE_EPSILON = 1e-12` (`:19`).

`ShadowVerdict` = `Promote { metrics } | Revert { reason, metrics }`. `ShadowRevertReason` variants: `TripwireCrossed`, `MetricRegression`, `BudgetExhausted`, `InsufficientReplay`, `MissingMetric`, `InvalidMetric`, `TripwireError` (`:218-235`).

---

## 13.4 Rollback store & change state machine (`src/rollback.rs`, `src/rollback_codec.rs`)

`RollbackStore<S: RollbackStorage>` keeps `ArtifactSnapshot` records and per-key live pointers in `ColumnFamily::AnnealRollback` (`AsterRollbackStorage`, `src/rollback.rs:72-108`).

`ArtifactKey` (live target) = `ConfigCache([u8;32]) | HnswGraph([u8;32]) | QuantLevel([u8;32])`; `ArtifactPtr` (value) = `ConfigCacheKeyHash([u8;32]) | HnswGraphPath(String) | QuantLevelRecordHash([u8;32])` (`:29-41`).

`ArtifactSnapshot { change_id, key, prior_ptr, candidate_ptr, ts, description, promoted, reverted, committed }` (`:44`).

### Change state machine

States are encoded by the `(promoted, reverted, committed)` flags on a snapshot.

| State | Flags | Entered by | Effect on live pointer |
|---|---|---|---|
| Prepared | all false | `prepare` / `prepare_with_description` (`:156-187`) | unchanged (prior_ptr) |
| Promoted | `promoted=true` | `promote` (`:189`) | live ← candidate_ptr |
| Reverted | `reverted=true` | `rollback` (`:214`) | live ← prior_ptr |
| Committed | `committed=true` | `commit` (`:236`) | frozen; no further promote/rollback |

Transition guards:
- `promote` errors `CALYX_ANNEAL_CHANGE_COMMITTED` if committed, and `CALYX_ANNEAL_INVALID_ROLLBACK_STATE` if already reverted (`:192-197`).
- `rollback` errors `CALYX_ANNEAL_CHANGE_COMMITTED` if committed (`:217`).
- `prepare` requires an existing live pointer (`install_live_ptr` first), else `CALYX_ANNEAL_INVALID_ROLLBACK_STATE` (`:167-171`).
- Unknown id → `CALYX_ANNEAL_UNKNOWN_CHANGE_ID` (`:325`).

**Change-id allocation** (`allocate_id`, `:300`): `next = max(ts*ID_BUCKET + seed%ID_BUCKET + counter, last_id+1)` with `ID_BUCKET = 1_000_000` (`:20`); monotonic, saturating.

**Codec** (`src/rollback_codec.rs`): snapshot key `b"change:" + change_id.to_be_bytes()`; live key `b"live:" + encode_artifact_key`. Magic tags `ARS1` (snapshot), `ARL1` (live) (`:7-24`).

---

## 13.5 Substrate orchestration (`src/integration_fsv.rs`)

`AnnealSubstrate<'a, R, L, C, P>` bundles `tripwires`, `replay`, `rollback`, `ledger`, `budget`, and clock (`src/integration_fsv.rs:68`). Default shadow budget request: `DEFAULT_SHADOW_CPU_WEIGHT = 0.01`, `DEFAULT_SHADOW_VRAM_BYTES = 0` (`:17-18`), overridable via `with_budget_request`.

### Propose-change pipeline — `propose_change_with_actions_and_details` (`:180`)

1. `rollback.prepare_with_description(key, candidate_ptr, description)` → `change_id` (`:193`).
2. `rollback.readback(change_id)` for ledger hashing (`:196`).
3. `shadow_verdict(candidate, incumbent)` (`:326`): acquire shadow budget; if `CALYX_ANNEAL_BUDGET_EXHAUSTED` → `Revert { BudgetExhausted }`; else run `ShadowExecutor::run_shadow`.
4. On `Promote`: write the promote ledger entry (with metrics + optional details), `rollback.promote(change_id)`, return `ChangeOutcome::Promoted(change_id)` (`:199-211`).
5. On `Revert`: `rollback.rollback(change_id)`, write the revert ledger entry, return `ChangeOutcome::Reverted { reason, change_id }` (`:212-224`).

`ChangeOutcome = Promoted(ChangeId) | Reverted { reason, change_id }` (`:21`). Default action pair is `(Promote, Revert)`; callers may pass custom `AnnealLedgerActionPair` and JSON details. Other helpers: `rollback_explicit_with_action`, `write_sleep_pass_deferred`, `write_outcome_event_with_details`, `status` (returns tripwire states + budget + last 16 ledger entries). Ledger pointer hashes: `ArtifactPtr::HnswGraphPath` hashed via `full_content_hash`; the others carry their own `[u8;32]` (`:393-398`). Error code `CALYX_LEDGER_WRITE_FAIL` wraps ledger failures (`:404`).

---

## 13.6 Budget enforcer (`src/budget.rs`)

`BudgetConfig { cpu_fraction, vram_bytes, tick_interval_ms }` defaults: `cpu_fraction = 0.15`, `vram_bytes = 512 MiB` (`512*1024*1024`), `tick_interval_ms = 100` (`src/budget.rs:15-17,27`). Validated: `cpu_fraction` finite in `0.0..=1.0`; `tick_interval_ms > 0` (`:48`). Persisted to `<vault>/.anneal/budget.toml`. Background nice value `BACKGROUND_NICE = 10` (`:11`).

`BudgetEnforcer::acquire(cpu_weight, vram_bytes)` (`:194`):
1. Validate request; tick the CPU/VRAM probe.
2. If `cpu_fraction ≤ ε` or `vram_bytes == 0` → `CALYX_ANNEAL_BUDGET_EXHAUSTED` ("zero capacity").
3. `projected_cpu = sampled_cpu + reserved_cpu + cpu_weight`; if `> cpu_fraction + ε` → exhausted (`:206`).
4. `projected_vram` saturating sum; if `> vram_bytes` → exhausted (`:209`).
5. Reserve, increment `handles_active`, return `BudgetHandle` with `handle_ticks = ceil(1000/tick_interval_ms)` cooperative ticks (`:372`).

`BudgetHandle` releases its reservation on `Drop` (RAII, `:143`). `BudgetHandle::new(ticks)` makes an unreserved test/shadow handle. `ProcStatBudgetProbe` samples CPU from `/proc/stat` (returns 1.0 if unreadable; `nvml_available=false`, vram 0) (`:264-302`). Warning `CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE` set when NVML absent (`:190`). Errors: `CALYX_ANNEAL_BUDGET_EXHAUSTED`, `CALYX_ANNEAL_BUDGET_INVALID_CONFIG`, `CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE`.

---

## 13.7 Anneal ledger (`src/ledger_anneal.rs`)

Hash-only audit entries written as `EntryKind::Anneal` to the shared `Ledger` CF. Payload tag `ANNEAL_LEDGER_PAYLOAD_TAG = "anneal_event_v1"`; max payload `MAX_ANNEAL_LEDGER_PAYLOAD_BYTES = 16 KiB` (`src/ledger_anneal.rs:14-15`).

`AnnealLedgerEntry` fields: `action, change_id, artifact_id, prior_ptr_hash[32], candidate_ptr_hash[32], metrics: MetricSnapshot, ts, description, fault, proposal, details, prev_hash` (`:107`). `write` enforces the prev_hash chain against the appender tip (`CalyxError::ledger_chain_broken` on mismatch, `:147-156`).

`AnnealLedgerAction` (32 variants, `:25-62`): `Promote, Revert, Propose, LensAdmitted, LensRejected, Park, DegradeChange, FaultEvent, Rebuild, BaseCorruptAlert, BaseRestored, Recalibrate, TauRecalibrated, TauRecalibrationReverted, LensPark, LensUnpark, MistakeUpdate, HeadUpdate, HeadUpdateReverted, OperatorPromoted, OperatorReverted, SleepPassDeferred, OutcomeReward, OutcomeContradiction, AutotuneAB, AutotuneAbandoned, AutotunePromote, GoodhartPassed, GoodhartFailed`.

Errors: `CALYX_LEDGER_ENTRY_TOO_LARGE`, `CALYX_ANNEAL_LEDGER_INVALID_ENTRY`, `CALYX_ASTER_CF_UNAVAILABLE`. `AsterAnnealLedgerStore` is an append-only adapter over `ColumnFamily::Ledger`.

---

## 13.8 The J-objective (`src/j/j_composite.rs`)

J is a weighted balance of 8 grounded positive terms minus 4 penalties. `JTerms` (`src/j/j_composite.rs:14`) and `JWeights { w1..w8 }` (default all 1.0, `:52`) drive `compute_j` (`:194`).

### J formula (`compute_j`, `src/j/j_composite.rs:273-284`)

```
weighted_positive = w1·info + w2·n_eff + w3·sufficiency
                  + w4·kernel_recall + w5·oracle_accuracy
                  + w7·compression + w8·coverage
weighted_negative = w6·mistake_rate + p_redundant + p_ungrounded + p_goodhart
J = weighted_positive − weighted_negative
```

| Term | Source method | Notes |
|---|---|---|
| `w1_info` | `mutual_info_panel_anchor` | clamped to `dpi_ceiling`; **zeroed if any provisional anchors present** (`:233`) |
| `w2_n_eff` | `n_eff` | grounded (generated credit excluded) |
| `w3_sufficiency` | `panel_sufficiency(domain)` | clamped to `dpi_ceiling` |
| `w4_kernel_recall` | `kernel_recall` | |
| `w5_oracle_accuracy` | `oracle_accuracy` | |
| `w6_mistake_rate` | `mistake_rate` | negative term |
| `w7_compression` | `compression_yield` | |
| `w8_coverage` | `coverage` | |
| `p_redundant` | `redundancy_penalty(panel_len, n_eff)` | `max(panel_len − n_eff, 0)·REDUNDANCY_PENALTY` (`:354`), `REDUNDANCY_PENALTY = 1.0` |
| `p_ungrounded` | `ungrounded_penalty(provisional_excluded)` | `count·UNIT_PENALTY` (`:367`), `UNIT_PENALTY = 1.0` |
| `p_goodhart` | `context.goodhart_penalty` | injected from Goodhart state |

**Anti-Goodhart / anti-synthetic-recursion guards:**
- If `synthetic_recursion_credit_attempted()` returns true → `CALYX_ANNEAL_J_SYNTHETIC_RECURSION` (`:198`); generated/model-output signals cannot get positive credit.
- `generated_positive_credit()` is **subtracted** from each measured input via `exclude_generated_credit` (errors if generated > measured) (`:218-267,358`).
- All metric inputs must be finite and non-negative (`validate_nonnegative`, `:377`).
- `dpi_headroom = min(dpi_ceiling − grounded_info, dpi_ceiling − grounded_sufficiency)` (`:294`) — Data Processing Inequality headroom.
- Non-finite computed J → `CALYX_ANNEAL_J_INVALID_METRIC`.

Weights persist to `<vault>/.anneal/j_weights.toml` (`set_objective_weights`/`read_objective_weights_from_vault`, `:300-352`). `JObjectiveContext { domain, panel_len, weights, goodhart_penalty }`; `DEFAULT_J_DOMAIN = "default"`. Errors: `CALYX_ANNEAL_J_INVALID_METRIC`, `CALYX_ANNEAL_J_INVALID_CONFIG`, `CALYX_ANNEAL_J_SYNTHETIC_RECURSION`.

---

## 13.9 Intelligence gradient (`src/j/gradient.rs`)

`IntelligenceGradient` is a max-heap of candidate actions ranked by `dj_per_cost` (`src/j/gradient.rs:177`). A `GradientCandidate { action, cost_budget_units }` becomes a `GradientEntry` with `dj_per_cost = estimated_dj / cost` (or `+∞` if cost 0) (`:126-130`). Ordering compares `dj_per_cost.total_cmp` then earlier sequence wins (`:169-175`).

`CandidateAction` variants and their weight mapping (`weighted_estimated_dj`, `:88-101`): `ProposeLens`→w1, `LabelAnchor`→w1, `PruneRedundantLens`→w2, `RecalibrateHeal`→w5, `RecomputeKernel`→w4, `MaterializeCrossTerm`→w1, `RetuneMath`→w7. `estimate_dj` validates weights and non-negativity (`:286`).

`refresh(candidates)` clears the heap, drops candidates over `current_budget_units` (warning `CALYX_ANNEAL_GRADIENT_OVER_BUDGET`), and pushes valid entries; returns `GradientRefreshReport { accepted, rejected }` (`:212-246`). `next_best_action` peeks the top; `snapshot(limit)` writes a `GradientSnapshot` (top readbacks, current J, budget, weights, warnings). Persisted to `<vault>/.anneal/gradient_queue.json` (`:354`). Errors: `CALYX_ANNEAL_GRADIENT_INVALID_METRIC`, `CALYX_ANNEAL_GRADIENT_INVALID_CONFIG`.

---

## 13.10 Goodhart guard (`src/j/goodhart.rs`)

`GoodhartChecker::check(before, after, lens_deltas)` runs three independent anti-gaming checks (`src/j/goodhart.rs:139`):

| Check | Method | Violation condition |
|---|---|---|
| Held-out regression | `check_held_out` (`:185`) | `j_heldout_delta ≤ held_out_min_gain_fraction · j_train_delta` |
| Gtau in-region | `check_gtau` (`:222`) | `in_region_frac < gtau_threshold` |
| Cross-lens anomaly | `check_cross_lens` (`:249`) | `abs(lens_delta / j_train_delta) > cross_lens_threshold` |

Defaults (`:16-19`): `DEFAULT_GTAU_THRESHOLD = 0.95`, `DEFAULT_CROSS_LENS_DOMINANCE_THRESHOLD = 0.80`, `DEFAULT_HELD_OUT_MIN_GAIN_FRACTION = 0.01`, `DEFAULT_GOODHART_VIOLATION_PENALTY_WEIGHT = 1.0`.

Penalty on failure: `p_goodhart_increment = max(abs(j_train_delta) · violation_penalty_weight, 0.0)` (`:155-159`). Held-out set must be `sealed` before validation (`CALYX_ANNEAL_GOODHART_INVALID_CONFIG` otherwise); empty held-out set is skipped with a warning (`:191-199`). Ward Gtau unavailable/error is treated as in-region 0.0 (warning, becomes a Gtau violation) (`:227-239`). `GoodhartState { p_goodhart }` persists to `<vault>/.anneal/goodhart_state.toml`; `add_goodhart_penalty_to_vault` accumulates increments. `record_goodhart_report` writes `GoodhartPassed`/`GoodhartFailed` ledger entries. Errors: `CALYX_ANNEAL_GOODHART_INVALID_METRIC`, `CALYX_ANNEAL_GOODHART_INVALID_CONFIG`.

`GoodhartViolation` = `HeldOutRegression { j_train_delta, j_heldout_delta } | GtauViolation { in_region_frac, threshold } | CrossLensAnomaly { anomalous_lens, delta_fraction }`.

---

## 13.11 Growth curve (`src/j/growth_curve.rs`)

`GrowthCurve<S: GrowthCf>` records J samples over time into `ColumnFamily::AnnealGrowth` (`AsterGrowthCf`, `src/j/growth_curve.rs:42`). `GrowthSample { ts, j, delta_j, n_queries_since_last, actions_taken }`; `delta_j = report.j − last.j` (`:142`). Tag `ANNEAL_GROWTH_TAG = "calyx_anneal_growth_v1"`; `DEFAULT_GROWTH_MAX_SAMPLES = 10_000`, `DEFAULT_GROWTH_WINDOW = 10` (`:11-13`). Samples beyond `max_samples` are trimmed FIFO (`:300`).

`record_sample` requires an `Available` report with finite J (`validate_report`, `:276`), else `CALYX_ANNEAL_GROWTH_INVALID_SAMPLE`. Row key = `ts.to_be_bytes() ++ seq.to_be_bytes()` (`anneal_growth_key`, `:238`).

**Growth-curve rising check** (`is_rising(window)`, `:163`): true iff `slope_recent(window) > 0 && latest_delta_j > 0`. `slope_recent` is the OLS slope of the last `window` J values via `linear_slope` (`:306`):

```
slope = (n·Σxy − Σx·Σy) / (n·Σx² − (Σx)²)   ; 0 if denominator == 0
```

`curve_summary` returns `{ samples_count, j_first, j_last, j_max, slope_recent, is_rising }`. `plot_ascii(width, height)` renders an ASCII J plot. Errors: `CALYX_ANNEAL_GROWTH_INVALID_CONFIG`, `CALYX_ANNEAL_GROWTH_INVALID_ROW`, `CALYX_ANNEAL_GROWTH_INVALID_SAMPLE`.

---

## 13.12 Intelligence report (`src/j/intelligence_report.rs`)

`intelligence_report(...)` calls `compute_j` and packages `IntelligenceReport { j, terms, weights, dpi_ceiling, dpi_headroom, provisional_excluded, gradient (top 5), next_best_action, goodhart_last, ts, availability }` (`src/j/intelligence_report.rs:76`). On `compute_j` error it produces an `Unavailable { code, message, remediation }` report with NaN terms (`:108,276`). `ReportAvailability = Available | Unavailable {…}`. Snapshots persist to `ColumnFamily::AnnealReport` keyed by `ts.to_be_bytes()`; `latest_intelligence_report_snapshot` returns the most recent *available* row (`:245`). `report_diff` computes per-term deltas and `delta_j`. Tag `ANNEAL_REPORT_TAG = "calyx_anneal_report_v1"`; error `CALYX_ANNEAL_REPORT_INVALID_ROW`. `format_report`/`to_json` render human and JSON views with per-term raw·weight contributions.

---

## 13.13 Self-heal (`src/heal/*`)

Self-heal is a five-stage fault-recovery framework: **degrade** (health state), **triggers** (detection), **rebuild**, **recalibrate**, **restore**. All mutate the `DegradeRegistry` health state machine and write ledger entries.

### 13.13.1 Component health state machine — `src/heal/degrade.rs`

`ComponentHealth` (`src/heal/degrade.rs:44`): `Ok | Degraded { since, reason } | Failing { since, reason } | Parked { since, reason }`. `excludes_lens()` is true for `Failing | Parked` (those lenses are dropped from routing) (`:82`). `ComponentKind` = `AnnIndex{slot_id} | KernelIndex{scope} | GuardProfile{slot_id} | LensEndpoint{lens_id} | BaseShard{shard_id}` (`:106`).

| Transition | Trigger | Method | Guard |
|---|---|---|---|
| Ok → Degraded/Failing/Parked | fault detected | `set_health` (`:236`) | — |
| Degraded → Ok | rebuild/recalibration confirmed | `confirm_healed` (`:249`) | bypasses confirm gate |
| Degraded → Ok (direct `set_health`) | — | `set_health` | **blocked** → `CALYX_ANNEAL_HEAL_CONFIRMATION_REQUIRED` (`:311`) |

`DegradeRegistry` persists rows to `ColumnFamily::AnnealHealth`, tag `ANNEAL_HEALTH_TAG = "anneal_health_v1"`. `route_lens_panel(panel)` returns a `LensRoute { requested, active, degraded }` excluding Failing/Parked lenses (`:275`). Errors: `CALYX_ANNEAL_HEAL_CONFIRMATION_REQUIRED`, `CALYX_ANNEAL_HEALTH_INVALID_ROW`, `CALYX_ASTER_CF_UNAVAILABLE`.

### 13.13.2 Fault detection — `src/heal/triggers.rs`

`FaultMonitor::run_once(registry, ledger)` iterates pluggable `FaultDetector`s under budget, emits `FaultEvent { component, fault_kind, recommendation, observed_at }`, transitions registry health, and writes `FaultEvent` ledger entries (`src/heal/triggers.rs:137`).

| Detector | Fault kind | Trigger condition | Resulting health (`health_transition`, `:80`) |
|---|---|---|---|
| `ChecksumDetector` | `Corruption` | SHA256(file) ≠ expected | Degraded |
| `LensProbeDetector` | `EndpointFailing` | consecutive failures ≥ `failure_threshold` (default 1); exp. backoff `2^min(n−1,8)` capped 256 | Failing |
| `TauDriftDetector` | `TauDrifted` | `sample.far > sample.tau + drift_tolerance` | Degraded |
| `SignalDecayDetector` | `SignalDecayed` | `bits_per_anchor < threshold_bits` (default 0.05) | Parked |
| `StaleDetector` | `StaleIndex` | `now − last_rebuild > rebuild_lag_bound_secs` | Degraded |
| (metric source error) | `MetricsUnavailable` | metrics read failed | Degraded |
| (probe panic) | `ProbeError` | `catch_unwind` caught | Failing |

Constants: `DEFAULT_SIGNAL_DECAY_BITS = 0.05`, `DEFAULT_PROBE_FAILURE_THRESHOLD = 1`, `MAX_BACKOFF_TICKS = 256`. `change_id` per fault is BLAKE3(component ++ fault_kind ++ observed_at). Error `CALYX_ANNEAL_FAULT_INVALID_EVENT`; budget exhaustion `CALYX_ANNEAL_BUDGET_EXHAUSTED`.

### 13.13.3 Rebuild — `src/heal/rebuild.rs` + `rebuild/*`

`RebuildScheduler` is a priority `BinaryHeap` of jobs (`RebuildPriority::{LOW=32, NORMAL=128, HIGH=224}`, `src/heal/rebuild.rs:54`). `RebuildTarget = AnnIndex{slot_id} | KernelIndex{scope} | GuardProfile{slot_id}`.

**`run_next` algorithm** (`scheduler.rs:91`):
1. Pop highest-priority job (empty → `NothingQueued`).
2. If component not `Degraded` → `SkippedNotDegraded`.
3. Acquire budget `REBUILD_CPU_WEIGHT = 0.01`, 0 VRAM; exhausted → requeue, `BudgetExhausted`.
4. Latest MVCC snapshot; lookup live ptr (missing → `Failed`).
5. Type-specific `Rebuilder::rebuild` reads **only** Base/Slot/Anchors CFs (whitelist enforced in `AsterRebuildSource::scan_cf`, else `CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION`).
6. `substrate.propose_change_with_description` (tripwire-shadowed; fixed metrics RecallAtK 0.95, FAR/FRR 0.001, P99 50, P95 80).
7. On Promote: `registry.confirm_healed`, write `Rebuild` ledger, `Completed`. On Revert: `Failed`.

`RebuildOutcome` = `Completed | Failed | BudgetExhausted | SkippedNotDegraded | NothingQueued`. Artifacts written atomically via tmp-rename (BLAKE3 content hash). Errors: `CALYX_ANNEAL_REBUILD_IO`, `CALYX_ANNEAL_REBUILD_INVALID_TARGET`, `CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED`, `CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION`, `CALYX_ASTER_SNAPSHOT_UNAVAILABLE`.

### 13.13.4 Recalibrate — `src/heal/recalibrate/*`

**Tau recalibration** (`trigger_tau_recalibration`, `recalibrate/tau.rs:17`): on a `TauDriftEvent`, looks up current tau, ensures the live tau pointer, acquires budget (`TAU_CPU_WEIGHT = 0.01`), calls `WardRecalibrate::recalibrate` → `NewTau { slot_id, tau∈[-1,1], far, frr, shadow_metrics }`, proposes a shadowed change ("tau recalibration"). On Promote: `tau_store.set_live_tau`, `registry.confirm_healed(GuardProfile)`, ledger `TauRecalibrated`. On Revert: ledger `TauRecalibrationReverted`. `RecalibrationOutcome = Promoted{…} | Reverted{…}`.

**Lens park/unpark** (`recalibrate/lens.rs`): `park_decayed_lens` requires `bits < SIGNAL_DECAY_FLOOR_BITS = 0.05` (else `CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET`), sets health `Parked`, writes `LensPark` ledger + JSONL alert. `unpark_lens` requires `bits ≥ 0.05` (else `CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET`), sets `Ok`, writes `LensUnpark`. `LensParkOutcome = Parked | AlreadyParked | Unparked | AlreadyOk`.

Tau store persists to `<vault>/.anneal/ward_tau.json`, tag `WARD_TAU_TAG = "ward_tau_v1"`. Errors: `CALYX_ANNEAL_TAU_INVALID`, `CALYX_WARD_RECALIBRATE_FAILED`, the two threshold codes above.

### 13.13.5 Restore — `src/heal/restore/*`

`verify_base_shards` recomputes SHA256 over each shard's key-range in the Base CF (`hash_rows`: length-prefixed key/value SHA256) and emits `BaseFaultEvent::Corrupt { shard, expected, actual, detected_at }` on mismatch (`restore/checksum.rs:50,158`). `alert_operator` writes a `BaseCorruptAlert` ledger entry + JSONL alert; `fail_reads_on_range` installs a read barrier (`barrier_installed=true`). `attempt_restore` runs the configured restore command if `auto_restore` (else `OperatorRequired`); success → `clear_reads_on_range` + `BaseRestored` ledger. Tag `BASE_SHARD_CHECKSUM_TAG = "anneal_base_shard_checksum_v1"`, CF `AnnealChecksums`. Errors: `CALYX_ANNEAL_CHECKSUM_INVALID_ROW`, `CALYX_ANNEAL_RESTORE_FAILED`, `CALYX_ANNEAL_ALERT_WRITE_FAILED`.

---

## 13.14 Learn: mistake-closure & online heads (`src/learn/*`)

### 13.14.1 Mistake log (`src/learn/mistake_log.rs`)

Append-only `MistakeEntry { cx_id, predicted, observed, anchor, ts, surprise }` where `surprise = |predicted − observed|` (`mistake_log.rs`). `mistake_rate(window)` = fraction of the last `window` entries with `surprise > high_surprise_threshold` (default `DEFAULT_MISTAKE_SURPRISE_THRESHOLD = 0.3`). CF `AnnealMistakes`, key = `seq.to_be_bytes()`, tag `anneal_mistake_v1`. Errors: `CALYX_ANNEAL_INVALID_WINDOW`, `CALYX_ANNEAL_MISTAKE_INVALID_ROW`, `CALYX_ANNEAL_MISTAKE_APPEND_ONLY`.

### 13.14.2 Replay buffer (`src/learn/replay_buffer.rs`)

Surprise-prioritized min-heap of `ReplayEntry { cx_id, surprise, mistake_ref, added_ts }`, default capacity `DEFAULT_REPLAY_CAPACITY = 4096`. `push` accepts if not full or `new.surprise > min.surprise` (replace). `sample_batch(n, seed)` does surprise-weighted sampling via `ChaCha8Rng` without replacement. Snapshot stored to CF `AnnealReplay` under fixed key `b"snapshot/v1"`. Errors: `CALYX_ANNEAL_INVALID_CAPACITY`, `CALYX_ANNEAL_REPLAY_INVALID_ROW`.

### 13.14.3 Online heads (`src/learn/online_head.rs` + subs)

Three heads (`HeadKind`): `Predictor` (linear), `Calibrator` (slope/intercept → sigmoid), `FusionWeights` (ensemble). `OnlineHead { kind, params, fisher_diag, version, prior_params }`; `MAX_ONLINE_HEAD_PARAMS = 1024`. CF `AnnealHeads`, keys `head/v1/{predictor,calibrator,fusion_weights}`, tag `anneal_online_head_v1`.

**Update rule** (`apply_update`, `online_head/update.rs`) — per-param SGD with Fisher L2 regularization:

```
gradient[i]        = mean_batch( (pred − target)·feature[i] )
observed_fisher[i] = mean_batch( ((pred − target)·feature[i])² )
regularizer[i]     = fisher_weight · fisher_diag[i] · (param[i] − prior[i])
param[i]          -= lr · (gradient[i] + regularizer[i])
fisher_diag[i]     = max(fisher_diag[i], observed_fisher[i])
```

Features per entry: `[1.0, entry.surprise, mistake_ref.surprise, seq/(seq+1), cx_id bytes/255…]`, scaled by head kind. `version` increments each update.

**Promotion gating** — `OnlineHeadState::update` (`online_head.rs:`): asserts no frozen-lens violation, computes candidate heads, proposes a shadowed change via `HeadPromotionGate`; on Promote persists & swaps, on Revert → `CALYX_ANNEAL_HEAD_UPDATE_REVERTED`. Post-update frozen-lens re-assert.

**Regression rollback** (`online_head/regression.rs`): `update_with_regression` computes candidate heads, runs `assert_no_regression` over the batch, proposes the change, then if `regression_rate(report) > config.max_regression_rate` calls `rollback_regressed_head_update` (ledger `HeadUpdateReverted`) and errors `CALYX_ANNEAL_REGRESSION_RECURRED`. `RegressionResult.recurred = new_surprise ≥ old_surprise`. `DEFAULT_MAX_REGRESSION_RATE = 0.05`; `RegressionConfig::strict()` = 0.0. Errors: `CALYX_ANNEAL_REGRESSION_{RECURRED,INVALID_CONFIG,SOURCE_UNAVAILABLE,NAN_PREDICTION}`.

**Sleep pass** (`online_head/sleep_pass.rs`) — offline batch-train state machine:
1. Validate config. If `degraded_components` non-empty → `record_sleep_pass_deferred`, `SleepPassOutcome::Deferred`.
2. Empty buffer / zero batch / empty sample → `Idle`.
3. `sample_batch(batch_size, seed)` then `update_with_regression`.
4. Promoted → `Promoted{update}`; no change → `Idle`; reverted/regressed error codes → `Reverted{…}`.

Defaults: `DEFAULT_SLEEP_PASS_BATCH_SIZE = 16`, `DEFAULT_SLEEP_PASS_MIN_SURPRISE = 0.01`. Outcomes `Idle | Deferred | Promoted | Reverted`. Error `CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG`.

### 13.14.4 Outcome / consequence recording (`src/learn/outcome.rs`)

`record_outcome(cx_id, anchor, prediction, context, config)` extracts a scalar outcome from the anchor, and:
- If a trusted prediction exists with `surprise ≥ contradiction_threshold` (default 0.3): **Contradiction** — records to mistake log + replay buffer (`OutcomeContradiction`), returns `RecordOutcomeContradiction`.
- Else **Reward**: `reward = clamp(observed·confidence, 0,1)`, `expected_delta_j = reward·max(1−surprise,0)`, `delta_j_per_cost = expected_delta_j/action_cost`; enqueues an `OutcomeQueueEntry`, trains the online head, returns `RecordOutcomeReward`.

Defaults: `DEFAULT_OUTCOME_ACTION_COST = 1.0`, `DEFAULT_OUTCOME_LR = 1.0`, `DEFAULT_OUTCOME_FISHER_WEIGHT = 0.0`. Outcome queue uses CF `Online` (`OnlineKeyKind::DeltaJQueue`), tag `anneal_outcome_delta_j_v1`. Errors: `CALYX_ANNEAL_OUTCOME_{INVALID_CONFIG,INVALID_ANCHOR,INVALID_ROW,APPEND_ONLY}`.

### 13.14.5 Frozen-lens guard (`src/learn/frozen_guard.rs`)

`FrozenLensGuard` snapshots SHA256 of frozen lens weights; `check()` reports `ok/violations/new_lenses`; `assert_no_violation()` errors if any frozen lens hash changed. Error `CALYX_REGISTRY_UNAVAILABLE`. This enforces that learning never mutates frozen lenses (a key anti-Goodhart invariant feeding `compute_j`'s grounded-input requirement).

---

## 13.15 Autotune (`src/tune/*`)

### 13.15.1 Bandit (`src/tune/bandit.rs`)

`ConfigBandit { policy, arms, incumbent_idx, hysteresis_wins, rng_seed }`. `BanditPolicy = EpsilonGreedy { epsilon } | Thompson`. `Arm { config, wins, trials, consecutive_wins }`.

**Arm selection** (`select_arm`): EpsilonGreedy picks a uniform-random arm with prob `epsilon` else best win-rate; Thompson samples `Beta(wins+1, losses+1)` per arm (Marsaglia–Tsang gamma) and picks the max. RNG advances deterministically (`ChaCha8`).

**Win/promotion** (`record_result`): increments `trials`; on win increments `wins` & `consecutive_wins`, on loss resets `consecutive_wins`; promotes the arm to incumbent when `consecutive_wins ≥ hysteresis_wins` (`DEFAULT_HYSTERESIS_WINS = 3`). CF `AnnealBandit`, key `bandit\0 ++ shape_key_hash`, tag `anneal_bandit_v1`. Errors: `CALYX_ANNEAL_BANDIT_{EMPTY,INVALID_CONFIG,INVALID_ROW}`.

### 13.15.2 A/B runner (`src/tune/ab_runner.rs`)

Paired trials of incumbent vs candidate over live queries. `ABTrial` lifecycle: **Init → Active (accumulating results) → Sealed (verdict)**. `record_query` consumes shadow budget (default cpu 0.01); after `min_samples` pairs (`DEFAULT_AB_MIN_SAMPLES = 100`) calls `declare_winner`.

**Verdict** (`declare_winner`): summarize P99 latency, mean recall, mean bits per arm. `candidate_won = faster && recall_ok(tripwire) && recall_regression_ok && latency_ok(tripwire) && bits_ok`. Feeds `bandit.record_result`; promotes only if the bandit's hysteresis makes the candidate incumbent. `ABVerdict = Promoted | Kept | Abandoned`. Ledger actions `AutotuneAB`/`AutotunePromote`/`AutotuneAbandoned`. Errors: `CALYX_ANNEAL_TRIAL_{ALREADY_ACTIVE,NOT_ACTIVE,INVALID_RESULT}`, `CALYX_ANNEAL_AB_CACHE_WRITE_FAIL`.

### 13.15.3 Soak harness (`src/tune/soak_harness.rs`)

Long-running (default `DEFAULT_SOAK_QUERIES = 1_000_000`, seed `0xABCDEF`) stress loop in `Seeded` or `LiveTraffic` mode. Runs the four scope tuners + an A/B trial per query, samples metrics every `DEFAULT_SOAK_SAMPLE_INTERVAL = 1_000`, and enforces a wall-clock budget (default 2 h → `CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED`).

**Gate** (`gate_passed`): `p99_reduction ≥ DEFAULT_SOAK_P99_TARGET_REDUCTION (0.20)` **and** `recall_final ≥ max(min_recall, recall_baseline)` **and** `!oscillation_detected`.

**Oscillation check** (`check_oscillation`, window `DEFAULT_SOAK_OSCILLATION_WINDOW = 10_000`): true if any in-window sample's `p99_ns > prev.p99_ns · 1.05` (>5% jump). CF `AnnealSoak`, tag `anneal_soak_v1`. Errors: `CALYX_ANNEAL_SOAK_{INVALID_CONFIG,INVALID_ROW,TIME_BUDGET_EXHAUSTED}`.

### 13.15.4 Per-scope tuners

All four share the bandit/win-check/promotion pattern; each generates ≤8 candidate configs and promotes via shadowed change.

| Scope | File | Config knobs | Win check | Recall target | Change-id base |
|---|---|---|---|---|---|
| Forge (GEMM) | `scope_forge.rs` | tile_m/n/k, dtype, batch | `latency < baseline && recall ≥ baseline` | `DEFAULT_FORGE_RECALL_TARGET = 0.99` | 413_000 |
| Index (ANN) | `scope_index.rs` | hnsw_ef/m, diskann_beam, spann_cutoff, quant_bits | latency↓ & recall≥ & `quant_win_check`; skips parked slots | `DEFAULT_INDEX_RECALL_TARGET = 0.99` | 414_000 |
| Loom (materialization) | `scope_loom.rs` | eager_pairs, indexed_concat_keys | `avg_latency↓ && bits_sum ≥ incumbent` | `DEFAULT_LOOM_RECALL_TARGET = 1.0` | 415_000 |
| Storage (compaction) | `scope_storage.rs` | compaction_interval, debt trigger, write-amp, tiers, prefetch | `storage_win_check` (p99↓, write-amp/cache-miss/staleness no worse, hot/prefetch-hit ≥) | `DEFAULT_STORAGE_RECALL_TARGET = 1.0` | 583_000 |

Index quant gating requires `QuantPromotionEvidence` (cosine error, guard FAR) when `quant_bits` changes; valid quant bits `{4,8,16,32}`; `DEFAULT_INDEX_VRAM_BUDGET_BYTES = 1 GiB`; `MIN_BITS_PER_ANCHOR = 0.05`. Common caps: `MAX_{FORGE,INDEX,STORAGE}_CANDIDATES = 8`, `MAX_LOOM_EAGER_PAIRS = 64`. Per-scope errors `CALYX_{FORGE,INDEX,LOOM,STORAGE}_SCOPE_INVALID_CONFIG` and `…_CACHE_WRITE_FAIL` / `CALYX_LOOM_PLAN_WRITE_FAIL`.

---

## 13.16 Propose: lens & operator synthesis (`src/propose/*`)

### 13.16.1 Lens proposal pipeline — `ProposeLens::propose_lens` (`propose_lens.rs`)

Terminal states (`ProposalTerminalState`): `NoDeficit | GateRejected | HotAddFailed | SubstrateReverted | NoSufficiencyGain | Admitted`.

1. **Deficit localize** (`DeficitLocalizer::localize`): per anchor `gap = max(0, H(anchor) − I(panel;anchor))`, DPI-validated (`I ≤ H + ε`), accumulate `total_bits_deficit`, find underrepresented modalities (lens coverage `> MODALITY_COVERAGE_THRESHOLD_BITS = 0.10`).
2. **Has deficit?** `total_bits_deficit > DEFAULT_DEFICIT_THRESHOLD_BITS = 0.5` else `NoDeficit`.
3. **Synthesize** (`candidate_synth.rs`): algorithmic (`FrequencyBand → TimeLag → Tfidf → Pca`) capped at `MAX_SYNTHESIS_CORPUS_SAMPLE = 1000`, else `build_commission_spec` (ranked HuggingFace conversion targets by modality, `expected_bits = max(0, min(gap, gap·weight))`).
4. **Differentiation gate** (`differentiation_gate.rs`): profile candidate (timeout `PROFILE_TIMEOUT_MS = 30_000`), require `bits ≥ DIFFERENTIATION_MIN_BITS = 0.05` and `max NMI corr ≤ DIFFERENTIATION_MAX_CORR = 0.6` over panel lenses; else `GateRejected`.
5. **Hot-add**: `plan_hot_add` → `ensure_prior` → `propose_hot_add` (shadowed); Revert → `SubstrateReverted`. `apply_hot_add` (`registry_hot_add.rs`) registers a frozen algorithmic lens or commissions an external model and adds the slot.
6. **Sufficiency re-check**: if `sufficiency_after ≤ sufficiency_before + ε` → rollback, `NoSufficiencyGain`; else `Admitted`.

`record_outcome`/`proposal_history` (`admission_record.rs`) write `LensAdmitted`/`LensRejected` ledger entries. Errors: `CALYX_ASSAY_{UNAVAILABLE,INVALID_METRIC}`, `CALYX_ANNEAL_DEFICIT_INVALID_CONFIG`, `CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT`, `CALYX_REGISTRY_{HOT_ADD_FAIL,PROFILE_TIMEOUT}`.

### 13.16.2 Operator proposal — `ProposeOperator::propose_operator` (`operator_synth.rs`)

When refit doesn't close the deficit, proposes a learned operator (`ProposedOperator = OnlineHead{kind, param_count} | KernelScope{scope, scope_hash, kernel_recall_before/after}`). Terminal states: `NoDeficit | RefitClosed | Promoted | RolledBack`. Gating: `deficit_total > deficit_threshold_bits (0.5)`, `refit_delta_j < deficit_total`, and `shadow_delta_j ≥ min_delta_j (1e-6)` else `CALYX_ANNEAL_OPERATOR_NO_GAIN`. Promotes via `OperatorPromotionGate` (ledger `OperatorPromoted`/`OperatorReverted`). Records stored to CF `AnnealOperators`, tag `anneal_operator_proposal_v1`, key `operator/v1/{ts_be}{proposal_id}`. Error `CALYX_ANNEAL_OPERATOR_INVALID_RECORD`.

---

## 13.17 Recurrence schedule (`src/recurrence_schedule.rs`)

`recurrence_schedule_for(cx_id, vault, clock)` derives a `RecurrenceSchedule { cx_id, importance_weight, next_expected_t, refresh_priority }` from the recurrence series. `frequency_kernel_bonus(frequency)` (`:70`):

```
0                                if frequency == 0
min( ln(min(frequency, 10_000)+1) / ln(10_001), 1.0 )   otherwise
```

with `FREQ_BONUS_MAX = 10_000`. `refresh_priority(cadence)` (`:112`): `None→OneTime`, `<3600s→Hot`, `<86400s→Warm`, else `Cold`. `retention_tier` maps `Hot→Memtable`, `Warm→SstableTier1`, `Cold|OneTime→Archive`. Error `CALYX_ANNEAL_INVALID_CADENCE`.

---

## 13.18 Cross-reference summary

- Every mutating loop (rebuild, tau recalibration, head update, lens/operator hot-add, autotune promotion) funnels through `AnnealSubstrate::propose_change_*` (§13.5), which gates on **tripwires** (§13.2) and **shadow replay** (§13.3), records to the **rollback store** (§13.4) and **ledger** (§13.7), and respects the **budget** (§13.6).
- **Goodhart** (§13.10) and **frozen-lens** guards (§13.14.5) protect the integrity of grounded inputs to **J** (§13.8); generated/model-derived signals are explicitly excluded and `synthetic_recursion` is rejected.
- The **intelligence gradient** (§13.9) ranks next actions; the **growth curve** (§13.11) and **intelligence report** (§13.12) track J over time.
- Sibling crates: `calyx-aster` (column families, vault, recurrence), `calyx-ledger` (`LedgerAppender`, `EntryKind::Anneal`), `calyx-core` (`CalyxError`, `Clock`, ids). The PH16 autotune cache is consumed by the scope tuners (§13.15.4).
