# calyx-loom — DDA Cross-Terms & Agreement Graph

**Source files covered:**
- `crates/calyx-loom/src/lib.rs`
- `crates/calyx-loom/src/cross_term.rs`
- `crates/calyx-loom/src/agreement_graph.rs`
- `crates/calyx-loom/src/materialization.rs`
- `crates/calyx-loom/src/abundance.rs`
- `crates/calyx-loom/src/blind_spot.rs`
- `crates/calyx-loom/src/lru_cache.rs`
- `crates/calyx-loom/src/error.rs`
- `crates/calyx-loom/src/reactive/mod.rs`
- `crates/calyx-loom/src/reactive/engine.rs`
- `crates/calyx-loom/src/reactive/signals.rs`
- `crates/calyx-loom/src/reactive/subscription.rs`
- `crates/calyx-loom/src/reactive/durable.rs`
- `crates/calyx-loom/src/recurrence/mod.rs`
- `crates/calyx-loom/src/recurrence/cross_terms.rs`
- `crates/calyx-loom/src/recurrence/periodic.rs`
- `crates/calyx-loom/src/recurrence/series_store.rs`
- `crates/calyx-loom/src/recurrence/signature.rs`
- `crates/calyx-loom/tests/cross_term_fail_closed.rs`
- `crates/calyx-loom/Cargo.toml`
- Cross-checked against `docs/dbprdplans/06_LOOM_DDA_ENGINE.md`

The crate-level doc comment (`src/lib.rs:1`) reads: *"Loom DDA cross-term and agreement-graph engine."*

---

## 1. What "DDA" computes

**DDA = Derived Data Abundance** (`docs/dbprdplans/06_LOOM_DDA_ENGINE.md`, title: "Loom: the Derived Data Abundance Engine"). The acronym is not spelled out in the Rust source itself; the source uses the bare token "DDA" in function names (`dda_signal_yield`) and comments. The doc below derives every behavioral claim from code.

DDA is the idea that `n` real inputs measured through `N` frozen lenses (slots) yield far more than `N` signals per input, because Loom also derives **cross-terms**: one association per *pair* of slots, plus the whole-panel constellation signal. The per-input signal yield is computed in `abundance::dda_signal_yield` (`src/abundance.rs:68`):

```
per_input  = N + C(N,2) + 1
yield      = n_inputs * per_input
```

where `C(N,2) = N·(N-1)/2` is `cross_term_upper_bound` (`src/abundance.rs:64`). All arithmetic uses saturating ops (no overflow panic).

Loom tags every materialized signal as **`Measured`** (a real input through a frozen lens) or **`Derived`** (a cross-term) — see `SignalProvenanceTag` (`src/cross_term.rs:20`). `LoomStore::weave` tags inputs `Measured` and writes cross-terms `Derived` (`src/agreement_graph.rs:67`).

### 1.1 What is implemented vs. planned

The plan (`06_LOOM_DDA_ENGINE.md` §6) describes Forge/CUDA batched matmul, Assay-gated `pair_gain`, and Sextant-driven Concat promotion. In the **code**:
- The Assay/Sextant gates are abstracted behind the `PairGainGate` trait (`src/materialization.rs:37`); the only concrete impl is `StaticPairGainGate` (a fixed constant). There is no live Assay/Sextant call inside calyx-loom.
- The interaction formula in code is a plain Hadamard (elementwise) product. The plan's "low-rank `vₐᵀW vᵦ`" form is **not** present.
- GPU agreement (`agreement_batch_gpu`) is gated behind the optional `cuda` feature; without it, it returns `CALYX_LOOM_FORGE_UNAVAILABLE` (`src/cross_term.rs:79`).

---

## 2. Cross-term computations (exact formulas)

Defined in `src/cross_term.rs`. Each takes two slot vectors `a, b: &[f32]`. Validation is fail-closed (see §2.2).

### 2.1 The four kinds and their formulas

`CrossTermKind` (`src/cross_term.rs:13`): `Agreement | Delta | Interaction | Concat` (serde `snake_case`).

