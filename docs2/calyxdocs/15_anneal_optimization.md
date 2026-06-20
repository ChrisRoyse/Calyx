# 15 — Anneal: Reversible Self-Optimization

**Source files covered:**

- `crates/calyx-anneal/src/lib.rs` (module tree, re-exports)
- `crates/calyx-anneal/src/tripwire.rs` (safety tripwires)
- `crates/calyx-anneal/src/shadow.rs` (shadow testing)
- `crates/calyx-anneal/src/rollback.rs` + `rollback_codec.rs` (reversibility)
- `crates/calyx-anneal/src/integration_fsv.rs` (substrate / propose→shadow→gate→commit orchestration)
- `crates/calyx-anneal/src/ledger_anneal.rs` (Ledger audit entries)
- `crates/calyx-anneal/src/budget.rs` (bounded background compute)
- `crates/calyx-anneal/src/tune/` (autotune scopes: index/forge/loom/storage, bandit, A/B, soak)
- `crates/calyx-anneal/src/propose/` (lens & operator proposal, differentiation gate)
- `crates/calyx-anneal/src/learn/` (mistake-closure, online heads, regression check, frozen-lens guard)
- `crates/calyx-anneal/src/heal/` (degrade/rebuild/recalibrate/restore/triggers)
- `crates/calyx-anneal/src/j/` (Intelligence Objective J, Goodhart guard, growth curve)
- `crates/calyx-anneal/src/janitor.rs`, `recurrence_schedule.rs`
- Plan: `docs/dbprdplans/12_ANNEAL_SELF_OPTIMIZATION.md`

Anneal is Calyx's background self-optimization subsystem. It tunes engine parameters
(index params, quantization, kernel tile sizes, materialization plans, storage cadence,
online heads, Ward `τ`), shadow-tests every change against a held-out query replay,
auto-reverts anything that crosses a tripwire or regresses, and records every promotion
or revert in the Ledger. The crate is large (≈156 `.rs` files); this doc focuses on the
**self-optimize / safety / reversibility** core and summarizes the heal/learn/propose
loops. Cross-refs: [13_ward_guard.md](13_ward_guard.md) (FAR/FRR, `τ`),
[14_ledger_provenance.md](14_ledger_provenance.md) (hash chain),
[11_assay_signal_bits.md](11_assay_signal_bits.md) (sufficiency deficit),
[09_sextant_search.md](09_sextant_search.md) (recall, ANN params).

The module-level doc comment is the entire description in `lib.rs`:
*"Anneal self-optimization contracts for reversible tuning loops."* Per the plan
(`12 §1`) Anneal has three loops — **self-heal** (continuous/on-fault), **self-learn**
(online, per-mistake), **self-optimize** (adaptive, usage-driven) — bound by two
invariants: never regress a tripwire metric, and always reversible + Ledger-logged.

---

## 1. Tunable parameters (the knobs)

Anneal autotunes four layers (`tune/` submodules). Each layer has a config struct, a
bandit over a small candidate set, and a scope tuner. Recall/VRAM "targets" are stored
as cache-key discriminators (`AutotuneKey`), not validated bounds; the hard gates come
from the `TripwireRegistry` (§3) and per-scope win checks (§4).

### 1.1 Index layer — `IndexConfig` (`tune/scope_index/types.rs`)

| Field | Type | Default | Range / valid values |
|---|---|---|---|
| `hnsw_ef` | `u32` | 64 | nonzero (`validate_index_config`) |
| `hnsw_m` | `u32` | 16 | nonzero |
| `diskann_beamwidth` | `u32` | 32 | nonzero |
| `spann_cutoff` | `u32` | 1024 | nonzero |
| `quant_bits` | `u8` | 16 | one of `{4, 8, 16, 32}` (`VALID_QUANT_BITS`) |

- Candidate arms (`candidate_configs`): up to 8, each pruned to fit a VRAM budget
  (`DEFAULT_INDEX_VRAM_BUDGET_BYTES = 1 << 30` = 1 GiB). Variants exercise
  `ef∈{128,256}`, `m∈{8,32}`, `beamwidth 64`, `spann 2048`, `quant∈{4,8}`. If pruning
  empties the set → `CALYX_INDEX_SCOPE_INVALID_CONFIG`.
- VRAM estimate (`estimate_vram_bytes`): `ef*m*16 + beamwidth*4096 + cutoff*8 + (slot+1)*4096*quant_bits`.
- Changing `quant_bits` during promotion **requires** `QuantPromotionEvidence`
  (`cosine_error_before/after`, `max_cosine_error`, `guard_far_before/after`, all
  `f64`). Validated: finite & ≥0; `cosine_error_after ≤ max_cosine_error + 1e-6`;
  `guard_far_after ≤ guard_far_before + 1e-12`.

### 1.2 Forge / kernel layer — `ForgeConfig` (`tune/scope_forge/types.rs`)

