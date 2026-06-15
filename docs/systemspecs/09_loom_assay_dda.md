# 09. Loom DDA & Assay Signal Bits (calyx-loom, calyx-assay)

This reference documents two sibling crates exactly as their source compiles today:

- **`calyx-loom`** — Dimensional Derivative Amplification (DDA): lazy cross-term materialization, the in-memory agreement graph, honest abundance reporting, a bounded reactive trigger engine, and recurrence/periodic series math.
- **`calyx-assay`** — signal-bit measurement: KSG / histogram-NMI / logistic mutual-information estimators, the lens differentiation contract, effective-rank (`n_eff`), panel sufficiency & deficit routing, per-sensor attribution, vault-scoped cache provenance, and a family of advanced estimators (Bayesian posteriors, MMD drift, transfer entropy, total correlation, Lomb-Scargle periodicity, inter-event hazard).

Every claim below is traced to a file path plus the function/type that implements it. Items the source does not determine are marked **"Not determined from source"**. Formulas are reproduced where the exact arithmetic is load-bearing.

> Cross-references: graph-kernel internals are documented separately — see [10_graph_kernel.md](10_graph_kernel.md). Core types (`SlotId`, `CxId`, `Anchor`, `AnchorKind`, `CalyxError`, `Result`, `Clock`, `VaultId`, `AsterVault`, `CfRouter`, `ColumnFamily`) come from `calyx-core`/`calyx-aster` — see [04_core_foundation.md](04_core_foundation.md).

## Source files covered

**calyx-loom** (`crates/calyx-loom/src/`):
`lib.rs`, `cross_term.rs`, `agreement_graph.rs`, `abundance.rs`, `materialization.rs`, `blind_spot.rs`, `lru_cache.rs`, `error.rs`, `reactive/{mod,engine,signals,subscription,durable}.rs`, `recurrence/{mod,periodic,cross_terms,series_store,signature}.rs`.

**calyx-assay** (`crates/calyx-assay/src/`):
`lib.rs`, `ksg.rs`, `nmi.rs`, `logistic.rs`, `estimate.rs`, `contract.rs`, `n_eff.rs`, `sufficiency.rs`, `attribution.rs`, `stratified.rs`, `formulas.rs`, `gate.rs`, `bootstrap.rs`, `samples.rs`, `projection.rs`, `store.rs`, `bayesian.rs`, `mmd.rs`, `transfer_entropy.rs`, `total_correlation.rs`, `periodicity.rs`, `recurrence_hazard.rs`, `recurrence_anchor.rs`, `loom_adapter.rs`, `formula_catalog.rs`, `special_fn.rs`.

---

# Part A — calyx-loom (DDA)

## A1. Cross-term value types & math kernels (`cross_term.rs`)

DDA derives new "cross-term" signals from pairs of lens/slot vectors. Four kinds exist.

### Public types

| Type | Kind | Fields / Variants | File |
| --- | --- | --- | --- |
| `CrossTermKind` | enum (`snake_case`, `Copy`, `Ord`, `Hash`) | `Agreement`, `Delta`, `Interaction`, `Concat` | `cross_term.rs:13` |
| `SignalProvenanceTag` | enum (`snake_case`, `Copy`) | `Measured`, `Derived` | `cross_term.rs:22` |
| `CrossTermKey` | struct (`Copy`, `Ord`, `Hash`) | `cx_id: CxId`, `a: SlotId`, `b: SlotId`, `kind: CrossTermKind` | `cross_term.rs:28` |
| `CrossTermValue` | enum (`snake_case`) | `Scalar(f32)`, `Vector(Vec<f32>)` | `cross_term.rs:37` |

### Kernels

| Function | Signature | Computes |
| --- | --- | --- |
| `canonical_pair` | `(a,b: SlotId) -> (SlotId, SlotId)` | Orders the pair `(min,max)` so `(a,b)` and `(b,a)` collapse to one key (`cross_term.rs:42`). |
| `agreement_scalar` | `(a,b: &[f32]) -> Result<f32>` | Cosine similarity `dot/(‖a‖·‖b‖)`; fails `CALYX_LOOM_ZERO_NORM_VECTOR` if either squared norm `≤ f32::EPSILON` (`cross_term.rs:46`). |
| `agreement_weight` | `(raw_cosine: f32) -> Result<f32>` | Clamps cosine to `[0.0, 1.0]`; fails `CALYX_LOOM_NON_FINITE_VECTOR` on non-finite input (`cross_term.rs:65`). |
| `agreement_batch_cpu` | `(&[(&[f32],&[f32])]) -> Result<Vec<f32>>` | Maps `agreement_scalar` over pairs (`cross_term.rs:75`). |
| `agreement_batch_gpu` | same | Returns empty for empty input; otherwise requires the `cuda` feature (CUDA cosine via `calyx_forge::CudaBackend`), else fails `CALYX_LOOM_FORGE_UNAVAILABLE` (`cross_term.rs:79`, `:134`). |
| `delta_vec` | `(a,b) -> Result<Vec<f32>>` | Elementwise `a−b` (`cross_term.rs:96`). |
| `interaction_vec` | `(a,b) -> Result<Vec<f32>>` | Elementwise `a·b` (Hadamard product) (`cross_term.rs:101`). |
| `concat_vec` | `(a,b) -> Result<Vec<f32>>` | Concatenation `a ‖ b` (`cross_term.rs:106`). |

The exact agreement (cosine) kernel:

```rust
// cross_term.rs:46
let mut dot=0.0; let mut an=0.0; let mut bn=0.0;
for (x,y) in a.iter().zip(b) { dot += x*y; an += x*x; bn += y*y; }
if an <= f32::EPSILON || bn <= f32::EPSILON { return Err(ZERO_NORM); }
Ok(dot / (an.sqrt() * bn.sqrt()))
```

Validation: `ensure_same_dim_finite` rejects empty or mismatched-length inputs (`CALYX_LOOM_DIM_MISMATCH`) and any NaN/∞ (`CALYX_LOOM_NON_FINITE_VECTOR`). `delta`/`interaction` require equal dims; `concat` only requires finiteness (`cross_term.rs:112`).

## A2. Materialization policy (`materialization.rs`)

Decides which cross-terms are stored eagerly versus computed lazily.

| Type | Variants / Fields | File |
| --- | --- | --- |
| `MaterializationAction` | `EagerStore`, `LazyCache` | `materialization.rs:10` |
| `MaterializationEntry` | `a,b: SlotId`, `kind: CrossTermKind`, `action: MaterializationAction` | `materialization.rs:16` |
| `MaterializationPlan` | `entries: Vec<MaterializationEntry>`; method `materialized_count()` counts `EagerStore` entries | `materialization.rs:24` |
| `PairGainGate` (trait) | `pair_gain_bits(a,b: SlotId) -> f32` | `materialization.rs:37` |
| `StaticPairGainGate` | `gain_bits: f32` (returns constant) | `materialization.rs:42` |

**`plan_cross_terms` / `plan_cross_terms_checked`** (`materialization.rs:52`, `:57`) iterate over all unordered slot pairs `(i<j)` and emit **four** entries per pair with a fixed policy:

| Cross-term kind | Action |
| --- | --- |
| `Agreement` | always `EagerStore` |
| `Delta` | always `LazyCache` |
| `Interaction` | `EagerStore` iff `gain_bits >= 0.05`, else `LazyCache` |
| `Concat` | always `LazyCache` |

`plan_cross_terms` wraps the checked variant with an infallible gate adapter; the checked variant threads a fallible `FnMut(SlotId,SlotId)->Result<f32>` gain callback so an Assay-backed gate can fail closed.

## A3. In-memory store & agreement graph (`agreement_graph.rs`)

| Type | Fields | File |
| --- | --- | --- |
| `XtermRow` | `key: CrossTermKey`, `value: CrossTermValue`, `tag: SignalProvenanceTag` | `agreement_graph.rs:18` |
| `AgreementEdge` | `a,b: SlotId`, `raw_mean_agreement: f32`, `mean_agreement: f32`, `agreement_weight: f32`, `n: usize` | `agreement_graph.rs:25` |
| `LoomStore` | private: `xterm_cf: BTreeMap<CrossTermKey,XtermRow>`, `measured_tags: BTreeMap<(CxId,SlotId),SignalProvenanceTag>`, `cache: LruCache<CrossTermKey,CrossTermValue>` | `agreement_graph.rs:35` |

### `LoomStore` algorithms

- **`weave(cx, slots) -> Result<usize>`** (`:67`): tags every present slot `Measured`, then for every unordered slot pair computes `agreement_scalar` and inserts an `Agreement` `XtermRow` tagged `Derived`. Returns the inserted count. (Only agreement scalars are woven here.)
- **`materialize_plan(cx, slots, plan) -> Result<usize>`** (`:98`): tags slots `Measured`; for each plan entry whose action is `EagerStore`, canonicalizes the pair, skips if the key already exists, computes the cross-term via `compute_cross_term`, and inserts a `Derived` row.
- **`cross_term(cx,a,b,kind,slots) -> Result<CrossTermValue>`** (`:137`): the lazy read path — canonicalize, then check (1) the persisted `xterm_cf`, (2) the LRU `cache`; on miss, compute and **put into the cache** (not the CF) before returning. This is where "lazy cross-terms" materialize on demand.
- **`agreement_graph() -> Vec<AgreementEdge>`** (`:163`): folds every scalar-valued `XtermRow` into per-`(a,b)` `(sum, count)`, then emits one edge per pair with `raw = sum / max(n,1)`, `mean_agreement = raw`, and `agreement_weight = agreement_weight(raw).unwrap_or(0.0)` (cosine clamped to `[0,1]`). `raw_mean_agreement` and `mean_agreement` are presently identical.

`compute_cross_term` (`:233`) dispatches by kind to the math kernels; a missing slot fails `CALYX_LOOM_SLOT_MISSING`.

### Persistence

- `persist_xterms_to_aster(&mut CfRouter) -> Result<usize>` (`:191`): JSON-encodes each `XtermRow` under `xterm_key`, writes to `ColumnFamily::XTerm`, flushes, returns row count.
- `load_xterms_from_aster(&CfRouter, cap) -> Result<Self>` (`:202`): rebuilds the store from the XTerm CF; **fails closed** with `aster_corrupt_shard` if the stored key does not match the recomputed `xterm_key(row.key)`.
- `xterm_key` (`:219`): 21-byte key = `cx_id` (16) ‖ `a` BE u64 (... see SlotId encoding) ‖ `b` ‖ 1-byte kind tag (`Concat=0, Interaction=1, Agreement=2, Delta=3`).

## A4. LRU cache (`lru_cache.rs`)

`LruCache<K: Clone+Ord, V: Clone>` — `capacity` (clamped to `≥1`), a `BTreeMap` store and a `VecDeque` recency order. `get` clones the value and touches recency; `put` updates-in-place if present, otherwise evicts oldest (`order.pop_front`) until under capacity, then inserts. `touch` removes the key from the order deque and re-pushes it to the back (`lru_cache.rs`). Deterministic, single-threaded.

## A5. Abundance reporting (`abundance.rs`)

Honest reporting of how much DDA "signal" exists versus how much was materialized.

| Type | Variants / Fields | File |
| --- | --- | --- |
| `NeffEstimate` | `Provisional{value: f32}` \| `Computed{value, ci_low, ci_high: f32}` | `abundance.rs:7` |
| `CeilingEstimate` | `Provisional{bits: f32}` \| `Computed{bits: f32}` | `abundance.rs:20` |
| `AbundanceReport` | `n_lenses`, `c_n2_upper_bound`, `n_constellations`, `materialized`, `n_eff: NeffEstimate`, `dpi_ceiling: CeilingEstimate`, `measured_count`, `derived_count`, `meaning_compression_yield: f32` | `abundance.rs:26` |

`AbundanceReport::new(...)` derives `c_n2_upper_bound = cross_term_upper_bound(n_lenses)` and `meaning_compression_yield = meaning_compression_yield(materialized, n_constellations)` (`abundance.rs:39`).

| Function | Formula | File |
| --- | --- | --- |
| `cross_term_upper_bound(n)` | `n·(n−1)/2` (saturating), i.e. `C(n,2)` | `abundance.rs:64` |
| `dda_signal_yield(n_inputs, n_lenses)` | `n_inputs · (n_lenses + C(n_lenses,2) + 1)` (all saturating) | `abundance.rs:68` |
| `meaning_compression_yield(materialized, n_inputs)` | `NaN` if `n_inputs==0`, else `materialized / n_inputs` | `abundance.rs:75` |

## A6. Blind-spot detector (`blind_spot.rs`)

Cross-lens anomaly alert. `Severity` is `Low`/`Medium`/`High`. `detect_blind_spot(cx,a,b, lens_a_similarity, lens_b_neighbor_mean) -> Option<BlindSpotAlert>` (`blind_spot.rs:23`):

- `delta = lens_a_similarity − lens_b_neighbor_mean`.
- `delta < 0.5` ⇒ `None` (no alert).
- Severity thresholds: `delta ≥ 0.8 ⇒ High`; `≥ 0.65 ⇒ Medium`; else `Low`.
- `BlindSpotAlert { cx_id, a, b, delta, severity }`.

## A7. Error codes (`error.rs`)

`loom_error(code, message) -> CalyxError` attaches a fixed remediation string per code (`error.rs:32`). Constants:

| Constant | Meaning |
| --- | --- |
| `CALYX_LOOM_ZERO_NORM_VECTOR` | agreement requested on a zero-norm vector |
| `CALYX_LOOM_DIM_MISMATCH` | xterm inputs differ in length / empty |
| `CALYX_LOOM_NON_FINITE_VECTOR` | NaN/∞ in a slot vector |
| `CALYX_LOOM_SLOT_MISSING` | requested cx/slot vectors not loaded |
| `CALYX_LOOM_FORGE_UNAVAILABLE` | GPU agreement path requires `cuda` feature/Forge |
| `CALYX_LOOM_SERIES_READ_ERROR` | recurrence series read failed |
| `CALYX_LOOM_TEMPORAL_XTERM_CORRUPT` | temporal xterm row malformed |
| `CALYX_RECURRENCE_CONTEXT_TOO_LARGE` | occurrence context blob too large |
| `CALYX_RECURRENCE_INVALID_RETENTION` | non-positive retention |
| `CALYX_REACTIVE_REGISTRY_FULL` | trigger/subscription registry at cap |
| `CALYX_REACTIVE_QUEUE_FULL` | fired-event queue overflowed (oldest discarded) |
| `CALYX_REACTIVE_DRAIN_OVERFLOW` | subscription drain buffer overflowed |
| `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND` | unknown subscription id |
| `CALYX_REACTIVE_SIGNAL_UNAVAILABLE` | signal source cannot evaluate this condition |
| `CALYX_REACTIVE_ROW_CORRUPT` | durable reactive CF row/key malformed |