| Kind | Function | Inputs | Output | Exact formula |
|---|---|---|---|---|
| Agreement | `agreement_scalar(a,b) -> Result<f32>` | two equal-len vectors | scalar `f32` | cosine: `Σ aᵢbᵢ / (√Σaᵢ² · √Σbᵢ²)` |
| Delta | `delta_vec(a,b) -> Result<Vec<f32>>` | two equal-len vectors | vector | elementwise `aᵢ − bᵢ` |
| Interaction | `interaction_vec(a,b) -> Result<Vec<f32>>` | two equal-len vectors | vector | elementwise (Hadamard) `aᵢ · bᵢ` |
| Concat | `concat_vec(a,b) -> Result<Vec<f32>>` | two vectors (any lens lengths) | vector len `|a|+|b|` | `[a‖b]` (a then b, copied) |

Notes from the code:
- Agreement requires non-zero norms: if `Σaᵢ² ≤ f32::EPSILON` or `Σbᵢ² ≤ f32::EPSILON` it returns `CALYX_LOOM_ZERO_NORM_VECTOR` (`src/cross_term.rs:56`). The cosine is the raw value and is **not** clamped here (it can be negative; test `cross_term_fail_closed.rs:16` asserts orthogonal vectors give `0.0`).
- `Concat` is the only kind that does **not** require equal dimensions; it only checks finiteness (`concat_vec`, `src/cross_term.rs:106`).

### 2.2 Agreement weight (edge clamping)

`agreement_weight(raw_cosine: f32) -> Result<f32>` (`src/cross_term.rs:65`): errors `CALYX_LOOM_NON_FINITE_VECTOR` if not finite, otherwise returns `raw_cosine.clamp(0.0, 1.0)`. So negative cosines are clamped to 0 when used as an edge weight (test `cross_term_fail_closed.rs:17`: `agreement_weight(-1.0) == 0.0`).

### 2.3 Batch agreement (CPU / GPU parity)

| Function | Behavior |
|---|---|
| `agreement_batch_cpu(pairs: &[(&[f32],&[f32])]) -> Result<Vec<f32>>` | maps `agreement_scalar` over each pair (`src/cross_term.rs:75`) |
| `agreement_batch_gpu(pairs) -> Result<Vec<f32>>` | empty → `Ok(vec![])`; with `cuda` feature → `agreement_batch_cuda`; without → `Err(CALYX_LOOM_FORGE_UNAVAILABLE)` (`src/cross_term.rs:79`) |
| `agreement_batch_cuda` (`cuda` only) | builds `CudaBackend`, calls `backend.cosine(left,right,len,&mut score)` per pair; any failure → `CALYX_LOOM_FORGE_UNAVAILABLE` (`src/cross_term.rs:134`) |

### 2.4 Input validation (fail-closed)

`ensure_same_dim_finite` (`src/cross_term.rs:112`): errors `CALYX_LOOM_DIM_MISMATCH` if `a.len() != b.len()` **or** either is empty; then `ensure_finite` on both. `ensure_finite` (`src/cross_term.rs:123`) errors `CALYX_LOOM_NON_FINITE_VECTOR` on any NaN/∞. Confirmed by `tests/cross_term_fail_closed.rs`.

### 2.5 Cross-term value types

| Type | Definition | File |
|---|---|---|
| `CrossTermKey` | `{ cx_id: CxId, a: SlotId, b: SlotId, kind: CrossTermKind }`; derives `Ord, Hash` | `src/cross_term.rs:27` |
| `CrossTermValue` | enum `Scalar(f32) | Vector(Vec<f32>)` (serde `snake_case`) | `src/cross_term.rs:35` |
| `canonical_pair(a,b)` | returns `(min,max)` so `(a,b)` and `(b,a)` map to one key | `src/cross_term.rs:42` |

---

## 3. The agreement graph data structure

### 3.1 LoomStore (the store)

`LoomStore` (`src/agreement_graph.rs:34`) is the in-memory cross-term column-family plus an LRU cache.