| Field | Type | Default | Notes |
|---|---|---|---|
| `tile_m` | `u32` | 64 | not range-validated (no `validate_forge_config`) |
| `tile_n` | `u32` | 64 | — |
| `tile_k` | `u32` | 32 | — |
| `dtype` | `DType` | from key | enum `{Fp32, Fp16, Bf16, Fp8}` |
| `batch_size` | `u32` | 1 | — |

Keyed by `ShapeKey { op_id, shape_bucketed: Vec<u32>, dtype, device_id }`. Dims are
bucketed to powers of two (cap `MAX_BUCKETED_DIM = 65_536`). Candidates (≤8): wider
tiles, batch 2, and (only when `device_id` contains `"cuda"` and `dtype != Fp32`) `Bf16`
and `Fp8` variants. Forge is the only scope with **no config validator** — its knobs are
unbounded `u32`s.

### 1.3 Materialization (Loom) layer — `MatPlanConfig` (`tune/scope_loom/types.rs`)

| Field | Type | Default | Range |
|---|---|---|---|
| `eager_pairs` | `Vec<(LensId, LensId)>` | empty | len ≤ `MAX_LOOM_EAGER_PAIRS` (64); canonical, sorted, unique |
| `indexed_concat_keys` | `Vec<ConcatKey>` | empty | each `a ≤ b`; sorted, deduped |

`generate_candidate_plan` ranks lens pairs by bits-per-anchor (desc) then query count;
promotes the top pair to eager only if its bits are finite and `≥ MIN_LOOM_PAIR_BITS`
(0.05), trimming to the eager-pair budget. `ConcatKey { a: LensId, b: LensId }`.

### 1.4 Storage layer — `StorageConfig` (`tune/scope_storage/types.rs`)

| Field | Type | Default | Range |
|---|---|---|---|
| `compaction_interval_ms` | `u64` | 10_000 | `100 ..= 600_000` |
| `debt_trigger_score_milli` | `u64` | 1_000 | `100 ..= 10_000` |
| `max_write_amp_milli` | `u64` | 2_000 | `1_000 ..= 10_000` |
| `hot_tier_min_hits` | `u64` | 8 | nonzero |
| `cold_tier_idle_secs` | `u64` | 86_400 | `60 ..= 31_536_000` |
| `codebook_refresh_secs` | `u64` | 3_600 | `60 ..= 604_800` |
| `prefetch_bytes` | `u64` | 65_536 | `≤ 16 MiB`; zero or multiple of 4096 |

### 1.5 Online heads (learn layer) — `OnlineHead` (`learn/online_head.rs`)

Tuned by the self-learn loop, not the bandit. `kind: HeadKind ∈ {Predictor, Calibrator,
FusionWeights}`, `params: Vec<f32>` capped at `MAX_ONLINE_HEAD_PARAMS` (1024;
`CALYX_ANNEAL_HEAD_TOO_LARGE` if exceeded), `fisher_diag`, `version`, `prior_params`.
Updated by EWC/Fisher-regularized SGD; **frozen lens weights are never touched** (§7).

### 1.6 Ward `τ` and lens parking (heal layer)

`τ` recalibration thresholds and the **signal-decay floor** `SIGNAL_DECAY_FLOOR_BITS =
0.05` (`heal/recalibrate/types.rs`): a lens whose Assay signal decays below 0.05 bits is
auto-**parked** (kept, search stopped). See [13_ward_guard.md](13_ward_guard.md).

---

## 2. The optimization loop (propose → shadow-test → gate → commit-or-revert)

The orchestration is `AnnealSubstrate` (`integration_fsv.rs`), parameterized over a
rollback store, ledger CF store, clock, and a budget probe. The canonical entry point is
`propose_change_with_actions_and_details` (and the simpler wrappers `propose_change`,
`propose_change_with_description`, `propose_change_with_actions`). Steps:

1. **Prepare (reserve rollback).** `rollback.prepare_with_description(key,
   candidate_ptr, description)` allocates a `ChangeId`, snapshots the current
   `prior_ptr` (the live artifact), and records `candidate_ptr` — all with
   `promoted=reverted=committed=false`. Requires an existing live pointer for `key`
   (else `CALYX_ANNEAL_INVALID_ROLLBACK_STATE`). The prior artifact is kept; nothing
   live changes yet.
2. **Acquire budget.** `budget.acquire(shadow_cpu_weight, shadow_vram_bytes)` (defaults
   `0.01` CPU weight, `0` VRAM). If the background budget is exhausted, the change
   short-circuits to a `Revert{ reason: BudgetExhausted }` verdict (it does **not**
   error).
3. **Shadow-test.** A `ShadowExecutor` runs the candidate and incumbent over the
   held-out replay and returns a `ShadowVerdict` (§4).
4. **Gate.** The verdict is `Promote{metrics}` only if every metric passed both the
   tripwire check **and** the per-metric non-regression check; otherwise `Revert{reason,
   metrics}`.