## A8. Reactive trigger engine (`reactive/`)

A bounded, audited subsystem that evaluates trigger conditions immediately after each ingest. "Bounded by construction (A26)" — every queue/registry/log has a hard cap.

### Caps & ids (`reactive/mod.rs`)

| Constant | Value |
| --- | --- |
| `DEFAULT_MAX_TRIGGERS` | 1024 |
| `DEFAULT_MAX_QUEUE_DEPTH` | 4096 |
| `DEFAULT_MAX_AUDIT_ENTRIES` | 65536 |
| `DEFAULT_MAX_SUBSCRIPTIONS` (`subscription.rs`) | 256 |
| `DEFAULT_MAX_DRAIN_BUF` (`subscription.rs`) | 1024 |

`TriggerId` is a UUID (v7, time-ordered).

### Conditions (`TriggerCondition`)

| Variant | Fields | Fires when |
| --- | --- | --- |
| `NewRegion` | `tau_override: Option<f32>` | novelty source returns `NoveltyVerdict::NewRegion` at calibrated τ (or override) |
| `EventRecurs` | `series: CxId`, `min_occurrences: u32` | occurrence count *increments* across the bar |
| `DriftDetected` | `slot: SlotId`, `drift_threshold: f32` | `|Δcosine|` for the slot `≥ drift_threshold` |

`NoveltyVerdict` = `NewRegion` | `Grounded`.

### Types

- `TriggerDef { id, condition, created_at: Ts, owner: Option<String> }`.
- `TriggerFired { trigger_id, cx_id, fired_at: Ts, ledger_ref: LedgerRef, condition_snapshot }`.
- `AuditEntry { eval_id: Uuid, trigger_id, cx_id, matched: bool, ts, ledger_ref, code: Option<String> }` — one immutable record per evaluation; `code` is set only for bounded-resource warnings.
- `BoundedQueue<T>` — FIFO ring; `push` returns `Some(oldest)` when capacity forces an eviction.
- `AuditLog` — append-only ring capped at `max_entries`, evicting oldest.
- `TriggerRegistry` — `register` fails `CALYX_REACTIVE_REGISTRY_FULL` at cap without disturbing existing entries.
- `ReactiveSignals` (trait): `novelty(cx,tau_override)`, `occurrence_count(series)`, `slot_drift(slot)` — implementors **must fail closed**.

### Engine evaluation (`reactive/engine.rs`)

`ReactiveEngine` holds `registry`, `queue: BoundedQueue<TriggerFired>`, `audit_log`, `clock: Arc<dyn Clock>`, `last_count: HashMap<TriggerId,u64>`, and `subscriptions`. Constructors: `new(clock)`, `with_caps(...)`, `with_subscription_caps(...)`.

**`evaluate_post_ingest(cx_id, ingest_ledger_ref, signals) -> Result<usize>`** (steps):
1. Snapshot registry defs (so the queue/audit/cursor can mutate during iteration).
2. For each def: call `evaluate_condition` (propagating signal-source errors — fail closed); append exactly one `AuditEntry`.
3. If matched: increment counter, build `TriggerFired` with a condition snapshot, dispatch to subscriptions, push to `queue`; on overflow record a `CALYX_REACTIVE_QUEUE_FULL` warning audit entry and cache the error.
4. Return fired count, or the cached overflow error after the batch.

**`evaluate_condition`** per variant:
- `NewRegion`: `signals.novelty(cx, tau_override) == NewRegion`.
- `EventRecurs`: `current = occurrence_count(series)`, `last = last_count.insert(id,current).unwrap_or(0)`, `threshold = max(min_occurrences,1)`; fire iff `current > last && last < threshold && current >= threshold` (fires exactly once, on the increment that crosses the bar).
- `DriftDetected`: `slot_drift(slot) >= drift_threshold`.

`drain_fired()` empties the queue oldest-first; `registry()`, `queue()`, `audit_log()` are read-only views.

### Signal sources (`reactive/signals.rs`)

- `RecurrenceSignals<C>` — backs `EventRecurs` from a durable `SeriesStore::occurrence_count`; novelty/drift error `CALYX_REACTIVE_SIGNAL_UNAVAILABLE`.
- `WardNoveltySignals<C>` — `{vault, profile: GuardProfile, matched_cx, high_stakes}`; `verdict` injects any `tau_override` into the profile, reads required slot vectors for produced vs. matched constellations, calls `calyx_ward::guard`, and maps a `NoveltyAction::NewRegion` action to `NewRegion`.
- `AgreementDriftTracker` — `Mutex<BTreeMap<SlotId,Vec<f32>>>` of previous vectors; `drift` computes `cosine = agreement_scalar(current, prior)` then drift `= (1.0 − cosine).abs()` (0.0 on first observation), updating the stored vector.
- `AgreementDriftSignals<C>` — delegates `slot_drift` to the tracker.
- `ReactiveSignalSet<C>` — composite: recurrence always present, `.with_ward_novelty(...)` and `.with_agreement_drift(...)` opt-in.

### Subscriptions (`reactive/subscription.rs`)

`SubscriptionId` (UUID v7, `Display`/`FromStr`). `SubscriptionHandle { id, trigger_id, condition, max_drain_buf, drain_buf, overflowed }` — `push` discards the oldest and sets `overflowed` when the per-subscription drain buffer is full. `SubscriptionDelta { subscription_id, events, overflowed }`. `SubscriptionStore` dispatches a fired event to every subscription matching its `trigger_id`. `observe_delta` errors if unknown or overflowed; `observe_delta_report` resets the flag and returns the structured delta. Engine helpers: `subscribe`, `unsubscribe`, `observe_delta[_report|_stream]`, and the durable variants `subscribe_durable` / `unsubscribe_durable` (persist a ledger entry, roll back on failure).

### Durable reactive rows (`reactive/durable.rs`)

Reactive CF key is `REACTIVE_KEY_LEN = 41` bytes = `tag(1)` ‖ `trigger_id(16)` ‖ `ledger_seq(8 BE)` ‖ `tail_id(16)`, with `AUDIT_TAG=0x01`, `FIRED_TAG=0x02`. `ReactiveRowKind` = `Audit|Fired`; `ReactiveRowKey { kind, trigger_id, ledger_seq, tail_id }`. Functions: `reactive_audit_key`, `reactive_fired_key`, `reactive_audit_prefix`, `reactive_row_key` (parse + validate length/tag), `decode_audit_entry`, `decode_trigger_fired`. `evaluate_post_ingest_durable` persists every audit/fired row (and queue-full warnings) to the vault's reactive CF with ledger accounting before returning.