| Field | Type | Role |
|---|---|---|
| `xterm_cf` | `BTreeMap<CrossTermKey, XtermRow>` | eager (materialized) cross-terms |
| `measured_tags` | `BTreeMap<(CxId,SlotId), SignalProvenanceTag>` | which (cx,slot) were measured |
| `cache` | `LruCache<CrossTermKey, CrossTermValue>` | lazy-computed cross-terms |

`XtermRow` (`src/agreement_graph.rs:17`): `{ key: CrossTermKey, value: CrossTermValue, tag: SignalProvenanceTag }`.

Methods: `new(cache_capacity)`, `tag_measured(cx,slot)`, `measured_count()`, `xterm_count()`, `cache_count()`, `weave(...)`, `materialize_plan(...)`, `cross_term(...)`, `agreement_graph()`, `xterm_rows()`, `persist_xterms_to_aster(router)`, `load_xterms_from_aster(router, cap)`.

### 3.2 The graph nodes & edges

The "graph" is returned by `LoomStore::agreement_graph() -> Vec<AgreementEdge>` (`src/agreement_graph.rs:163`). It is an **undirected weighted edge list**, not a stored adjacency structure.

- **Nodes** = `SlotId`s (the slots/lenses). They are implicit — never materialized as their own type.
- **Edges** = `AgreementEdge` (`src/agreement_graph.rs:24`):

| Field | Type | Meaning |
|---|---|---|
| `a`, `b` | `SlotId` | the slot pair (canonicalized `a ≤ b`) |
| `raw_mean_agreement` | `f32` | mean of stored scalar agreements for this pair |
| `mean_agreement` | `f32` | identical to `raw_mean_agreement` in current code |
| `agreement_weight` | `f32` | `agreement_weight(raw)` (clamped 0..1; `0.0` on error) |
| `n` | `usize` | number of scalar agreement rows aggregated for this pair |

Note: `mean_agreement == raw_mean_agreement` — the code sets both to the same `raw` value (`src/agreement_graph.rs:178`). There is no separate normalization step yet.

### 3.3 Persistence (Aster XTerm CF)

`persist_xterms_to_aster` serializes each `XtermRow` to JSON under a binary key and `put`s it into `ColumnFamily::XTerm`, then flushes (`src/agreement_graph.rs:191`). The key (`xterm_key`, `src/agreement_graph.rs:219`) is 21 bytes:

```
cx_id (16 bytes) ‖ a.get() (u32 BE, 4 bytes? ) ‖ b.get() (BE) ‖ kind_tag (1 byte)
kind_tag: Concat=0, Interaction=1, Agreement=2, Delta=3
```

(`out` is reserved with capacity 21; `a.get()`/`b.get()` are written big-endian.) `load_xterms_from_aster` iterates the CF, JSON-decodes each row, and verifies `entry.key == xterm_key(row.key)`, erroring `aster_corrupt_shard` on mismatch (`src/agreement_graph.rs:202`). Round-trip is covered by `xterms_roundtrip_through_aster_cf` (`src/agreement_graph.rs:262`).

---

## 4. Algorithm steps

### 4.1 Building the graph — `weave(cx, slots)` (`src/agreement_graph.rs:67`)

Input: `cx: CxId`, `slots: &BTreeMap<SlotId, Vec<f32>>`. Returns count inserted.

1. Tag every slot in `slots` as `Measured` (`tag_measured`).
2. Collect slot ids (sorted, since `BTreeMap`).
3. For each pair `(i<j)` → `(a,b)`: compute `agreement_scalar(slots[a], slots[b])` (fail-closed: a bad vector aborts the whole weave).
4. Insert an `XtermRow` keyed `{cx, a, b, Agreement}` with `CrossTermValue::Scalar`, tag `Derived`; increment counter.

So `weave` materializes **only Agreement scalars** for all `C(N,2)` pairs — matching the plan's "Agreement = always eager" rule.

### 4.2 Building via a plan — `materialize_plan(cx, slots, plan)` (`src/agreement_graph.rs:98`)

1. Tag all slots `Measured`.
2. For each plan entry whose `action == EagerStore`: canonicalize `(a,b)`, build the key, skip if already present, else `compute_cross_term(a,b,kind,slots)` and insert as `Derived`.