5. **Commit-or-revert.**
   - `Promote`: write a Ledger entry (`AnnealLedgerAction::Promote` by default), then
     `rollback.promote(change_id)` — a **single pointer swap** that makes
     `candidate_ptr` the live pointer. Returns `ChangeOutcome::Promoted(change_id)`.
   - `Revert`: `rollback.rollback(change_id)` swaps the live pointer back to
     `prior_ptr`, write a Ledger entry (`AnnealLedgerAction::Revert`), return
     `ChangeOutcome::Reverted{ reason, change_id }`.

`ChangeOutcome` (`integration_fsv.rs`): `Promoted(ChangeId)` |
`Reverted{ reason: ShadowRevertReason, change_id: ChangeId }`.

The four scope tuners (§4) and the lens-proposal pipeline (§5) all drive this same
substrate. The A/B runner (`tune/ab_runner.rs`) is a parallel live-traffic variant that
trials a candidate arm against the incumbent over `min_samples` query pairs before
declaring a verdict.

`AnnealSubstrate::status()` returns `AnnealStatus { tripwire_states:
Vec<TripwireStatus>, budget: BudgetStatus, recent_changes: Vec<AnnealLedgerEntry> }`
(last 16 ledger entries).

---

## 3. Safety tripwires (exact metrics & thresholds)

Defined in `tripwire.rs`. Five guarded metrics, persisted to vault
`.anneal/tripwire.toml` (`tripwire_config_path`). On first load, defaults are written if
the file is absent.

### 3.1 The metrics, directions, and default bounds

`TripwireMetric` enum + `default_bound` / `default_direction`:

| Metric (`serde` key) | Direction | Default bound | Crosses when… |
|---|---|---|---|
| `RecallAtK` (`recall_at_k`) | `Below` | **0.90** | recall falls below the bound |
| `GuardFAR` (`guard_far`) | `Above` | **0.01** | guard false-accept rate rises above |
| `GuardFRR` (`guard_frr`) | `Above` | **0.05** | guard false-reject rate rises above |
| `SearchP99` (`search_p99`) | `Above` | **200.0** | search p99 latency (ms) rises above |
| `IngestP95` (`ingest_p95`) | `Above` | **500.0** | ingest p95 latency (ms) rises above |

Any candidate metric crossing its tripwire forces a revert (§2 step 5; §4).
`GuardFAR`/`GuardFRR` thresholds tie into Ward — see [13_ward_guard.md](13_ward_guard.md).

### 3.2 Hysteresis and crossing logic

`TripwireThreshold { bound: f64, hysteresis: f64, direction: ThresholdDir }`. Default
hysteresis = `bound * DEFAULT_HYSTERESIS_FRACTION` (0.05, i.e. 5% of the bound).
`TRIPWIRE_EPSILON = 1e-12`. `threshold_crossed` is stateful (Schmitt-trigger style): once
crossed, the metric must recover **past the hysteresis band** to clear:

- `Below`, not yet crossed: `value < bound − ε`.
- `Below`, already crossed: `value < bound + hysteresis − ε` (must rise to `bound +
  hysteresis` to clear).
- `Above`, not yet crossed: `value > bound + ε`.
- `Above`, already crossed: `value > bound − hysteresis + ε` (must fall to `bound −
  hysteresis` to clear).

`TripwireResult` enum (serde tag `result`): `Ok` | `Crossed{ metric, threshold,
hysteresis }`. `TripwireRegistry::check(metric, value)` updates state and returns the
result; a non-finite value yields `CALYX_TRIPWIRE_INVALID_METRIC`.

### 3.3 Config validation (`validate_threshold`)