## A9. Recurrence & periodic series (`recurrence/`)

`recurrence/mod.rs` re-exports occurrence/series types from `calyx_aster::recurrence` (`Occurrence`, `OccurrenceContext`, `RecurrenceSeries`, `RetentionPolicy`, `RollupSummary`, `StoredRecurrenceRow`, `RecurrenceReadStats`, encode/decode helpers, `FREQUENCY_SCALAR`, `MAX_CONTEXT_BYTES`).

### Periodic fitting (`recurrence/periodic.rs`)

Constants: `SECS_PER_HOUR=3600`, `SECS_PER_DAY=86_400`, `UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO=3`.

`PeriodicFit` fields: `target_hour: Option<u8>` (0..=23), `target_day_of_week: Option<u8>` (0..=6, Monday=0), `target_hour_day: Option<PeriodicTimeBucket>`, `tz_offset_secs: i32`, `dominant_period_secs: Option<f64>`, `support`, `active_support`, `rolled_support: u64`, `rollup_period_estimate_secs: Option<f64>`, `hour_confidence`, `day_confidence`, `hour_day_confidence: f32`.

**`periodic_fit_with_tz_offset(occurrences, tz_offset_secs)`** steps:
1. Map each occurrence time to local `(hour, day_of_week)`.
2. `mode(...,24,...)` → dominant hour + confidence (`max_count/n`); ties ⇒ `None`.
3. `mode(...,7,...)` → dominant day; `hour_day_mode` over the `24×7` grid → dominant `(hour,day)`.
4. Cadence = median inter-occurrence period (`recurrence::cadence_secs`).
5. Mode returns `(None,0.0)` for `< 2` occurrences.

`periodic_time_bucket(time_secs, tz_offset)` (`:..`): `local = time + tz_offset` (saturating); `hour = (local % 86400)/3600`; `day_of_week = (local/86400 + 3) mod 7`.

`PeriodicRecallQuery { target_hour, target_day_of_week, tz_offset_secs }` validates hour ≤23, day ≤6, at least one set; `matches(fit)` requires the pattern to match **and** `fit.active_support >= 2`. `recurrence_series[_with_tz_offset]` reads a series + computes its fit; `periodic_recall[_readback]` scans all recurrence cx_ids, fits each, collects sorted `PeriodicRecallHit`s, and returns `PeriodicRecallStats` instrumentation.

### Temporal cross-terms / lead-lag (`recurrence/cross_terms.rs`)

`LeadLagResult { cx_a, cx_b, lead_lag_secs: f64, n_pairs, proximity_window_secs }`.

- `co_occurrence_pairs(a,b,window) -> Vec<(EpochSecs,EpochSecs)>`: all `(t_a,t_b)` with `|t_b−t_a| < window` (empty if `window==0`).
- `lead_lag_secs(a,b,window) -> Option<LeadLagResult>`: self-pair (`cx_a==cx_b`) returns `0.0` lead-lag if `≥3` occurrences; otherwise requires `≥3` co-occurrence pairs, then returns the **median** of `t_b−t_a`.
- `temporal_cross_term(cx_a, cx_b, vault, window) -> Result<Option<LeadLagResult>>`: reads both series via `SeriesStore`, computes lead-lag, and for distinct pairs persists via `vault.put_temporal_xterm`. Read failures map to `CALYX_LOOM_SERIES_READ_ERROR`.
- Serialization: `encode_lead_lag_result`/`decode_lead_lag_result` use a fixed `VALUE_LEN=45`-byte layout with magic `b"LLAG1"`; decode fails `CALYX_LOOM_TEMPORAL_XTERM_CORRUPT` on bad length/magic/non-finite value.

### Series store & signature (`recurrence/series_store.rs`, `signature.rs`)

`SeriesStore<C>` wraps a vault + `RetentionPolicy`, exposing `append_occurrence[_observed_at]`, `read_series`, `recurrence_series[_with_tz_offset]`, `occurrence_count`, `periodic_recall[_readback]`. `signature.rs` re-exports `detect_recurrence_signature`, `temporal_slot_ids_for_panel`, `SignatureResult`, and `CALYX_RECURRENCE_SLOT_MISSING` from `calyx_aster::dedup`.

---

# Part B — calyx-assay (signal bits)

## B1. Shared estimate types (`estimate.rs`)

| Type | Variants / Fields | File |
| --- | --- | --- |
| `TrustTag` | `Trusted`, `Provisional` | `estimate.rs:8` |
| `EstimatorKind` | `Ksg`, `HistogramNmi`, `LogisticProbe`, `Bootstrap`, `PanelSufficiency`, `OutcomeEntropy`, `PairGain` | `estimate.rs:15` |
| `MiEstimate` | `bits`, `ci_low`, `ci_high: f32`, `n_samples`, `estimator: EstimatorKind`, `trust: TrustTag` | `estimate.rs:26` |

`MiEstimate::new` clamps `bits = max(bits,0)`, `ci_low = clamp(ci_low, 0..=bits)`, `ci_high = max(ci_high, bits)` (`estimate.rs:36`). `point(...)` builds a band `±max(|bits|·0.15, 0.02)` (`:57`).

**Trust gating** (`:63`): `trust_for_anchor(Some(anchor))` returns `Trusted` only when `is_grounded_anchor` holds — `source` non-empty after trim, `confidence` finite and in `(0,1]`. Otherwise `Provisional`. `provisional_without_anchor(_) -> Provisional` always demotes. `require_grounded_anchor` errors `assay_insufficient_samples` if the anchor is not grounded. This is the crate's central provenance/trust rule: **no anchor ⇒ Provisional**.

## B2. Sample validation (`samples.rs`)

`validate_rectangular_finite(name, samples) -> Result<usize>` (`samples.rs:3`): requires ≥1 dimension, every row to share the first row's dimension, and all values finite; returns the dimension. Failures use `assay_insufficient_samples`.

## B3. Mutual-information estimators

### B3.1 KSG continuous & continuous–discrete (`ksg.rs`)

`MIN_ASSAY_SAMPLES = 50` (`ksg.rs:13`). Bootstrap config: `(DEFAULT_BOOTSTRAP_RESAMPLES=200, DEFAULT_BOOTSTRAP_SEED=0)`.

| Function | Inputs → Output |
| --- | --- |
| `ksg_mi_continuous(x,y,k)` | two `&[Vec<f32>]` + `k` → `MiEstimate` (KSG), trust `Provisional` |
| `ksg_mi_continuous_with_anchor(x,y,k,anchor)` | trust from anchor |
| `ksg_mi_continuous_discrete(x, labels, k)` | continuous `x` + `&[usize]` labels → one-hot encodes labels into `y`, then KSG |
| `ksg_mi_continuous_discrete_with_anchor(...)` | as above with trust |

**Validation** (`validate_sample_counts`, `ksg.rs:121`): fail `assay_insufficient_samples` unless `left==right`, `left ≥ 50`, `0 < k < left`.

**KSG estimator core** (`ksg_bits_from_validated_samples`, `ksg.rs:58`):