`compute_cross_term` (`src/agreement_graph.rs:233`) looks up both slot vectors (missing → `CALYX_LOOM_SLOT_MISSING`) and dispatches: Agreement→`Scalar(agreement_scalar)`, Delta→`Vector(delta_vec)`, Interaction→`Vector(interaction_vec)`, Concat→`Vector(concat_vec)`.

### 4.3 Lazy query — `cross_term(cx,a,b,kind,slots)` (`src/agreement_graph.rs:137`)

1. Canonicalize `(a,b)`; build key.
2. If present in `xterm_cf` → return stored value.
3. Else if present in `cache` → return cached value.
4. Else compute via `compute_cross_term`, `cache.put(key, value)`, return. (Cache is bounded LRU.)

### 4.4 Aggregating the graph — `agreement_graph()` (`src/agreement_graph.rs:163`)

1. Walk all `xterm_cf` rows; for each `CrossTermValue::Scalar` (only Agreement scalars qualify), accumulate `(sum, count)` into a `BTreeMap<(a,b)>`.
2. For each pair: `raw = sum / max(n,1)`; emit an `AgreementEdge` with `mean_agreement = raw`, `agreement_weight = agreement_weight(raw).unwrap_or(0.0)`, `n`.

Vector cross-terms (Delta/Interaction/Concat) do **not** contribute edges. Complexity: O(rows) over the materialized CF.

### 4.5 Materialization policy — `plan_cross_terms` (`src/materialization.rs:52`)

`plan_cross_terms(slots: &[SlotId], gate: &dyn PairGainGate) -> MaterializationPlan` (infallible wrapper around `plan_cross_terms_checked`). For each pair `(i<j)` it emits **four** entries:

| Kind | Action |
|---|---|
| Agreement | `EagerStore` (always) |
| Delta | `LazyCache` (always) |
| Interaction | `EagerStore` iff `gate.pair_gain_bits(a,b) ≥ 0.05`, else `LazyCache` |
| Concat | `LazyCache` (always) |

The `0.05`-bit threshold is the only gating constant (`src/materialization.rs:86`) and matches the plan's `≥ 0.05 bits` rule. `MaterializationAction` = `EagerStore | LazyCache` (`src/materialization.rs:10`). `MaterializationPlan { entries: Vec<MaterializationEntry> }` with `materialized_count()` (counts EagerStore). `MaterializationEntry { a, b, kind, action }`. `PairGainGate::pair_gain_bits(a,b) -> f32`; `StaticPairGainGate { gain_bits: f32 }` returns the fixed value regardless of slots.

### 4.6 LRU cache (`src/lru_cache.rs`)

`LruCache<K: Clone+Ord, V: Clone>` = `{ capacity, map: BTreeMap, order: VecDeque }`. `new(cap)` clamps `cap` to ≥1. `get` clones + touches (moves key to back). `put` overwrites+touches if present, else evicts from front while `len ≥ capacity`, then pushes. Deterministic, single-threaded.

---

## 5. Abundance reporting (`src/abundance.rs`)

The "honest dashboard" (plan §8). All serde `snake_case`.

| Type | Definition |
|---|---|
| `NeffEstimate` | enum `Provisional { value: f32 } | Computed { value: f32, ci_low: f32, ci_high: f32 }` |
| `CeilingEstimate` | enum `Provisional { bits: f32 } | Computed { bits: f32 }` |
| `AbundanceReport` | `{ n_lenses, c_n2_upper_bound, n_constellations, materialized, n_eff, dpi_ceiling, measured_count, derived_count, meaning_compression_yield }` |

`AbundanceReport::new(...)` computes `c_n2_upper_bound = cross_term_upper_bound(n_lenses)` and `meaning_compression_yield = materialized / n_constellations`.

Free functions:
- `cross_term_upper_bound(n) = n·(n−1)/2` (saturating).
- `dda_signal_yield(n_inputs, n_lenses) = n_inputs·(n_lenses + C(n_lenses,2) + 1)` (saturating).
- `meaning_compression_yield(materialized, n_inputs) = materialized / n_inputs`, or `f32::NAN` when `n_inputs == 0`.