`bound` and `hysteresis` must be finite & ≥ 0; `direction` must equal the metric's fixed
`default_direction` (you cannot flip a metric's sense); for `Below` metrics, hysteresis
must not exceed the bound. `set_tripwire(metric, bound, hysteresis)` re-validates,
requires all five metrics present, persists the TOML, and resets state. Errors:
`CALYX_TRIPWIRE_INVALID_CONFIG`.

### 3.4 Public tripwire types

| Type | Shape |
|---|---|
| `TripwireMetric` | enum (5 variants above) |
| `ThresholdDir` | `Below` \| `Above` |
| `TripwireThreshold` | `{ bound: f64, hysteresis: f64, direction: ThresholdDir }` |
| `ThresholdState` | `{ last_value: f64, crossed: bool }` |
| `TripwireStatus` | `{ metric, threshold, state }` |
| `TripwireResult` | `Ok` \| `Crossed{ metric, threshold: f64, hysteresis: f64 }` |
| `TripwireThresholdEntry` | `{ metric, threshold }` |
| `TripwireConfigReadback` | `{ config_path: PathBuf, thresholds: Vec<TripwireThresholdEntry> }` |
| `TripwireRegistry` | live registry; `load_from_vault`, `check`, `set_tripwire`, `status`, `config_path` |

### 3.5 Related non-tripwire safety thresholds

| Constant | Value | File | Role |
|---|---|---|---|
| `SIGNAL_DECAY_FLOOR_BITS` | 0.05 | `heal/recalibrate/types.rs` | park a lens below this Assay signal |
| `DIFFERENTIATION_MIN_BITS` | 0.05 | `propose/differentiation_gate.rs` | min bits a proposed lens must add |
| `DIFFERENTIATION_MAX_CORR` | 0.6 | same | max NMI vs an existing lens |
| `DEFAULT_MAX_REGRESSION_RATE` | 0.05 | `learn/regression_assert.rs` | max recurred-mistake fraction before head rollback |
| `DEFAULT_GTAU_THRESHOLD` | 0.95 | `j/goodhart.rs` | Goodhart `g(τ)` guard |
| `DEFAULT_CROSS_LENS_DOMINANCE_THRESHOLD` | 0.80 | same | single-lens dominance guard |
| `DEFAULT_HELD_OUT_MIN_GAIN_FRACTION` | 0.01 | same | min held-out J gain to keep a change |
| `MAX_JANITOR_BYTES_PER_TICK` | 100 MiB | `janitor/types.rs` | GC throughput cap |

---

## 4. Shadow testing (how a candidate is evaluated before going live)

`shadow.rs` defines the held-out replay evaluation. Same five metrics as tripwires
(`SHADOW_METRICS`). `COMPARE_EPSILON = 1e-12`.

### 4.1 Held-out replay

- `ReplayQuery { query_id: u64, query_vector: Vec<f32>, expected_top_k: Vec<ReplayAnchor> }`,
  `ReplayAnchor { cx_id: CxId, similarity: f32 }`.
- `HeldOutReplay { queries: Vec<ReplayQuery>, seed: u64 }`. `HeldOutReplay::sample(queries,
  n, seed)` shuffles with `ChaCha8Rng` seeded from `seed`, then truncates to `n`
  (deterministic).
- `ReplaySource::replay_queries() -> Result<Vec<ReplayQuery>>`; `build_replay(source, n,
  seed)` samples from a source.

### 4.2 Candidate interface

`trait AnnealAction { fn apply_shadow(&self, query: &ReplayQuery) -> ActionMetricSnapshot; }`
(`Send + Sync`). Both the candidate and the incumbent implement it; the executor runs
both on each replay query and compares.

### 4.3 `ShadowExecutor::run_shadow(candidate, incumbent)` steps

1. If the replay is empty → `Revert{ InsufficientReplay }`.
2. For each replay query: `budget.try_consume()` first — if the cooperative budget tick
   is exhausted → `Revert{ BudgetExhausted }` with the metrics gathered so far. Then call
   `candidate.apply_shadow(query)` and `incumbent.apply_shadow(query)`.
3. The `MetricAccumulator` validates that **every** of the five metrics is present and
   finite on both sides (`MissingMetric{metric, side}` / `InvalidMetric{metric, side}`
   → revert) and accumulates per-metric sums.
4. After all queries, take the **mean** candidate/incumbent value per metric →
   `MetricSnapshot { evaluated_at: Ts, query_count, metrics: Vec<MetricComparison> }`,
   `MetricComparison { metric, candidate_value, incumbent_value }`.
5. For each comparison:
   - `registry.check(metric, candidate_value)` — if `Crossed` →
     `Revert{ TripwireCrossed(metric) }`; if the check errors → `Revert{ TripwireError{
     metric, code } }`.
   - `regressed(comparison)` — if regressed → `Revert{ MetricRegression(metric) }`.
6. If nothing reverted → `Promote{ metrics }`.

### 4.4 Regression rule (`regressed`)

- `RecallAtK`: regressed if `candidate + ε < incumbent` (recall must not drop).
- `GuardFAR`, `GuardFRR`, `SearchP99`, `IngestP95`: regressed if `candidate > incumbent +
  ε` (these must not rise).

So a candidate is promoted only if it **crosses no tripwire** and is **no worse than the
incumbent on every metric** over the held-out replay.

### 4.5 Verdict / revert-reason types

`ShadowVerdict` (serde tag `verdict`): `Promote{ metrics }` | `Revert{ reason:
ShadowRevertReason, metrics }`.

`ShadowRevertReason` (serde tag `reason`, content `details`): `TripwireCrossed(metric)`,
`MetricRegression(metric)`, `BudgetExhausted`, `InsufficientReplay`, `MissingMetric{
metric, side }`, `InvalidMetric{ metric, side }`, `TripwireError{ metric, code: String }`.
`MetricSide`: `Candidate` | `Incumbent`.

### 4.6 Soak harness (extended shadow run)

`tune/soak_harness.rs` runs a long deterministic load. `SoakConfig { n_queries, seed,
mode: SoakMode∈{Seeded,LiveTraffic}, p99_target_reduction, min_recall, oscillation_window,
sample_interval, max_runtime_ms }`. Defaults: `DEFAULT_SOAK_QUERIES = 1_000_000`,
`DEFAULT_SOAK_SEED = 0xABCDEF`, `DEFAULT_SOAK_P99_TARGET_REDUCTION = 0.20`,
`DEFAULT_SOAK_OSCILLATION_WINDOW = 10_000`, `DEFAULT_SOAK_SAMPLE_INTERVAL = 1_000`,
`max_runtime_ms = 7_200_000` (2 h). `SoakReport.gate_passed` iff `p99_reduction ≥
p99_target_reduction` **and** `recall_final ≥ max(min_recall, recall_baseline)` **and**
`!oscillation_detected`. `check_oscillation` flags a >5% consecutive p99 rise within the
window. Errors: `CALYX_ANNEAL_SOAK_INVALID_CONFIG`, `CALYX_ANNEAL_SOAK_INVALID_ROW`,
`CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED`.

### 4.7 Scope tuners and the bandit / A-B path

Each layer (§1.1–1.4) has a bandit-backed tuner: `IndexScopeTuner`, `ForgeScopeTuner`,
`LoomScopeTuner`, `StorageScopeTuner`. On each live observation a tuner (a) evaluates the
current shadow arm, (b) `record_result` (hysteresis may flip the incumbent → a
promotion), (c) persists the bandit, (d) `select_arm` to pick the next candidate. The
returned `*TuneDecision` carries `{ evaluated_arm, won, incumbent, promoted:
Option<*PromotionRecord>, shadow_arm, shadow_candidate }`. Only `IndexTuneDecision` adds
`skipped: Option<IndexTuneSkip>` (`IndexTuneSkip::ParkedSlot`).

Per-scope win checks (in addition to the tripwire/regression rules):

| Tuner | Promotes when |
|---|---|
| Index | `p99 < baseline` **and** `recall + 1e-12 ≥ baseline` **and** `quant_win_check` (quant change needs evidence) |
| Forge | `elapsed < baseline_latency` **and** `recall + 1e-12 ≥ baseline_recall` |
| Loom | candidate `avg_latency_ns < incumbent` **and** `bits_sum + 1e-12 ≥ incumbent` |
| Storage | `storage_win_check`: p99↓, write-amp / cache-miss / staleness ↓-or-equal, hot-hit / prefetch-hit ↑-or-equal |

**Bandit** (`tune/bandit.rs`): `ConfigBandit { policy, arms: Vec<Arm>, incumbent_idx,
hysteresis_wins, rng_seed }`. `BanditPolicy = EpsilonGreedy{epsilon}` (validated `[0,1]`)
| `Thompson` (Beta sampling). `Arm { config: Vec<u8>, wins, trials, consecutive_wins }`.
Promotion requires the candidate to win `≥ DEFAULT_HYSTERESIS_WINS` (3) consecutive
trials. Errors: `CALYX_ANNEAL_BANDIT_EMPTY`, `CALYX_ANNEAL_BANDIT_INVALID_CONFIG`,
`CALYX_ANNEAL_BANDIT_INVALID_ROW`.

**A/B runner** (`tune/ab_runner.rs`): `ABRunner` holds active `ABTrial`s keyed by
`ShapeKey`. `start_trial` (default `DEFAULT_AB_MIN_SAMPLES = 100` query pairs;
`CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE` if one is live). `record_query` consumes shadow
budget (abandons on exhaustion), validates results (`CALYX_ANNEAL_TRIAL_INVALID_RESULT`),
and once `min_samples` pairs are collected, `declare_winner` computes p99/mean summaries
and a `candidate_won` flag combining the candidate's recall tripwire, recall
non-regression, latency tripwire, bits non-regression, and `p99 < incumbent p99`.
`ABVerdict = Promoted(rec) | Kept(rec) | Abandoned(rec)`; `ABVerdictRecord` carries
before/after metrics and a `reason` string (`"promoted"`, `"recall_tripwire"`,
`"recall_regression"`, `"latency_tripwire"`, `"bits_regression"`,
`"candidate_not_faster_or_hysteresis_pending"`).

---

## 5. Lens & operator proposal (closing the sufficiency deficit)

`propose/` implements the self-optimize loop's "propose a new sensor" branch (plan
`12 §5`). When Assay reports `I(panel;anchor) ≪ H(anchor)`, Anneal proposes a new lens or
operator.