```rust
for i in 0..n {
    let eps = kth_joint_radius(x, y, i, k);          // k-th Chebyshev radius in joint space
    let nx = neighbor_count(x, i, eps);              // count_j (j!=i, chebyshev(x_i,x_j) < eps)
    let ny = neighbor_count(y, i, eps);
    let local = digamma(k) + digamma(n)
              - digamma(nx + 1) - digamma(ny + 1);
    local_bits.push(local / LN_2);                   // nats → bits
}
mean(local_bits).max(0.0)                            // KSG-1 estimator, floored at 0
```

`kth_joint_radius` = the `k`-th smallest of `max(chebyshev(x_i,x_j), chebyshev(y_i,y_j))`, floored at `f32::EPSILON`. `chebyshev` is the L∞ distance. `digamma(x)` uses an upward-recurrence-then-asymptotic series (`ksg.rs:156`). The CI comes from `bootstrap_paired_ci` re-running the point estimator on resampled pairs.

### B3.2 Histogram NMI (`nmi.rs`)

`NmiReport { nmi, mi_bits, x_entropy_bits, y_entropy_bits, bins, n_samples }`.

**`partitioned_histogram_nmi(x,y,bins) -> Result<NmiReport>`** (`nmi.rs:20`):
1. Validate paired length, `≥ MIN_ASSAY_SAMPLES`, finite (`bins` is floored at 2).
2. Bin each series into `bins` equal-width buckets (min/max scaled, last bucket inclusive).
3. `hx, hy` = Shannon entropies (bits) of the binned marginals; `hxy` = entropy of the joint `(xb,yb)` pairs.
4. `mi = max(hx + hy − hxy, 0)`.
5. `nmi = mi / sqrt(hx·hy)` if `sqrt(hx·hy) > 0`, else `0`.

Entropy: `−Σ p·log2(p)` over bucket counts (`nmi.rs:93`).

### B3.3 Logistic-probe MI (`logistic.rs`)

`LogisticProbeReport { estimate: MiEstimate, accuracy: f32, selected_field: &'static str }` (the field is the literal `"logistic_probe"`).

`logistic_probe_mi(samples, labels: &[bool])` and `..._with_anchor` require `samples.len()==labels.len()` and `≥ MIN_ASSAY_SAMPLES` (the gate-driven variants accept a configurable `min_samples`). **Algorithm** (`logistic_summary`, `logistic.rs:113`):
1. `class_means` → positive/negative feature centroids.
2. `direction = pos_mean − neg_mean`; `midpoint = (pos_mean + neg_mean)/2`; `threshold = midpoint·direction`.
3. Predict `label̂_i = (row_i·direction >= threshold)` (a linear discriminant along the class-mean axis — not an iteratively fit logistic regression).
4. `accuracy` = fraction correct.
5. `bits = binary_mi(labels, predictions)` — the 2×2 confusion MI: `Σ_{y,p} P(y,p)·log2( P(y,p) / (P(y)·P(p)) )`, floored at 0 (`logistic.rs:173`).

CI via `bootstrap_paired_ci`. `EstimatorKind::LogisticProbe`.

### B3.4 Estimator summary table

| Estimator | Function | Inputs | Output bits formula | Trust default |
| --- | --- | --- | --- | --- |
| KSG (continuous) | `ksg_mi_continuous` | `x,y: &[Vec<f32>]`, `k` | `mean_i[ ψ(k)+ψ(n)−ψ(nx+1)−ψ(ny+1) ] / ln2`, floored 0 | Provisional |
| KSG (cont/discrete) | `ksg_mi_continuous_discrete` | `x`, `labels: &[usize]`, `k` | one-hot `y` then KSG | Provisional |
| Histogram NMI | `partitioned_histogram_nmi` | `x,y: &[f32]`, `bins` | `mi = max(Hx+Hy−Hxy,0)`; `nmi = mi/√(Hx·Hy)` | n/a (no trust field) |
| Logistic probe | `logistic_probe_mi` | `samples: &[Vec<f32>]`, `labels: &[bool]` | 2×2 confusion MI of mean-axis predictions | Provisional |
| Pair gain | `AssayGate::pair_gain` | left/right `&[Vec<f32>]`, labels | `max(pair_bits − max(left,right), 0)` | Provisional |

## B4. Bootstrap CIs (`bootstrap.rs`)

`DEFAULT_BOOTSTRAP_RESAMPLES=200`, `DEFAULT_BOOTSTRAP_SEED=0`. RNG is deterministic `ChaCha8Rng::seed_from_u64`.

- `bootstrap_mean_ci[_with_config]` — resamples values, returns `BootstrapCi { mean, ci_low, ci_high, resamples }`.
- `bootstrap_paired_ci(left, right, point, config, estimator)` — jointly resamples paired slices (same indices), reruns a fallible estimator, returns `Ok(None)` when empty / mismatched / `resamples==0`.

`ci_from_estimates` (`bootstrap.rs:95`): sort estimates; take the 2.5% / 97.5% percentile indices; `bootstrap_span = max(p97.5 − p2.5, 0)`; then `ci_low = min(p2.5 − span, point)`, `ci_high = max(p97.5 + span, point)`. Percentile index = `round((len−1)·p)` clamped. (This widens the band by one span on each side and always brackets the point estimate.)

## B5. Lens differentiation contract (`contract.rs`)

The admission gate that decides whether a lens earns a slot.

| Constant | Value | File |
| --- | --- | --- |
| `MIN_SIGNAL_BITS` | `0.05` | `contract.rs:8` |
| `MAX_PAIRWISE_CORR` | `0.6` | `contract.rs:9` |

`AdmissionDecision { admitted: bool, signal_bits: f32, max_pairwise_corr: f32, stratified_override: bool }`.

| Contract field | Source / Rule |
| --- | --- |
| `signal_bits` | the lens MI in bits being judged |
| `max_pairwise_corr` | worst correlation against existing lenses |
| `admitted` | `true` only if both gates pass |
| `stratified_override` | set by `admit_lens_with_strata` |

**`admit_lens(signal_bits, max_pairwise_corr)`** → `decide(..., false)`. **`decide`** (`contract.rs:37`) fails closed:
- non-finite `signal_bits` ⇒ `CALYX_ASSAY_LOW_SIGNAL`; non-finite corr ⇒ `CALYX_ASSAY_REDUNDANT`.
- `signal_bits < 0.05` ⇒ `CALYX_ASSAY_LOW_SIGNAL`.
- `max_pairwise_corr > 0.6` ⇒ `CALYX_ASSAY_REDUNDANT`.
- otherwise `admitted = true`.

**`admit_lens_with_strata(strata, corr)`** (`contract.rs:23`) judges on `strata.effective_bits` and sets `stratified_override = (effective_bits ≥ 0.05) && (global_bits < 0.05) && any stratum.sole_carrier)` — i.e. a rare-stratum sole carrier can be admitted even when global bits fall short.

### Stratified bits (`stratified.rs`)