The `n_eff` (effective rank) and `dpi_ceiling` (data-processing-inequality ceiling) are **carried as inputs** to the report (enum-tagged Provisional/Computed) — calyx-loom does not itself compute them; they are supplied by the caller (Assay; see [11_assay_signal_bits.md](11_assay_signal_bits.md)).

---

## 6. Blind-spot detector (`src/blind_spot.rs`)

Cross-lens anomaly detector. `detect_blind_spot(cx_id, a, b, lens_a_similarity, lens_b_neighbor_mean) -> Option<BlindSpotAlert>`:

1. `delta = lens_a_similarity − lens_b_neighbor_mean`.
2. If `delta < 0.5` → `None` (no anomaly).
3. Severity: `delta ≥ 0.8 → High`; `≥ 0.65 → Medium`; else `Low`.
4. Returns `BlindSpotAlert { cx_id, a, b, delta, severity }`.

`Severity` = `Low | Medium | High` (`src/blind_spot.rs:8`). The thresholds `0.5 / 0.65 / 0.8` are hard-coded.

---

## 7. Reactive trigger/subscription engine (`src/reactive/`)

A bounded, audited subsystem (labelled "PH72 · T02", A26 bounded-by-construction) that evaluates trigger conditions after each ingest.

### 7.1 Trigger conditions and definitions

`TriggerCondition` (`src/reactive/mod.rs:59`, serde `snake_case`):

| Variant | Data | Fires when |
|---|---|---|
| `NewRegion` | `{ tau_override: Option<f32> }` | Ward novelty verdict = `NewRegion` at calibrated τ (or override) |
| `EventRecurs` | `{ series: CxId, min_occurrences: u32 }` | recurrence count crosses `min_occurrences` on the incrementing ingest |
| `DriftDetected` | `{ slot: SlotId, drift_threshold: f32 }` | `|Δcosine|` for `slot` `≥ drift_threshold` |

`TriggerDef { id: TriggerId, condition, created_at: Ts, owner: Option<String> }`. `TriggerId = Uuid` (v7).
`TriggerFired { trigger_id, cx_id, fired_at: Ts, ledger_ref: LedgerRef, condition_snapshot: TriggerCondition }`.
`AuditEntry { eval_id: Uuid, trigger_id, cx_id, matched: bool, ts: Ts, ledger_ref, code: Option<String> }`.
`NoveltyVerdict = NewRegion | Grounded`.

### 7.2 Bounded containers (A26)

| Type | Default cap const | Overflow behavior |
|---|---|---|
| `TriggerRegistry` | `DEFAULT_MAX_TRIGGERS = 1024` | `register` fails closed → `CALYX_REACTIVE_REGISTRY_FULL`, registry untouched |
| `BoundedQueue<T>` (fired events) | `DEFAULT_MAX_QUEUE_DEPTH = 4096` | `push` discards & returns oldest (ring) |
| `AuditLog` | `DEFAULT_MAX_AUDIT_ENTRIES = 65536` | ring; oldest entry evicted |
| `SubscriptionStore` | `DEFAULT_MAX_SUBSCRIPTIONS = 256` / `DEFAULT_MAX_DRAIN_BUF = 1024` | drain buffer rings, sets `overflowed` |

All caps are clamped to ≥1 in constructors.

### 7.3 ReactiveEngine (`src/reactive/engine.rs`)

`ReactiveEngine` holds `registry`, `queue`, `audit_log`, `clock: Arc<dyn Clock>`, `last_count: HashMap<TriggerId,u64>`, `subscriptions`. Constructors: `new(clock)`, `with_caps(...)`, `with_subscription_caps(...)`.

`evaluate_post_ingest<S: ReactiveSignals>(cx_id, ingest_ledger_ref, signals) -> Result<usize>` (`src/reactive/engine.rs:92`):
1. Snapshot the registry defs (clone), iterate.
2. `evaluate_condition` per trigger (errors propagate = fail closed; no fire recorded).
3. Append exactly one `AuditEntry` per trigger (match or not).
4. On match: increment `fired`, build `TriggerFired`, dispatch to subscriptions, push to queue. On queue overflow: append a second `AuditEntry` with `code = CALYX_REACTIVE_QUEUE_FULL`, remember error.
5. Return fired count, or the queue-full error after the batch.