### 5.1 Lens proposal pipeline (`propose/propose_lens.rs`)

`ProposeLens::propose_lens` steps:
1. Collect non-retired panel lens ids.
2. `DeficitLocalizer::localize(assay, anchor, panel)` → `DeficitMap { top_gaps:
   Vec<AnchorGap>, underrepresented_modalities, total_bits_deficit }`. `AnchorGap {
   anchor_class, entropy_h, mutual_info_i, gap = max(H−I, 0) }`.
3. If `total deficit ≤ DEFAULT_DEFICIT_THRESHOLD_BITS` (0.5) → terminal `NoDeficit`.
4. `synthesize(deficit, corpus)` → `CandidateLens` (Algorithmic PCA/TimeLag/
   FrequencyBand/Tfidf, else a Commission spec).
5. **Differentiation gate** `gate(...)` (§5.2). Reject → `GateRejected`.
6. `plan_hot_add` → on error `HotAddFailed{code}`.
7. Through `AnnealSubstrate` shadow path (`propose_hot_add`): `Reverted` →
   `SubstrateReverted{reason}`; `Promoted` → `apply_hot_add` (hot-add, no re-embed).
8. Re-measure sufficiency; if `after ≤ before + 1e-12` → roll back, `NoSufficiencyGain`;
   else `Admitted`.