`StratumBits { name, bits, frequency, sole_carrier }`; `StratifiedBits { global_bits, effective_bits, strata, no_frequency_multiplier }`. `stratified_bits(global_bits, strata)` sets `effective_bits = max(global_bits, max bits among sole-carrier strata)` and `no_frequency_multiplier = true` (frequency is recorded but **not** used to weight bits) (`stratified.rs:21`).

### PRD-22 formula wrappers (`formulas.rs`)

| Wrapper | Behaviour | File |
| --- | --- | --- |
| `lens_signal(bits, corr)` | delegates to `admit_lens` | `formulas.rs:7` |
| `pair_redundancy(corr)` | `|corr|`; fails `CALYX_ASSAY_REDUNDANT` if `> 0.6` | `formulas.rs:11` |
| `marginal_value(panel, panel_without_lens)` | `max(panel − panel_without_lens, 0)` | `formulas.rs:26` |
| `dpi_ceiling(panel_outcome_bits)` | passthrough of non-negative bits (the Data-Processing-Inequality ceiling) | `formulas.rs:32` |

## B6. AssayGate facade (`gate.rs`)

`AssayGate { min_samples: usize }`, default `min_samples = 50`. Wraps the logistic probe.

- `lens_signal(samples, labels) -> LensSignal{ estimate }` / `lens_signal_with_anchor(...)`.
- `pair_gain(left, right, labels) -> PairGain`: computes `left_signal`, `right_signal`, and a `pair_signal` over the concatenated features, then `pair_gain_from_estimates`.
- `PairGain { left_bits, right_bits, pair_bits, gain_bits, ci_low, ci_high, n_samples }` where `gain_bits = max(pair_bits − max(left,right), 0)`, `ci_low = max(pair.ci_low − max(left.ci_high,right.ci_high), 0)`, `ci_high = max(pair.ci_high − max(left.ci_low,right.ci_low), gain_bits)` (`gate.rs:130`).
- `pair_gain_estimate[_with_anchor]` wraps a `PairGain` as a `MiEstimate` of kind `PairGain`.

`loom_adapter.rs` (`AsterAssayMaterializationGate<S: VaultStore>`) is the bridge that lets Loom's `plan_cross_terms_checked` call this gate: it loads per-`cx_id` slot vectors + a bool anchor for a pair, runs `AssayGate::pair_gain`, and exposes `materialization_plan` plus `*_fail_safe_lazy` variants (errors collapse to `0.0` gain, recorded in a `Mutex<Option<CalyxError>>`). See A2 for the resulting policy.

## B7. Effective rank / n_eff (`n_eff.rs`, plus TC variant)

`NeffReport { n_eff, trace, frobenius_sq }`. **`stable_rank(matrix) -> NeffReport`** (`n_eff.rs:12`):

```
trace        = Σ_i matrix[i][i]
frobenius_sq = Σ_{i,j} matrix[i][j]^2
n_eff        = trace^2 / frobenius_sq   (0 if frobenius_sq == 0)
```

This is the stable-rank / participation-ratio measure of panel redundancy. A second, distinct `n_eff_from_tc` (Part B11) derives effective rank from total correlation.

## B8. Per-sensor attribution (`attribution.rs`)

`SlotAttribution { slot: SlotId, marginal_bits: f32, sole_carrier: bool }`; `BitsReport { slots, total_bits, trust }`.

`per_sensor_attribution(slot_bits: &[(SlotId,f32)], sole_threshold_bits)` (`attribution.rs:22`): a slot is `sole_carrier` iff its bits `≥ threshold` **and** it is the *only* slot at/above threshold (`strong_slots == 1`). `bits_report` sums `marginal_bits` into `total_bits`; `bits_report` always demotes trust to Provisional, while `bits_report_with_anchor` derives trust from the anchor.

## B9. Panel sufficiency & deficit routing (`sufficiency.rs`)

`PanelSufficiency { panel_bits, anchor_entropy_bits, sufficient: bool, deficit_bits, deficits: Vec<SufficiencyDeficit>, trust }`.

`DeficitSuggestedAction` = `AddOutcomeAnchor | ProposeLens | IncreaseSamples`. `DeficitRoutingContext { panel_id: String, anchor: AnchorKind, computed_at_seq: u64 }` (default `panel:unspecified`, `AnchorKind::Reward`, seq 0). `SufficiencyDeficit { panel_id, anchor, slot: Option<SlotId>, per_slot_gaps: BTreeMap<SlotId,f32>, deficit_bits, suggested_action, computed_at_seq, reason }`.

**Algorithm** (`panel_sufficiency_with_trust`, `sufficiency.rs:143`):
1. `deficit_bits = max(anchor_entropy_bits − panel_bits, 0)`.
2. `sufficient = panel_bits >= anchor_entropy_bits`.
3. If sufficient ⇒ no deficits. Else `localized_deficits`:
   - **No slots**: a single panel-level deficit, action `AddOutcomeAnchor`, reason "panel below anchor entropy".
   - **With slots**: distribute `deficit_bits` across slots by inverse-marginal weight `w_i = 1/(marginal_bits_i + 0.01)`; each slot's share is `deficit_bits · w_i / Σw`, action `ProposeLens`. `per_slot_gaps` carries the same distribution.

`entropy_bits(labels)` is the Shannon entropy (bits) helper (`sufficiency.rs:167`). The four public entry points (`panel_sufficiency`, `..._with_anchor`, `..._with_context`, `..._with_anchor_and_context`) differ only in trust derivation and routing context; the non-anchor ones force Provisional via `provisional_without_anchor`. `SufficiencyDeficitSink` + `InMemoryDeficitSink` + `PanelSufficiency::route_to` push deficits to a sink.

## B10. Cache provenance & vault scoping (`store.rs`)

The Assay result cache, with mandatory vault/anchor scoping.

| Type | Fields | File |
| --- | --- | --- |
| `AssayCacheKey` | `vault_id: Option<VaultId>`, `anchor: AnchorKind`, `panel_version: u32`, `corpus_shard: String` | `store.rs:15` |
| `AssaySubject` | `Lens{slot}` \| `Pair{a,b}` \| `Panel` \| `OutcomeEntropy` | `store.rs:62` |
| `AssayRow` | `cache_key`, `subject`, `estimate: MiEstimate`, `provenance: String`, `written_at_seq: u64` | `store.rs:70` |
| `AssayStore` | `rows: BTreeMap<(AssayCacheKey,AssaySubject),AssayRow>` | `store.rs:79` |

- `AssayCacheKey::new(...)` is **`#[deprecated]`** (unscoped); `scoped(panel_version, corpus_shard, vault_id, anchor)` is the supported constructor. `require_scoped()` fails `CALYX_VAULT_ACCESS_DENIED` when `vault_id` is `None`.
- `put / get / cache_hit / invalidate_panel(panel_version) / rows / len`.
- `persist_to_aster(router)` and `persist_to_vault(vault)` both call `aster_rows()`, which **calls `require_scoped()` on every row before encoding** — unscoped rows fail closed before any write.
- `load_from_aster / load_from_vault` decode rows, re-check `require_scoped`, and verify the stored key equals the recomputed `assay_key`; a mismatch fails `CALYX_ASTER_CORRUPT_SHARD`.
- `assay_key` (`store.rs:194`) = `panel_version` (BE u32) ‖ len-prefixed `vault_id` ‖ len-prefixed JSON `anchor` ‖ len-prefixed `corpus_shard` ‖ a subject tag (`Lens=0+slot`, `Pair=1+a+b`, `Panel=2`, `OutcomeEntropy=3`). This makes cache entries provenance-scoped per (vault, anchor, panel version, corpus shard, subject). The `provenance` string on each row records the producing stage.