`evaluate_condition` (`src/reactive/engine.rs:150`) edge-detects recurrence: for `EventRecurs`, with `last = last_count.insert(id, current).unwrap_or(0)` and `threshold = max(min_occurrences,1)`, fires only when `current > last && last < threshold && current >= threshold` — i.e. exactly once on the crossing ingest (test `event_recurs_fires_only_when_threshold_crossed`). Other methods: `register`, `deregister`, `drain_fired`, `registry()`, `queue()`, `audit_log()`.

### 7.4 Signal sources (`src/reactive/signals.rs`)

`ReactiveSignals` trait: `novelty(cx_id, tau_override)`, `occurrence_count(series)`, `slot_drift(slot)` — all `Result`, **fail closed** (a source that can't answer returns `CALYX_REACTIVE_SIGNAL_UNAVAILABLE`). Concrete sources:
- `RecurrenceSignals` — answers `occurrence_count` from the durable `SeriesStore`; novelty/drift → unavailable.
- `WardNoveltySignals` — answers `novelty` via `calyx_ward::guard` (NewRegion if `NoveltyAction::NewRegion`); others unavailable.
- `AgreementDriftSignals` + `AgreementDriftTracker` — `slot_drift` reads the current dense slot vector, compares to the prior snapshot via `agreement_scalar`, returns `|1 − cos|` (`src/reactive/signals.rs:121`); 0 on first observation.
- `ReactiveSignalSet` — composite (recurrence always; Ward novelty and drift opt-in via `with_ward_novelty` / `with_agreement_drift`).

### 7.5 Subscriptions (`src/reactive/subscription.rs`)

`SubscriptionId(Uuid v7)` (Display/FromStr). `SubscriptionHandle { id, trigger_id, condition, max_drain_buf, drain_buf, overflowed }`. `SubscriptionDelta { subscription_id, events, overflowed }`. `SubscriptionStore` maps id→handle.
Engine extension methods: `subscribe` (registers a trigger + a subscription, rolls back the trigger on store-full), `unsubscribe`, `observe_delta` (errors `CALYX_REACTIVE_DRAIN_OVERFLOW` if overflowed), `observe_delta_report` (clears overflow flag, returns `SubscriptionDelta`), `observe_delta_stream`, `subscribe_durable`/`unsubscribe_durable` (append a ledger entry tagged `reactive_subscription_v1`, rolling back on ledger failure).

### 7.6 Durable rows (`src/reactive/durable.rs`)

`evaluate_post_ingest_durable<C,S>(vault, cx_id, ingest_ledger_ref, signals)` mirrors §7.3 but also persists each audit/fired row to `ColumnFamily::Reactive` via `write_cf_batch_with_ledger_entry` (ledger tag `reactive_state_v1`). Key layout (`REACTIVE_KEY_LEN = 41` bytes): `tag(1) ‖ trigger_id(16) ‖ ledger_seq(u64 BE, 8) ‖ tail_id(16)`, tags `AUDIT=0x01`, `FIRED=0x02`. Public: `ReactiveRowKind` (`Audit|Fired`), `ReactiveRowKey { kind, trigger_id, ledger_seq, tail_id }`, `reactive_audit_key`, `reactive_fired_key`, `reactive_audit_prefix`, `reactive_row_key` (parse), `decode_audit_entry`, `decode_trigger_fired`. Corrupt/unknown keys → `CALYX_REACTIVE_ROW_CORRUPT`.

---

## 8. Recurrence & temporal cross-terms (`src/recurrence/`)

Bounded recurrence-series storage over Aster's recurrence CF, plus the temporal (lead/lag) extension of DDA across time (plan §5).

### 8.1 Temporal cross-terms (`src/recurrence/cross_terms.rs`)

`LeadLagResult { cx_a: CxId, cx_b: CxId, lead_lag_secs: f64, n_pairs: usize, proximity_window_secs: u64 }`.

- `co_occurrence_pairs(series_a, series_b, window_secs) -> Vec<(EpochSecs,EpochSecs)>`: Cartesian over both series' occurrence times; keeps a pair when `|t_b − t_a| < window_secs` (strict). `window_secs==0` → empty.
- `lead_lag_secs(series_a, series_b, window_secs) -> Option<LeadLagResult>`: builds signed deltas `t_b − t_a` over co-occurring pairs, sorts by `f64::total_cmp`, and takes the **median** (even length → average of the two middle values). Requires `≥ 3` pairs (self-pair case: `≥ 3` occurrences, `lead_lag = 0.0`). Positive ⇒ B follows A.
- `temporal_cross_term<C>(cx_a, cx_b, vault, window_secs) -> Result<Option<LeadLagResult>>`: reads both series from the vault, computes lead/lag, and (when distinct cx and a result exists) persists via `vault.put_temporal_xterm`. Self-pair is not persisted. Read failures → `CALYX_LOOM_SERIES_READ_ERROR`.
- `encode_lead_lag_result` / `decode_lead_lag_result`: fixed **61-byte** big-endian wire format, magic `LLAG1`: `magic(5) ‖ cx_a(16) ‖ cx_b(16) ‖ lead_lag(f64,8) ‖ n_pairs(u64,8) ‖ window(u64,8)`. Non-finite lead/lag or bad magic/length → `CALYX_LOOM_TEMPORAL_XTERM_CORRUPT`.

### 8.2 Periodic fit & recall (`src/recurrence/periodic.rs`)

Detects time-of-day / day-of-week patterns. Day-of-week convention: **Monday = 0 … Sunday = 6** (epoch alignment constant `+3`).

- `periodic_time_bucket(time_secs, tz_offset_secs) -> PeriodicTimeBucket`: `local = time + tz_offset`; `hour = (local mod 86400)/3600`; `dow = (local/86400 + 3) mod 7`.
- `periodic_fit(occurrences)` / `_with_tz_offset`: histogram-mode over hours (24), days (7), and hour×day (168) buckets. Each `mode` needs ≥2 occurrences; a tie → `None`; `confidence = max_count / len`. Returns `PeriodicFit { target_hour, target_day_of_week, target_hour_day, tz_offset_secs, dominant_period_secs, support, active_support, rolled_support, rollup_period_estimate_secs, hour_confidence, day_confidence, hour_day_confidence }`. `dominant_period_secs` from `recurrence::cadence_secs`.
- `PeriodicRecallQuery { target_hour: Option<u8>, target_day_of_week: Option<u8>, tz_offset_secs }`; `new`/`with_tz_offset` validate (hour 0..=23, dow 0..=6, at least one set) else `CALYX_TEMPORAL_INVALID_PERIOD`. `matches(fit)` requires `fit.active_support ≥ 2`.
- `periodic_recall` / `periodic_recall_readback<C>(vault, query)`: scan all recurrence CxIds, read each series, keep matches; returns `PeriodicRecallHit { cx_id, frequency, occurrence_count, cadence_secs, periodic_fit }` plus `PeriodicRecallStats` (visited/decoded counters). `RecurrenceRead { series, periodic_fit, read_stats }`.

### 8.3 Series store & signature

- `SeriesStore<'a,C>` (`series_store.rs`): handle over `AsterVault` + `RetentionPolicy`. Delegating methods: `new`, `with_retention` (validates), `append_occurrence`, `append_occurrence_observed_at`, `read_series`, `recurrence_series(_with_tz_offset)`, `occurrence_count`, `periodic_recall(_readback)`.
- `signature.rs` is a **re-export facade** over `calyx_aster::dedup`: `SignatureResult` (`RecurrenceSignature { same_action: CxId, new_time: EpochSecs } | NewContent | ContentMismatch | SameTime`), `detect_recurrence_signature(...)`, `temporal_slot_ids_for_panel(...)`, and the constant `CALYX_RECURRENCE_SLOT_MISSING`. `detect_recurrence_signature` gates on content cosine, then compares temporal (E2/E3/E4) slots; identical temporal vectors (cosine `≥ 0.999999`) ⇒ `SameTime`, differing ⇒ `RecurrenceSignature`. The implementation lives in calyx-aster, not calyx-loom; see [06_aster_storage_engine.md](06_aster_storage_engine.md). `SignatureResult::NewContent` is defined but never produced by this function.

---

## 9. Error taxonomy (`src/error.rs`)

`loom_error(code, message) -> CalyxError` attaches a fixed remediation string per code. All codes are `&'static str` constants:

| Constant | Raised by |
|---|---|
| `CALYX_LOOM_ZERO_NORM_VECTOR` | zero-norm vector in `agreement_scalar` |
| `CALYX_LOOM_DIM_MISMATCH` | unequal/empty dims |
| `CALYX_LOOM_NON_FINITE_VECTOR` | NaN/∞ vector or non-finite weight |
| `CALYX_LOOM_SLOT_MISSING` | slot not in `slots` map |
| `CALYX_LOOM_FORGE_UNAVAILABLE` | GPU agreement without `cuda` / CUDA failure |
| `CALYX_LOOM_SERIES_READ_ERROR` | recurrence series read failure |
| `CALYX_LOOM_TEMPORAL_XTERM_CORRUPT` | bad temporal-xterm encode/decode |
| `CALYX_RECURRENCE_CONTEXT_TOO_LARGE` | (defined; used in recurrence ctx bounds) |
| `CALYX_RECURRENCE_INVALID_RETENTION` | non-positive retention |
| `CALYX_REACTIVE_REGISTRY_FULL` | registry/subscription store full |
| `CALYX_REACTIVE_QUEUE_FULL` | fired-queue overflow |
| `CALYX_REACTIVE_DRAIN_OVERFLOW` | subscription drain buffer overflow |
| `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND` | unknown subscription id |
| `CALYX_REACTIVE_SIGNAL_UNAVAILABLE` | source can't evaluate a condition |
| `CALYX_REACTIVE_ROW_CORRUPT` | bad durable reactive CF row/key |

`CALYX_RECURRENCE_SLOT_MISSING` and `CALYX_TEMPORAL_INVALID_PERIOD` originate outside this file (calyx-aster / calyx-core).

---

## 10. Dependencies & features (`Cargo.toml`)

Deps: `blake3`, `calyx-aster`, `calyx-core`, `calyx-ledger`, `calyx-ward`, `serde`, `serde_json`, `uuid` (v7). `calyx-forge` is **optional** (only with the `cuda` feature). Feature `cuda = ["dep:calyx-forge", "calyx-forge/cuda"]`; default features none. Dev-deps: `calyx-forge`, `proptest`.

---

## Gaps / not covered

- **Interaction is Hadamard only.** The plan's low-rank `vₐᵀW vᵦ` interaction is not implemented; no learned/random `W` matrix exists in code.
- **No live Assay/Sextant integration.** `PairGainGate` is the only seam; the sole impl `StaticPairGainGate` returns a constant. Concat is never auto-promoted to eager/ANN inside calyx-loom (always `LazyCache`); the plan's "Sextant promotes Concat" is absent.
- **`mean_agreement` == `raw_mean_agreement`.** No distinct normalization is applied to edge means despite the two separate fields.
- **`n_eff` / DPI ceiling are inputs, not computed here.** `AbundanceReport` carries them; calyx-loom does not derive effective rank or `I(panel;outcome)`.
- **GPU path untested without hardware.** `agreement_batch_cuda` requires the `cuda` feature and a working CUDA backend.
- **`SignatureResult::NewContent`** is defined but never produced by `detect_recurrence_signature`.
- **Graph is an edge list, not a persisted adjacency index.** `agreement_graph()` recomputes the edge list from the materialized XTerm CF on each call; there is no stored graph object or traversal/path API in calyx-loom (graph traversal lives in calyx-mincut/calyx-paths; see [17_graph_mincut_paths.md](17_graph_mincut_paths.md)).