`ProposalTerminalState`: `NoDeficit | GateRejected | HotAddFailed{code} |
SubstrateReverted{reason} | NoSufficiencyGain | Admitted`.

### 5.2 Differentiation gate (`propose/differentiation_gate.rs`)

Admission contract: a candidate is admitted iff `bits ≥ DIFFERENTIATION_MIN_BITS` (0.05)
**and** `max_corr ≤ DIFFERENTIATION_MAX_CORR` (0.6) against every existing panel lens
(NMI), profiled within `PROFILE_TIMEOUT_MS` (30_000 ms). `GateOutcome = Admitted{bits,
max_corr} | Rejected{reason}`; `RejectReason = InsufficientBits{bits, threshold} |
TooCorrelated{corr, offending_lens, threshold} | ProfileTimeout`.
`MODALITY_COVERAGE_THRESHOLD_BITS = 0.10`, `MAX_SYNTHESIS_CORPUS_SAMPLE = 1000`.

### 5.3 Operator proposal (`propose/operator_synth.rs`)

`ProposeOperator::propose_operator`: validates the deficit obeys DPI (`I ≤ H`); returns
`NoDeficit` (`≤ 0.5` bits) or `RefitClosed` (existing refit already closes the gap);
synthesizes a `ProposedOperator = OnlineHead{kind, param_count} | KernelScope{scope,
recall_before, recall_after}`; requires `shadow_delta_j ≥ min_delta_j` (default `1e-6`,
else `CALYX_ANNEAL_OPERATOR_NO_GAIN`); routes through the gate →
`OperatorTerminalState = NoDeficit | RefitClosed | Promoted | RolledBack{reason}`.

---

## 6. Self-learn (mistake closure) — summary

`learn/` implements the "wrong only once" loop (plan `12 §3`). Key flow:

1. `record_outcome` (`learn/outcome.rs`): an arriving `Anchor` (test pass / tie / thumbs)
   is mapped to a scalar in `[0,1]`. If a **trusted** prediction is contradicted with
   `surprise = |predicted − observed| ≥ contradiction_threshold` (default 0.3) →
   contradiction path (record a mistake, ledger `OutcomeContradiction`); otherwise reward
   path (queue the outcome, update online heads, ledger `OutcomeReward`).
2. `record_mistake_for_replay`: append to `MistakeLog`, push to a surprise-prioritized
   `ReplayBuffer` (capacity `DEFAULT_REPLAY_CAPACITY = 4096`, min-surprise evicted) if
   `surprise ≥ DEFAULT_SLEEP_PASS_MIN_SURPRISE` (0.01).
3. `run_sleep_pass` (`learn/online_head/sleep_pass.rs`): sample a surprise-weighted batch
   (`DEFAULT_SLEEP_PASS_BATCH_SIZE = 16`), run `update_with_regression` (EWC/Fisher SGD on
   online heads), and re-assert that replayed mistakes do not recur.
4. `assert_no_regression` / `record_regression`: if the recurred fraction exceeds
   `RegressionConfig.max_regression_rate` (default `DEFAULT_MAX_REGRESSION_RATE = 0.05`,
   sleep pass uses `strict() = 0.0`), the head update is rolled back
   (`CALYX_ANNEAL_REGRESSION_RECURRED`) and the recurred mistakes are re-queued at higher
   priority.

`SleepPassOutcome` (serde tag `status`): `Idle | Deferred | Promoted{update} |
Reverted{error_code, message, ...}`. Other defaults: `DEFAULT_MISTAKE_SURPRISE_THRESHOLD =
0.3`, `DEFAULT_OUTCOME_LR = 1.0` (f32), `DEFAULT_OUTCOME_FISHER_WEIGHT = 0.0`,
`DEFAULT_OUTCOME_ACTION_COST = 1.0`.

---

## 7. Reversibility (rollback store) — cross-ref Ledger

`rollback.rs` is the reversible-change backbone. Every Anneal mutation keeps the prior
artifact until the candidate is proven, and a promotion/revert is a single live-pointer
swap.

### 7.1 Artifacts and snapshots

| Type | Shape |
|---|---|
| `ChangeId(pub u64)` | monotonic id allocated from `clock.now() * 1_000_000 + seed%bucket + counter`, never below `last_id+1` |
| `LogicalTime` | alias for `Ts` |
| `ArtifactKey` (serde) | `ConfigCache([u8;32])` \| `HnswGraph([u8;32])` \| `QuantLevel([u8;32])` |
| `ArtifactPtr` (serde) | `ConfigCacheKeyHash([u8;32])` \| `HnswGraphPath(String)` \| `QuantLevelRecordHash([u8;32])` |
| `ArtifactSnapshot` | `{ change_id, key, prior_ptr, candidate_ptr, ts, description, promoted, reverted, committed }` |
| `RollbackReadback` | snapshot + live pointer + raw snapshot/live key & value bytes |