## B11. Advanced estimators

These are exhaustively re-exported from `lib.rs` and implemented in dedicated modules. Constants are exact.

### B11.1 Bayesian posteriors (`bayesian.rs`)

Constants: `DEFAULT_BAYES_PRIOR_ALPHA=1.0`, `DEFAULT_BAYES_PRIOR_BETA=1.0`, `BAYESIAN_POSTERIOR_KEY_PREFIX=b"bayesian/posterior/v1"`, error `CALYX_BAYES_INVALID_INTERVAL`.

- **`GammaPoisson { alpha, beta: f64 }`** — conjugate Poisson-rate posterior. `update(events, interval)`: `alpha += events`, `beta += interval`. `mean_rate = alpha/beta`. `credible_interval[_95]` via `gamma_rate_quantile` (doubling-then-bisection on the regularised incomplete gamma). `next_occurrence_expected = 1/mean_rate`.
- **`BetaBernoulli { alpha, beta: f64 }`** — conjugate success-probability posterior. `update(successes, failures)`: `alpha += successes`, `beta += failures`. `mean_consistency = alpha/(alpha+beta)`. `reliability_probability(threshold) = 1 − I_threshold(alpha,beta)`; `is_reliable(threshold, confidence)`; CIs via `beta_quantile` (bisection on the regularised incomplete beta).
- `BayesianPosteriorRow { domain_id, outcome_anchor: AnchorKind, gamma_poisson, beta_bernoulli, written_at_seq }`. `bayesian_posterior_key/persist_bayesian_posterior/bayesian_posterior_for_domain/{gamma_poisson,beta_bernoulli}_for_domain` read/write the Assay CF (defaulting to priors `(1,1)`).

### B11.2 MMD drift & change point (`mmd.rs`)

Constants: `MIN_MMD_SAMPLES=4`, `MAX_MMD_SAMPLES=2048`, `DEFAULT_MMD_PERMUTATIONS=99`, `DEFAULT_MMD_ALPHA=0.01`, `DEFAULT_MMD_SEED=609`.

`MmdConfig { bandwidth: Option<f64>, permutations, seed, alpha }`. `MmdReport { n_a, n_b, dimension, bandwidth, mmd2, null_mean, critical_value, p_value, significant }`. `ChangePointReport { split_index, left_n, right_n, report }`.

`gaussian_mmd_with_config`: bandwidth defaults to the median pairwise Euclidean distance; observed statistic `MMD² = mean κ(X,X') + mean κ(Y,Y') − 2·mean κ(X,Y)` with Gaussian kernel `κ(a,b)=exp(−‖a−b‖²/(2σ²))`; a seeded permutation null (ChaCha8) yields `p_value = (count_geq + 1)/(permutations + 1)` (add-one) and `critical_value = quantile(null, 1−alpha)`; `significant = p_value ≤ alpha && mmd2 > critical_value`. `mmd_change_point` scans every split `s ∈ [min_window, n−min_window]` for the max MMD² (fixed bandwidth) and reruns the test at the best split.

### B11.3 Transfer entropy (`transfer_entropy.rs`)

Constants: `MIN_TE_QUORUM=30`, `DEFAULT_TE_WINDOW=1`, `DEFAULT_TE_K=3`, `DEFAULT_TE_BOOTSTRAP_RESAMPLES=500`, `DEFAULT_TE_BOOTSTRAP_SEED=52`, `DEFAULT_TE_LAGS=[1,2,4,8]`, error `CALYX_TE_INSUFFICIENT_SAMPLES`.

`Direction = AToB|BToA|Unclear`. `TEResult { t_a_to_b, t_b_to_a, dominant_direction, ci_95, t_b_to_a_ci_95, difference_ci_95, lag, window_size, provisional, n_samples, error_code, trust, computed_at }`. `TransferEntropyConfig { window_size, k, bootstrap_resamples, bootstrap_seed }`.

`T(A→B) = I(B_future; [A_past, B_past]) − I(B_future; B_past)`, each MI estimated by the KSG continuous estimator, clamped `≥0`. Below `MIN_TE_QUORUM`/`MIN_ASSAY_SAMPLES` the result is `provisional`. Bootstrap (with-replacement, seeded) yields the three 95% CIs. `dominant_direction = AToB` iff `t_a_to_b > t_b_to_a && a_to_b_ci.low > b_to_a_ci.high` (symmetric for `BToA`), else `Unclear`. `transfer_entropy_sweep[_with_config]` runs the lag set; `max_transfer_entropy_lag` returns the non-provisional lag maximizing `t_a_to_b`.

### B11.4 Total correlation & interaction information (`total_correlation.rs`)

Constants: `MIN_QUORUM_TC_PER_SLOT=50`, `DEFAULT_TC_K=3`, `DEFAULT_TC_BOOTSTRAP_RESAMPLES=500`, error `CALYX_TC_INSUFFICIENT_SAMPLES`.

`IISign = Redundant|Synergistic|Unclear`. `TCResult { tc, n_eff, ci_95, n_samples, slot_count, sum_marginal_entropy, joint_entropy, provisional, error_code, trust, computed_at }`. `IIResult { ii, sign, ci_95, n_samples, provisional, error_code, trust, computed_at }`.

- `TC(Φ) = Σ_k H(slot_k) − H(Φ)`, clamped `≥0`, with KSG differential entropy `h_bits = [ψ(n) − ψ(k) + d·(ln2 + mean_log_radius)] / ln2` (`entropy_bits_ksg`).
- `min_quorum_tc(slot_count) = 50·slot_count`.
- **`n_eff_from_tc(slot_count, tc, sum_marginal_entropy)`**: `0` for 0 slots, `1` for 1 slot; otherwise `n·(1 − tc/denom)` (denom = `sum_marginal_entropy` if `> eps`, else `max(tc,eps)`), clamped to `[1, n]`. A distinct effective-rank notion from `stable_rank` (B7).
- `interaction_information(a,b,c)` computes `II = I(A;B) − [I(A;[B;C]) − I(A;C)]` via KSG; sign from CI: `ci.low>0 ⇒ Redundant`, `ci.high<0 ⇒ Synergistic`, else `Unclear`.

### B11.5 Lomb-Scargle periodicity (`periodicity.rs`)

Constants: `MIN_PERIODICITY_SAMPLES=8`, `DEFAULT_PERIODOGRAM_OVERSAMPLE=10.0`, `DEFAULT_FAP_PERMUTATIONS=100`, `DEFAULT_PERIODICITY_SEED=0`, `DEFAULT_MAX_PEAKS=3`, `SIGNIFICANT_PEAK_FAP=0.01`, `MAX_FREQUENCY_GRID=1<<20`, `MAX_ACF_SAMPLES=8192`.