### 7.2 State machine (per change_id)

A change progresses through boolean flags on its `ArtifactSnapshot`. States and
transitions (`RollbackStore`):

| Op | Precondition | Effect | State after |
|---|---|---|---|
| `prepare` / `prepare_with_description` | a live ptr exists for `key` | snapshot written (`promoted=reverted=committed=false`) | **Prepared** |
| `promote(id)` | not `committed`, not `reverted` | live ptr ← `candidate_ptr`; `promoted=true` | **Promoted** (revertible) |
| `rollback(id)` | not `committed` | live ptr ← `prior_ptr`; `reverted=true` | **Reverted** |
| `commit(id)` | — | `committed=true` | **Committed** (frozen) |

Transition guards / errors:
- Promote a committed change → `CALYX_ANNEAL_CHANGE_COMMITTED`.
- Promote a reverted change → `CALYX_ANNEAL_INVALID_ROLLBACK_STATE`.
- Rollback a committed change → `CALYX_ANNEAL_CHANGE_COMMITTED`.
- Unknown id → `CALYX_ANNEAL_UNKNOWN_CHANGE_ID`.

`commit` is the terminal "lock in" — once committed, the change can no longer be reverted
(open a new change instead). A `Promoted` (but not committed) change is still revertible:
`AnnealSubstrate::rollback_explicit(change_id)` performs a `rollback` and writes a
`Revert` ledger entry. This is the `rollback(change_id)` API in plan `12 §7`.

### 7.3 Storage

`RollbackStorage` trait (`put_many`, `get`, `scan`). `AsterRollbackStorage` persists to
Aster column family `ColumnFamily::AnnealRollback` (live-pointer rows keyed
`rollback_live_key(key)`, snapshot rows `rollback_snapshot_key(change_id)`).
`RollbackStore::open` rebuilds in-memory state by scanning the CF (prefixes
`CHANGE_PREFIX` / `LIVE_PREFIX`). State is guarded by an `RwLock`; poisoning →
`CalyxError::backpressure`.

### 7.4 Ledger entries (cross-ref [14_ledger_provenance.md](14_ledger_provenance.md))

`ledger_anneal.rs` writes hash-only audit entries to `EntryKind::Anneal`.
`AnnealLedger::write` enforces the hash chain: an entry's optional `prev_hash` must match
the appender tip (`CalyxError::ledger_chain_broken`), then `prev_hash` is set to the
chain tip. Payload tag `ANNEAL_LEDGER_PAYLOAD_TAG = "anneal_event_v1"`, JSON, capped at
`MAX_ANNEAL_LEDGER_PAYLOAD_BYTES = 16 KiB` (`CALYX_LEDGER_ENTRY_TOO_LARGE`).

`AnnealLedgerEntry`: `{ action: AnnealLedgerAction, change_id, artifact_id: String,
prior_ptr_hash: [u8;32], candidate_ptr_hash: [u8;32], metrics: MetricSnapshot, ts,
description, fault: Option<AnnealFaultLedgerDetails>, proposal: Option<AdmissionRecord>,
details: Option<Value>, prev_hash: Option<[u8;32]> }`. Pointers are stored as hashes only
(redaction-safe).

`AnnealLedgerAction` (28 variants): `Promote, Revert, Propose, LensAdmitted,
LensRejected, Park, DegradeChange, FaultEvent, Rebuild, BaseCorruptAlert, BaseRestored,
Recalibrate, TauRecalibrated, TauRecalibrationReverted, LensPark, LensUnpark,
MistakeUpdate, HeadUpdate, HeadUpdateReverted, OperatorPromoted, OperatorReverted,
SleepPassDeferred, OutcomeReward, OutcomeContradiction, AutotuneAB, AutotuneAbandoned,
AutotunePromote, GoodhartPassed, GoodhartFailed`. The promote/revert pair used by a
change is supplied via `AnnealLedgerActionPair { promote, revert }`.

---

## 8. Bounded background compute (budget)

`budget.rs` caps Anneal's resource use so it never starves serving (plan `12 §6`).
`BudgetConfig { cpu_fraction: f64 (default 0.15, validated 0.0..=1.0), vram_bytes: u64
(default 512 MiB), tick_interval_ms: u64 (default 100, must be >0) }`, persisted to
`.anneal/budget.toml`. `BACKGROUND_NICE = 10`.

`BudgetEnforcer::acquire(cpu_weight, vram_bytes)` samples current usage
(`BudgetProbe`/`ProcStatBudgetProbe` reads `/proc/stat`; no NVML → `vram=0`,
`nvml_available=false` → warning `CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE`), and rejects
with `CALYX_ANNEAL_BUDGET_EXHAUSTED` if projected CPU/VRAM would exceed the budget.
Returns a RAII `BudgetHandle` (cooperative `remaining_ticks`, releases reservations on
drop). `BudgetStatus { cpu_used_fraction, vram_used_bytes, handles_active, last_tick_at,
low_priority_nice, warning_code }`. `CALYX_ANNEAL_BUDGET_INVALID_CONFIG` for bad config.