`PeriodogramConfig { oversample, min_frequency, max_frequency, fap_permutations, seed, max_peaks }`. `PeriodogramPeak { frequency, period=1/f, power∈[0,1], false_alarm_probability }`. `PeriodicityReport { frequencies, powers, peaks, n_samples, time_span, trust }` (`dominant()`, `significant_peaks(max_fap)`). `AutocorrelationReport { lags, coefficients, pair_counts, slot_width, dominant_period, n_samples, trust }`.

`lomb_scargle[_with_config|_with_anchor]`: validate (≥8 samples, strictly increasing times, nonzero variance); build a regular frequency grid (spacing `1/(oversample·span)`); compute the generalised (floating-mean) Lomb-Scargle power per Zechmeister & Kürster; rank interior local maxima by power (≤ `max_peaks`); assign FAP via a seeded permutation null `(count_geq + 1)/(perms + 1)`. `bin_event_counts` bins event timestamps into uniform bins; `autocorrelation` computes slotted (irregular-sampling) autocorrelation and reports a `dominant_period` as the smallest lag whose local max ≥ `0.8·`strongest.

### B11.6 Inter-event hazard & CUSUM (`recurrence_hazard.rs`)

Constants: `MIN_HAZARD_GAPS=3` (≥4 occurrences), `MIN_CUSUM_GAPS=4` (≥5), `DEFAULT_OVERDUE_ALPHA=0.05`, `CV_DETERMINISTIC=1.0e-6`, `DEFAULT_CUSUM_SLACK_K=0.5`, `DEFAULT_CUSUM_THRESHOLD_H=5.0`, `DEFAULT_MIN_SIGMA_FRAC=1.0e-3`.

`RateShift = SpeedUp|SlowDown`. `InterEventHazardReport` (fields incl. `mean_gap`, `gap_variance`, `coefficient_of_variation = σ/μ`, `gamma_shape = μ²/σ²`, `gamma_scale = σ²/μ`, `deterministic`, `elapsed`, `survival`, `hazard`, `empirical_survival`, `expected_next`, `overdue_threshold_secs`, `alpha`, `overdue`, `trust`). The renewal model uses a Gamma fit (deterministic step function when `CV ≤ 1e-6`); `survival S(elapsed)=Q(k, elapsed/θ)`, `hazard = pdf/S`, `overdue = S ≤ alpha`. `CusumReport`/`CusumChangePoint`/`CusumConfig` implement Page's two-sided CUSUM on standardized gaps with reference `k=0.5σ`, decision interval `h=5σ`, reporting the change-point onset, alarm index, and `RateShift`.

### B11.7 Recurrence anchors & oracle self-consistency (`recurrence_anchor.rs`)

Constants: `CONSISTENT_AGREEMENT_THRESHOLD=0.75`, `DEFAULT_OUTCOME_ANCHOR_LABEL="OutcomeAnchor"`, error `CALYX_ASSAY_MISSING_OUTCOME_SLOT`.

`RecurrenceAnchor { cx_id, frequency, cadence_secs }`. `Domain { id, cx_ids, outcome_anchor: AnchorKind }`. `OutcomeAgreement = Consistent{rate} | Flaky{rate} | Insufficient{n}`. `outcome_agreement_from_observations`: `Insufficient` if `< 3` observations; otherwise `agreement_rate = agreeing_pairs / C(n,2)`, classified `Consistent` if `≥0.75` else `Flaky`. `oracle_self_consistency[_from_agreements]` averages agreement rates across a domain's series (frequency ≥ 3), returning `1.0` if empty.

### B11.8 Random projection (`projection.rs`)

`ProjectionReport { input_rows, input_dim, output_dim, projected, seed }`. `target_projection_dim(rows, dim) = min(dim, max(2·ceil(log2 rows), 1))` (`= min(dim,1)` for `rows ≤ 1`). `project_cpu` applies a deterministic ±1 sign matrix (BLAKE3 of `seed,in_col,out_col`) scaled by `1/√output_dim`. `project_gpu` always fails `forge_device_unavailable` (no GPU implementation).

### B11.9 Special functions (`special_fn.rs`)

Deterministic, fail-closed (no silent NaN). `gammp(a,x)`/`gammq(a,x)` = regularised lower/upper incomplete gamma (series for `x < a+1`, Lentz continued fraction otherwise; results clamped `[0,1]`). `ln_gamma(z)` = Lanczos `g=7` approximation with the reflection formula for `z<0.5`. Internal tolerances: `GAMMA_ITMAX=300`, `GAMMA_EPS=3.0e-14`, `GAMMA_TINY=1.0e-300`. These back the Bayesian quantiles and the hazard survival/quantile functions.

### B11.10 Formula coverage catalog (`formula_catalog.rs`)

A self-documenting PRD-22 test matrix. Constants: `FORMULA_COVERAGE_SURFACE="formula-coverage"`, `FORMULA_COVERAGE_ARTIFACT_KIND="prd22.formula-coverage.v1"`, `FORMULA_COVERAGE_SCHEMA_VERSION=1`, `FORMULA_COVERAGE_SOT_KEY="formula_coverage/prd22"`, error `CALYX_FORMULA_COVERAGE_MISSING`. `FormulaCoverageStatus = Covered|Missing`. `FormulaRowSpec`/`FormulaCoverageRow` map each formula → `{prd_ref, engine, callable, tunable_params, test, fsv_root, status}`. `formula_coverage_artifact/json` emit the artifact; `validate_formula_coverage` checks surface/kind/schema/completeness. `self_tuning_representatives = ["rrf.k", "ksg.k"]`.

---

## Cross-crate integration map

- **Loom planning ⇄ Assay gain**: Loom's `plan_cross_terms_checked` (A2) accepts a fallible gain callback; `calyx-assay`'s `AsterAssayMaterializationGate` (B6) supplies it from real `AssayGate::pair_gain` bits. Interaction cross-terms are stored eagerly only when gain `≥ 0.05`.
- **Loom recurrence ⇄ Assay hazard/anchors**: `inter_event_hazard_from_series` and `recurrence_rate_cusum_from_series` (B11.6) and the recurrence anchors (B11.7) consume `calyx_aster::recurrence::RecurrenceSeries`, the same series Loom's `SeriesStore` writes (A9).
- **Reactive `EventRecurs`** (A8) reads occurrence counts from the same durable recurrence store.
- **Trust/provenance**: the Assay `TrustTag` rule (B1) and the vault-scoped `AssayStore` keys (B10) jointly enforce that only grounded-anchor, vault-scoped results are trusted/persisted.

## Items not determined from source

- The exact byte encoding of `SlotId` inside `xterm_key`/`assay_key` (relies on `SlotId::get().to_be_bytes()` from `calyx-core`) — **see [04_core_foundation.md](04_core_foundation.md)**.
- Ward novelty internals (`calyx_ward::guard`, `GuardProfile`, `NoveltyAction`) — invoked by `WardNoveltySignals` but defined outside these crates.
- Aster CF/vault storage internals (`CfRouter`, `AsterVault`, `ColumnFamily::{XTerm,Assay,reactive}`) — defined in `calyx-aster`.
- Graph-kernel consumers of the agreement graph — **see [10_graph_kernel.md](10_graph_kernel.md)**.