---

## 9. Error taxonomy (selected, by module)

All `CALYX_*` string constants equal their identifier verbatim unless noted.

| Module | Codes |
|---|---|
| tripwire | `CALYX_TRIPWIRE_INVALID_METRIC`, `CALYX_TRIPWIRE_INVALID_CONFIG` |
| budget | `CALYX_ANNEAL_BUDGET_EXHAUSTED`, `_INVALID_CONFIG`, `_NVML_UNAVAILABLE` |
| rollback | `CALYX_ANNEAL_UNKNOWN_CHANGE_ID`, `_CHANGE_COMMITTED`, `_INVALID_ROLLBACK_STATE` |
| ledger | `CALYX_LEDGER_ENTRY_TOO_LARGE`, `CALYX_ANNEAL_LEDGER_INVALID_ENTRY`, `CALYX_ASTER_CF_UNAVAILABLE` |
| integration | `CALYX_LEDGER_WRITE_FAIL` |
| tune (scopes) | `CALYX_{INDEX,FORGE,LOOM,STORAGE}_CACHE_WRITE_FAIL`, `CALYX_{INDEX,FORGE,LOOM,STORAGE}_SCOPE_INVALID_CONFIG` |
| tune (bandit/AB/soak) | `CALYX_ANNEAL_BANDIT_{EMPTY,INVALID_CONFIG,INVALID_ROW}`, `CALYX_ANNEAL_TRIAL_{ALREADY_ACTIVE,NOT_ACTIVE,INVALID_RESULT}`, `CALYX_ANNEAL_AB_CACHE_WRITE_FAIL`, `CALYX_ANNEAL_SOAK_{INVALID_CONFIG,INVALID_ROW,TIME_BUDGET_EXHAUSTED}` |
| propose | `CALYX_REGISTRY_HOT_ADD_FAIL`, `CALYX_REGISTRY_PROFILE_TIMEOUT`, `CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT`, `CALYX_ASSAY_{UNAVAILABLE,INVALID_METRIC}`, `CALYX_ANNEAL_DEFICIT_INVALID_CONFIG`, `CALYX_ANNEAL_OPERATOR_{INVALID_RECORD,NO_GAIN}`, `CALYX_REGISTRY_{HOT_ADD_FAIL,PROFILE_TIMEOUT}` |
| learn | `CALYX_ANNEAL_{INVALID_WINDOW,INVALID_CAPACITY,HEAD_TOO_LARGE,HEAD_INVALID_ROW,HEAD_UPDATE_REVERTED}`, `CALYX_ANNEAL_MISTAKE_{INVALID_ROW,APPEND_ONLY}`, `CALYX_ANNEAL_OUTCOME_{INVALID_CONFIG,INVALID_ANCHOR,INVALID_ROW,APPEND_ONLY}`, `CALYX_ANNEAL_REGRESSION_{RECURRED,INVALID_CONFIG,SOURCE_UNAVAILABLE,NAN_PREDICTION}`, `CALYX_ANNEAL_REPLAY_INVALID_ROW`, `CALYX_ANNEAL_SLEEP_PASS_INVALID_CONFIG`, `CALYX_REGISTRY_UNAVAILABLE` |

---

## 10. Plan vs. code divergences

- The plan (`12 §6`) names tripwires as "recall@k, guard FAR/FRR, search p99, ingest
  p95" — the code matches exactly (`TripwireMetric`, §3.1) with default bounds
  0.90 / 0.01 / 0.05 / 200 / 500.
- Plan `12 §7` lists an `autotune_report` and `set_tripwire(metric, bound)` API; the
  crate exposes `set_tripwire(metric, bound, hysteresis)` (third arg) and the autotune
  data via the scope tuners / A-B ledger events rather than a single named function.
- Plan `12 §4` describes ε-greedy / Thompson exploration — both are implemented
  (`BanditPolicy`).
- The plan's framing of Anneal as "the optimizer of the Intelligence Objective J" is
  realized in the `j/` module (`compute_j`, `GoodhartChecker`, growth curves), which is
  summarized here but documented in depth only as far as its safety constants (§3.5).

## Gaps / not covered

- The `heal/` submodules (degrade, rebuild, recalibrate, restore, triggers) and the `j/`
  Intelligence-Objective machinery are summarized (key thresholds in §3.5, §6) but not
  field-by-field; they are large and adjacent to Ward/Assay. See
  [13_ward_guard.md](13_ward_guard.md), [11_assay_signal_bits.md](11_assay_signal_bits.md).
- `janitor.rs` (GC) and `recurrence_schedule.rs` (refresh cadence, `FREQ_BONUS_MAX =
  10_000`) are noted but not detailed.
- No `todo!()`/stub paths were observed in the core files read (tripwire, shadow,
  rollback, budget, integration, ledger). The behavior documented above is what the code
  does, not aspirational.
