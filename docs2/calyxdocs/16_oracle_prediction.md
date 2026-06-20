# 16 — Oracle Prediction (calyx-oracle)

`calyx-oracle` is grounded consequence prediction. Given an action and a domain it
predicts the grounded outcome from **recurrence evidence**, expands a hop-attenuated
**butterfly tree** of downstream consequences, can walk **backward** from an outcome to
its causes, and — binding — passes every confidence reading through an **honesty gate**
that returns `Insufficient` (with a per-sensor deficit) when the panel cannot carry the
outcome's bits. It also exposes the six-tier per-domain **super-intelligence predicate**,
an energy-descent **completion** primitive, time-of-next-occurrence prediction, and a set
of pure PRD-22 formula functions.

**Source files covered:**
- `crates/calyx-oracle/src/lib.rs`
- `crates/calyx-oracle/src/types.rs`
- `crates/calyx-oracle/src/error.rs`
- `crates/calyx-oracle/src/honesty_gate.rs`
- `crates/calyx-oracle/src/predict.rs`
- `crates/calyx-oracle/src/predict/context.rs`
- `crates/calyx-oracle/src/butterfly.rs`
- `crates/calyx-oracle/src/butterfly/context.rs`
- `crates/calyx-oracle/src/reverse_query.rs`
- `crates/calyx-oracle/src/reverse_query_context.rs`
- `crates/calyx-oracle/src/self_consistency.rs`
- `crates/calyx-oracle/src/time_prediction.rs`
- `crates/calyx-oracle/src/energy.rs`
- `crates/calyx-oracle/src/complete.rs`
- `crates/calyx-oracle/src/prd22.rs`
- `crates/calyx-oracle/src/super_intel.rs`
- `crates/calyx-oracle/src/super_intel_full.rs`
- `crates/calyx-oracle/src/super_intel_types.rs`
- `crates/calyx-oracle/Cargo.toml`

Planning cross-checks: `docs/dbprdplans/21_ORACLE_AND_AGI.md`, `docs/dbprdplans/27_INTELLIGENCE_OBJECTIVE.md`.

Related docs: [11_assay_signal_bits.md](11_assay_signal_bits.md) (panel sufficiency / MI),
[13_ward_guard.md](13_ward_guard.md) (guard verdicts, calibration),
[12_lodestar_kernel.md](12_lodestar_kernel.md) (kernel recall),
[15_anneal_optimization.md](15_anneal_optimization.md) (β config, goodhart/regression reports),
[06_aster_storage_engine.md](06_aster_storage_engine.md) (recurrence series, vault).

---

## 1. Crate layout and dependencies

Workspace member, edition 2024. Library only; everything is re-exported flat from
`lib.rs`. The crate is a thin orchestration layer over other engines:

| Dependency | Used for |
|---|---|
| `calyx-assay` | `PanelSufficiency`, per-sensor attribution, KSG MI, entropy, `MIN_ASSAY_SAMPLES` |
| `calyx-aster` | `AsterVault`, `ColumnFamily::Base` scans, recurrence `read_series`, constellation decode |
| `calyx-core` | `AnchorValue`, `Panel`, `LensId`, `SlotId`, `Clock`, `LedgerRef`, `CalyxError`, `content_address` |
| `calyx-forge` | `cosine_batch`, `normalize_f32` (energy descent) |
| `calyx-ledger` | `EntryKind`, `SubjectId`, `ActorId` (provenance writes) |
| `calyx-lodestar` | `KernelIndex`, `RecallReport`, `kernel_recall_test_with_clock` |
| `calyx-paths` | `AssocGraph`, `reach_scored` (PRD-22 formula butterfly/reverse) |
| `calyx-ward` | `GuardVerdict`, `TrustedRegion`, `GuardProfile`, calibration meta |
| `calyx-anneal` | `AnnealConfig` (β), `GoodhartReport`, `RegressionReport`, `regression_rate` |

All vault-backed functions are generic over `C: Clock` and scan
`ColumnFamily::Base`, decode each constellation, and filter by domain metadata.

### 1.1 Domain / metadata keys (constellation matching)

Forward prediction, butterfly, reverse, and self-consistency all identify relevant
constellations by metadata keys. Every match accepts a primary key OR a fallback key:

| Constant | Value | Defined in |
|---|---|---|
| `ORACLE_DOMAIN_METADATA_KEY` | `oracle.domain` | `self_consistency.rs` |
| `ORACLE_FALLBACK_DOMAIN_METADATA_KEY` | `domain` | `self_consistency.rs` |
| `ORACLE_ACTION_METADATA_KEY` | `oracle.action` | `predict.rs` |
| (action fallback, private) | `action` | `predict.rs`, `butterfly.rs`, `reverse_query.rs` |
| `ORACLE_EFFECT_METADATA_KEY` | `oracle.effect` | `reverse_query.rs` |
| `ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY` | `oracle.structural_confidence` | `reverse_query.rs` |

`matches_domain` returns true when either domain key equals the requested domain string;
`matches_action` likewise for the two action keys.

---

## 2. Public contract types

### 2.1 Core types (`types.rs`)

| Type | Kind | Fields / notes |
|---|---|---|
| `DomainId` | struct (`transparent` newtype over `String`) | `new`, `as_str`, `Display`, `From<&str>`, `From<String>` |
| `Prediction` | struct | `outcome: AnchorValue`, `confidence: f32`, `consequences: Vec<Consequence>`, `bound: SufficiencyBound`, `provenance: LedgerRef`, `guard: GuardVerdict` |
| `SufficiencyBound` | struct | `i_panel_oracle: f32` (serde `I_panel_oracle`), `dpi_ceiling: f32`, `sufficient: bool`, `per_sensor_deficit: Vec<(LensId, f32)>` |
| `OracleSelfConsistency` | struct | `flakiness: f32`, `validity: f32`, `ceiling: f32`, `provisional: bool`, `provenance: Option<LedgerRef>` |
| `Consequence` | struct | `action_or_event: String`, `domain: DomainId`, `outcome: AnchorValue`, `confidence: f32`, `hop: u8`, `provenance: LedgerRef` |
| `ConsequenceTree` | struct | `root: Consequence`, `children: Vec<ConsequenceTree>`, `max_depth: u8`; ctor `ConsequenceTree::leaf` |
| `SlotSet` | type alias | `HashSet<LensId>` |
| `CompletionSlotPartition<'a>` | struct | `all_slots`, `clamp`, `free` (all `&SlotSet`) |
| `SlotTag` | enum | `Measured`, `Inferred`, `Provisional` (serde snake_case) |
| `TaggedSlot` | struct | `lens_id: LensId`, `vector: Vec<f32>`, `tag: SlotTag` |
| `CompletionResult` | struct | `filled_cx: Vec<TaggedSlot>`, `confidence: f32`, `converged: bool`, `energy: f32`, `provenance: LedgerRef`; helpers `inferred_slots`/`provisional_slots`/`measured_slots` |

`OracleSelfConsistency::ceiling` is **always computed** as `validity * (1.0 - flakiness)`
inside `with_provenance` (the only place the struct is built). `measured(...)` sets
`provisional=false`; `provisional(...)` sets it true.

`OracleSelfConsistency` ceiling is the hard cap referenced by the plan
(`τ_corr ≤ oracle_self_consistency`).

### 2.2 Constants

| Constant | Value | Module | Meaning |
|---|---|---|---|
| `DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH` | `4` (`u8`) | `types` | default tree max_depth |
| `MAX_DEPTH` | `= DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH` (4) | `butterfly` | hop cutoff |
| `HOP_ATTENUATION` | `0.7` | `butterfly` (also private in `predict`) | per-hop confidence multiplier |
| `MIN_CONFIDENCE_THRESHOLD` | `0.05` | `butterfly` | prune child below this attenuated confidence |
| `MAX_REVERSE_DEPTH` | `3` (`u8`) | `reverse_query` | backward walk cutoff |
| `STRUCTURAL_CONFIDENCE` (private) | `0.35` | `reverse_query` | default provisional cause confidence |
| `MIN_FLAKINESS_PAIRS` | `10` (`u64`) | `self_consistency` | min total pairs to compute flakiness |
| `MIN_VALIDITY_SAMPLES` | `= calyx_assay::MIN_ASSAY_SAMPLES` | `self_consistency` | min validity samples |
| `KSG_K` (private) | `3` | `self_consistency` | KSG estimator k |
| `SOLE_CARRIER_BITS` (private) | `0.10` | `honesty_gate` | per-sensor attribution threshold |
| `MIN_TIME_PREDICTION_OCCURRENCES` | `3` | `time_prediction` | min active occurrences for cadence |
| `FULL_CONFIDENCE_SUPPORT` (private) | `12.0` | `time_prediction` | support saturating confidence |
| `MAX_STEPS` | `20` | `energy` | energy descent iteration cap |
| `DEFAULT_EPS` | `1.0e-4` | `energy` | descent convergence epsilon |
| `DEFAULT_BETA` | `1.0` | `energy` | default softmax sharpness β |
| `ORACLE_CLEAN_THRESHOLD` | `0.7` | `super_intel` | tier-1 ceiling threshold |
| `KERNEL_RECALL_RATIO` | `0.95` | `super_intel` | tier-3 kernel-recall threshold |
| `GOODHART_THRESHOLD` | `0.9` | `super_intel_full` | tier-5 pass-rate threshold |
| `CALIBRATION_CEILING_DELTA` | `0.0` | `super_intel_full` | tier-4 threshold offset added to ceiling |
| `COMPLETION_LEDGER_TAG` | `oracle_completion_v1` | `complete` | ledger tag |

### 2.3 Error taxonomy (`error.rs`)

`OracleError` is a `Clone + PartialEq` enum. `.code()` returns the string constant;
`.remediation()` returns a fixed remediation string; `Display` is
`"{code}: {message}; remediation: {remediation}"`. `From<CalyxError>` wraps as
`AssayFailure`; `From<OracleError> for CalyxError` unwraps `AssayFailure` or builds a
`CalyxError { code, message, remediation }`.

| Variant | Payload | Code constant | Value |
|---|---|---|---|
| `Insufficient` | `bound: SufficiencyBound` | `CALYX_ORACLE_INSUFFICIENT` | `"CALYX_ORACLE_INSUFFICIENT"` |
| `FlakyAnchor` | `self_consistency: f32` | `CALYX_ORACLE_FLAKY_ANCHOR` | `"CALYX_ORACLE_FLAKY_ANCHOR"` |
| `NoRecurrence` | `domain: DomainId` | `CALYX_ORACLE_NO_RECURRENCE` | `"CALYX_ORACLE_NO_RECURRENCE"` |
| `DomainNotFound` | — | `CALYX_ORACLE_DOMAIN_NOT_FOUND` | `"CALYX_ORACLE_DOMAIN_NOT_FOUND"` |
| `LedgerWriteFailure` | — | `CALYX_ORACLE_LEDGER_WRITE_FAILURE` | `"CALYX_ORACLE_LEDGER_WRITE_FAILURE"` |
| `SlotConflict` | `overlap`, `missing`, `extra`, `tag_mismatch: Vec<LensId>` | `CALYX_ORACLE_SLOT_CONFLICT` | `"CALYX_ORACLE_SLOT_CONFLICT"` |
| `AssayFailure` | `source: CalyxError` | (delegates `source.code`) | varies |

Note: `FlakyAnchor` and `DomainNotFound` variants exist; `DomainNotFound` is returned by
`reverse_query` (no match found) and by `self_consistency::domain_series` (no domain
constellations). `FlakyAnchor` is **not constructed anywhere in this crate's `src/`**
(the flakiness/validity path returns `NoRecurrence` on insufficient samples) — it is part
of the public catalog only.

`time_prediction.rs` and `prd22.rs` do **not** use `OracleError`; they return
`calyx_core::Result` and build errors via `CalyxError::oracle_insufficient(...)`
(code `CALYX_ORACLE_INSUFFICIENT`).

---

## 3. The honesty gate (`honesty_gate.rs`)

The binding gate. `check_sufficiency` (vault) and `check_sufficiency_with_assay` (over the
`SufficiencyAssay` trait) return `Result<SufficiencyBound, OracleError>`.

### 3.1 Trait and the vault assay

```rust
pub trait SufficiencyAssay {
    fn panel_sufficiency(&self, panel: &Panel, domain: &DomainId, clock: &dyn Clock)
        -> Result<PanelSufficiency, OracleError>;
}
```

`VaultSufficiencyAssay<'a, C>` implements it by:
1. `AssayStore::load_from_vault(vault)`.
2. Build an `AssayCacheKey::scoped(panel.version, domain, vault_id, AnchorKind::Reward)`.
3. Read three classes of `AssayRow` via `required_row` — `AssaySubject::Panel`,
   `AssaySubject::OutcomeEntropy`, and one `AssaySubject::Lens { slot }` per panel slot.
   A missing row → `CalyxError::assay_insufficient_samples` wrapped as `AssayFailure`.
4. `bits(row)` extracts `row.estimate.bits`; rejects non-finite or negative (→
   `aster_corrupt_shard`).
5. `per_sensor_attribution(&slot_bits, SOLE_CARRIER_BITS=0.10)` builds attributions, then
   `panel_sufficiency_with_context(panel_bits, outcome_entropy_bits, attributions, trust, ctx)`.
   `ctx.panel_id = "oracle:{domain}:panel:{version}"`, `anchor = Reward`,
   `computed_at_seq = clock.now()`. `trust` defaults to `TrustTag::Provisional` if the
   panel row is absent.

### 3.2 EXACT insufficiency criteria

`check_sufficiency_with_assay` does the following (see code lines 47–66):

1. `validate_report` — `panel_bits` and `anchor_entropy_bits` must both be **finite and
   `>= 0.0`**; otherwise `AssayFailure(assay_insufficient_samples)`.
2. **Sufficiency test (the gate):**
   ```
   sufficient = report.panel_bits >= report.anchor_entropy_bits
   ```
   i.e. `I(panel; oracle) >= H(outcome)`. This is a `>=` comparison of raw bits; there is
   no slack/τ term in code (`CALIBRATION_CEILING_DELTA` is for the calibrated tier, not
   here).
3. The returned `SufficiencyBound`:
   - `i_panel_oracle = report.panel_bits`
   - `dpi_ceiling = report.panel_bits` (DPI ceiling = the panel's measured bits)
   - `sufficient`
   - `per_sensor_deficit` = empty if sufficient, else `lens_deficits(panel, report)`.
4. **If `sufficient`** → `Ok(bound)`. **If not** → `Err(OracleError::Insufficient { bound })`.

So the gate fires (returns `Insufficient`) precisely when `panel_bits < anchor_entropy_bits`.
A second failure path returns `AssayFailure` when an assay row is missing or bits are
invalid (this is `NoRecurrence`-adjacent: it means evidence is missing, not that the panel
is provably insufficient).

### 3.3 Deficit-reporting structure (`lens_deficits`)

When insufficient, the per-sensor deficit is built (lines 178–213):

1. If `panel.slots` is empty → return empty vec.
2. Build a per-slot gap map (`BTreeMap<SlotId, f32>`) from `report.deficits`: for each
   `deficit`, record `deficit.slot -> deficit.deficit_bits` (first wins) and **overwrite**
   from `deficit.per_slot_gaps` (`slot -> gap`).
3. For each panel slot, if it has a finite gap `> 0.0`, **accumulate** that gap onto its
   `lens_id` in `by_lens: BTreeMap<LensId, f32>`.
4. If `by_lens` is empty after that (insufficiency with no attributable per-sensor gap) →
   `Err(AssayFailure(assay_insufficient_samples))` with message
   `"oracle insufficiency lacks per-sensor deficit attribution"`.
5. Otherwise return `by_lens` as a sorted `Vec<(LensId, f32)>`.

Result: `SufficiencyBound.per_sensor_deficit` is a list of `(LensId, deficit_bits)` telling
the caller **which lens is short and by how many bits** — the ME-JEPA-style localized
deficit. Tests (`honesty_gate_tests.rs`) confirm a `0.46`-bit panel against a `1.0`-bit
outcome returns `Insufficient` with `dpi_ceiling == i_panel_oracle == 0.46` and a
non-empty per-sensor deficit; a `1.05`-bit panel returns `Ok` with an empty deficit.

---

## 4. Forward prediction (`predict.rs`)

`oracle_predict(vault, action: &Action, domain, clock) -> Result<Prediction, OracleError>`.

`Action` = `{ action_id: String, panel: Panel, guard: Option<GuardVerdict> }`.

### 4.1 Algorithm steps

1. **Honesty gate first:** `check_sufficiency(...)` — if insufficient, the whole call
   returns `Insufficient` before any prediction.
2. **Gather evidence** (`prediction_evidence`): scan `ColumnFamily::Base`, keep
   constellations whose domain metadata matches; for each, `read_series` and parse every
   non-empty occurrence context as `PredictionContext`. Keep observations whose action
   matches (`action_id`/`action` field, or base constellation action metadata) and that
   carry an outcome anchor (`outcome_anchor`, falling back to `oracle_verdict`). Each
   `OutcomeObservation` records `cx_id`, the outcome `AnchorValue`, its JSON label, and any
   `consequences`. **Empty observations → `NoRecurrence`.**
3. **Posterior** (`posterior`): bucket observations by outcome label, count each, sort by
   count desc then label asc. Top bucket is the predicted outcome.
   - `raw_confidence(top, second, total)` =
     `support * separation * sample_support`, clamped to `[0,1]`, where
     `support = top/total`, `separation = (top - second)/total`,
     `sample_support = total/(total+2)`.
4. **Self-consistency ceiling:** `oracle_self_consistency(...)` (§6) → `consistency.ceiling`.
5. **Confidence ceiling** (`apply_confidence_ceiling`): final confidence =
   `min(raw_confidence, self_consistency_ceiling, dpi_ceiling)` (each unit-clamped). This
   enforces the plan's "confidence capped at oracle self-consistency" and the DPI cap.
6. **Guard:** use `action.guard` if present, else a `provisional_guard` — `overall_pass =
   true`, `provisional = true`, per-slot `cos=1.0`, `tau=DEFAULT_TAU`, `pass=true`,
   guard id `018f48a4-9a79-74d2-8a5c-9ad7f6b8c104`.
7. **Ledger write** (`oracle_predict_v1`): digests of domain/action/outcome, source cx ids,
   recurrence counts (`top_count`, `second_count`, `distinct_outcomes`), raw + ceiling +
   final confidence. Write failure → `LedgerWriteFailure`.
8. **First-order consequences** (`first_order_consequences`): from observations whose
   outcome label == predicted label, bucket their `consequences` by
   `(action_or_event, domain, outcome_label)`, count each. Each emitted `Consequence` has
   `hop = 1` and `confidence = (confidence * HOP_ATTENUATION * count / predicted_count)`
   clamped to `[0, confidence]`.

Returns `Prediction { outcome, confidence, consequences, bound, provenance, guard }`.

### 4.2 `PredictionContext` shape (`predict/context.rs`)

Occurrence contexts are JSON. Recognized fields (all optional): `action`/`action_id`;
`outcome_anchor`/`oracle_verdict` (`{ "value": AnchorValue }`); `consequence` (single) and
`consequences` (list), each `ConsequenceEvidence { action_or_event, domain (default
"oracle"), outcome: { value } }`. Empty `action_or_event` consequences are dropped.

---

## 5. The butterfly consequence tree (`butterfly.rs`)

### 5.1 Data structure

`ConsequenceTree { root: Consequence, children: Vec<ConsequenceTree>, max_depth: u8 }`.
A recursive tree; `max_depth` is set to `MAX_DEPTH` (4) on every node. `Consequence.hop`
is the node depth (root hop is whatever the caller passed; children = parent hop + 1,
saturating).

### 5.2 Public entry points

| Function | Returns | Behavior |
|---|---|---|
| `build_tree(vault, root, clock)` | `ConsequenceTree` | builds the full tree from `root` |
| `expand(vault, consequence, clock)` | `Vec<Consequence>` | builds the tree, returns all descendants flattened (excludes root) |
| `select(tree, desired_outcome)` | `Option<&ConsequenceTree>` | best **terminal (leaf)** node by anchor score vs the desired outcome |
| `provisional_ledger_ref()` / `is_provisional_ledger_ref(&r)` | `LedgerRef` / `bool` | sentinel `{ seq: u64::MAX, hash: [0;32] }` |

### 5.3 Branch generation and scoring (`build_tree_internal` → `expand_node`)

Iterative-recursive DFS with a `BTreeSet<NodeKey>` (domain+action) visited set seeded with
the root:

1. `stats.nodes_visited += 1`.
2. **Depth limit:** if `node.root.hop >= MAX_DEPTH (4)` → prune (`depth_prunes`), stop.
3. **Confidence limit:** `child_confidence = attenuate(node.confidence) =
   clamp01(confidence) * HOP_ATTENUATION (0.7)`. If `child_confidence <
   MIN_CONFIDENCE_THRESHOLD (0.05)` → prune (`threshold_prunes`), stop. **All children of
   a node share this single attenuated confidence** (it is not per-edge scored beyond
   attenuation).
4. **Candidate generation** (`outgoing_candidates`): scan `ColumnFamily::Base`; for each
   domain-matching constellation, `read_series`, parse each occurrence as
   `ExpansionContext`, filter by action match, and collect `ChildCandidate`s. Dedup by
   `ChildKey { domain, action_or_event, outcome_label }` (first occurrence wins, ordered by
   `BTreeMap`).
5. For each candidate: skip if its `NodeKey` is in `visited` (`cycle_skips`). Build a child
   `Consequence` with `confidence = child_confidence`, `hop = parent.hop + 1`, and
   provenance:
   - **grounded** candidate (`grounded && !provisional` in the edge evidence) → recurse
     (insert/expand/remove key for cycle safety), provenance = pending ref.
   - **non-grounded** candidate → emitted as a leaf, **not expanded** (`provisional_edges`),
     provenance = `provisional_ledger_ref()` sentinel.
6. After the whole tree is built, `write_expansion_ledger` (`oracle_expand_v1`) records all
   stats, and `apply_grounded_provenance` rewrites every non-provisional child's provenance
   to the real ledger ref. Sentinel (provisional) refs are left as-is.

So **branching = the set of distinct grounded/observed consequence edges per node**
(no fixed fan-out cap; it is data-driven, deduped). **Depth ≤ 4**; **min confidence 0.05**;
**per-hop attenuation ×0.7**; cycles blocked by domain+action key.

### 5.4 `select` scoring

`select_terminal` recurses to leaves and scores each leaf's outcome against the desired
`AnchorValue` via `anchor_score`, keeping the max:

| Anchor pair | Score |
|---|---|
| `Bool`/`Enum`/`Text` equal | `1.0` (else `None`) |
| `Number` (both finite) | `1.0 / (1.0 + |a-b|)` |
| `OneHot` | Jaccard (`|∩| / |∪|`), `None` if 0 |
| `Vector` (same non-empty len, finite) | cosine similarity |
| mismatched variants | `None` |

### 5.5 `ExpansionContext` (`butterfly/context.rs`)

Same shape as `PredictionContext` plus a `grounded` (default true) and `provisional`
(default false) flag per `EdgeEvidence`; `grounded = grounded && !provisional` controls
expansion.

---

## 6. Self-consistency (`self_consistency.rs`)

`oracle_self_consistency(vault, domain, clock) -> Result<OracleSelfConsistency, OracleError>`.
This is the ceiling source for forward prediction (§4) and tier-1 super-intelligence (§8).

Steps:
1. `domain_series` — scan Base, keep domain-matching constellations, `read_series` each.
   **No domain constellations → `DomainNotFound`.**
2. `consistency_stats` over all series:
   - Parse each occurrence as `RecurrenceEvidence` (`oracle_verdict`/`outcome_anchor` →
     verdict label, optional `ground_truth_anchor` → ground-truth label).
   - **Flakiness:** for each series count verdict-label multiplicities; `total_pairs =
     Σ C(n,2)`, `agreement_pairs = Σ_label C(count,2)`. If `total_pairs <
     MIN_FLAKINESS_PAIRS (10)` → `NoRecurrence`. `flakiness = 1 - agreement/total`
     (clamped).
   - **Validity** (`validity`): from `(verdict, ground_truth)` pairs. No samples →
     `(0.0, provisional=true)`. `< MIN_VALIDITY_SAMPLES` → `NoRecurrence`. All match →
     `1.0`. If ground-truth entropy ≈ 0 → fraction matching. Else KSG MI of one-hot
     verdict vs truth codes (`ksg_mi_continuous_discrete`, k=3), normalized
     `bits/entropy`, clamped → validity.
3. Build `OracleSelfConsistency::with_provenance(flakiness, validity, provisional, None)`
   (so `ceiling = validity*(1-flakiness)`), write `oracle_self_consistency_v1` ledger
   (`EntryKind::Assay`), attach provenance.

---

## 7. Backward / abductive walk (`reverse_query.rs`)

`reverse_query(vault, answer: &AnchorValue, domain, clock) -> Result<Vec<Cause>, OracleError>`
— "what action/event causes this outcome?" (epistemic symmetry, plan §5).

`Cause` (`super_intel_types.rs`) = `{ action_or_event: String, domain: DomainId,
confidence: f32, provisional: bool, provenance: LedgerRef }`.

### 7.1 Algorithm (`walk_answer`, depth-bounded DFS, `MAX_REVERSE_DEPTH = 3`)

`WalkState` tracks `visited_answers` (seeded with the answer label), `visited_actions`
(seeded with the answer's text/enum value), accumulated `causes` keyed by
`(domain, action)`, stats, and a `found` flag.

For each depth (prune when `depth > MAX_REVERSE_DEPTH`):
1. Scan Base, keep domain-matching constellations.
2. **Structural causes** (`collect_structural_cause`): a constellation matches if any of its
   anchors equals `answer`, or its `oracle.effect` metadata decodes/equals `answer`. If so
   and it has an action, add a **provisional** cause with confidence =
   `oracle.structural_confidence` metadata (parsed, clamped) or `STRUCTURAL_CONFIDENCE
   (0.35)`. Sets `found`.
3. **Recurrence causes** (`collect_recurrence_causes`): `read_series`, parse each occurrence
   as `ReverseContext`, iterate its edges. An edge matches when `edge.outcome.value ==
   answer && edge.domain == domain`. The action is the context action (or base
   constellation action). Skip actions already in `visited_actions` (`cycle_skips`).
   Otherwise mark `found`, add a cause candidate: `provisional = !edge.is_grounded()`
   (`grounded && !provisional`), confidence = `grounded_confidence(1) = 1/2`.
4. **Antecedent walk** (`maybe_walk_antecedent`): only if the just-inserted cause was
   **grounded** and `depth < MAX_REVERSE_DEPTH`. Treat the cause's action as the next
   `answer` (`AnchorValue::Text(action)`), guard against revisiting via `visited_answers`
   /`visited_actions`, recurse at `depth+1`, then unwind both visited sets. This is the
   backward chaining: outcome → cause → cause-of-cause, up to 3 hops.

After the walk: **if nothing matched (`!found`) → `DomainNotFound`.** Else finalize each
`CauseAccumulator` into a `Cause`:
- if any grounded observation: `confidence = grounded_confidence(grounded_count) =
  count/(count+1)`, `provisional = false`.
- else: `confidence = max provisional confidence seen`, `provisional = true`.

Sort causes by `provisional` (grounded first), then confidence desc, then action, then
domain. Write `reverse_query_v1` ledger and stamp its ref onto every cause.

`grounded_confidence(n) = n/(n+1)`: 1 obs → 0.5, 2 → 0.667, etc. (monotone, never reaches 1).

### 7.2 `ReverseContext` (`reverse_query_context.rs`)

`EdgeEvidence` here ignores `action_or_event` (renamed `_action_or_event`) — the action for
a reverse cause comes from the context/constellation action, not the edge. `domain` default
"oracle", `grounded` default true, `provisional` default false; `is_grounded = grounded &&
!provisional`.

---

## 8. Super-intelligence predicate (six tiers)

The plan's `super_intelligence(domain)` predicate. Types in `super_intel_types.rs`;
tiers 1–3 measured in `super_intel.rs`; full six tiers in `super_intel_full.rs`.

### 8.1 Tier types (`super_intel_types.rs`)

`Tier` enum (serde snake_case), fixed `Tier::ORDER` of six:
`OracleClean, PanelSufficient, KernelExists, Calibrated, GoodhartDefended, MistakeClosed`.

`TierResult { tier, passed, measured_value, threshold, cheapest_fix: Option<String> }`.

`SuperIntelReport { domain, tiers, failing_tier: Option<Tier>, cheapest_fix, overall }`:
- `overall = tiers.iter().all(passed)` (vacuously true for empty).
- `failing_tier` = first tier in predicate order that exists in `tiers` and failed.
- `cheapest_fix` = that failing tier's `cheapest_fix`.
- helpers: `failing_tier_report`, `passed_count`, `failed_count`, `Display`.

### 8.2 Tier thresholds and pass rules

| Tier | Measured value | Threshold | Pass rule | Cheapest-fix strings |
|---|---|---|---|---|
| OracleClean | `self_consistency.ceiling` | `0.7` | value `>= 0.7` (finite, ≥0) | "add validity-tracking anchor" (if provisional or validity<0.7) else "label more oracle instances to reduce flakiness" |
| PanelSufficient | `report.panel_bits` | `report.anchor_entropy_bits` | `panel_bits >= anchor_entropy_bits` | "add outcome/execution-derived lens for {lens} (deficit {bits})" (max-deficit lens) |
| KernelExists | `report.ratio` | `0.95` | `ratio >= 0.95` and `n_queries_tested > 0` | "label held-out instances" / "ingest more anchor instances for domain" / recall error |
| Calibrated | `measurement.calibration_error` | `ceiling + 0.0` | `error <= threshold` | "run conformal calibration with more held-out instances" / "label held-out oracle instances" |
| GoodhartDefended | `measurement.pass_rate` | `0.9` | `report_passed && pass_rate >= 0.9` | "strengthen Gtau guard or add cross-lens anomaly detector" |
| MistakeClosed | `recurring_mistakes` (count) | `0.0` | `recurring_mistakes == 0` | "trigger online head update for the recurring mistake pattern" |

Note: `measured_tier`/`failed_tier`/`valid_measurement` sanitize non-finite values to 0.0.
`Calibrated` threshold is `oracle_self_consistency_ceiling + CALIBRATION_CEILING_DELTA`
where the delta is `0.0` — so the predictor must calibrate to at most the self-consistency
ceiling (a "lower is better" calibration error compared against the ceiling).

### 8.3 Sources and orchestration

Trait-based, so callers inject measured engines:
- `OracleConsistencySource` (impl'd for `AsterVault`), `SufficiencyAssay`,
  `KernelRecallSource` (`KernelRecallGate` wraps Lodestar; forces `min_recall_ratio =
  0.95`).
- `CalibrationSource` (impl for `calyx_ward::GuardProfile`), `GoodhartDefenseSource`
  (impl for `calyx_anneal::GoodhartReport`), `MistakeClosureSource` (impl for
  `RegressionReport`).

`HeldOutSplit { split_id, training_ids, held_out_ids }` — `has_training_leakage` rejects
overlap; empty held-out fails KernelExists/Calibrated/GoodhartDefended with a label fix.

`ShortCircuit { Enabled, MeasureAll (default) }` — `Enabled` stops at the first failing
tier; `MeasureAll` measures all.

| Function | Tiers | Notes |
|---|---|---|
| `measure_tier_oracle_clean[_with_source]` | 1 | |
| `measure_tier_panel_sufficient[_with_assay]` | 2 | |
| `measure_tier_kernel_exists` | 3 | validates held-out |
| `measure_tiers_1_to_3` / `measure_super_intelligence_tiers_1_to_3` | 1–3 | honors short-circuit |
| `measure_tier_calibrated` / `_goodhart_defended` / `_mistake_closed` | 4 / 5 / 6 | |
| `measure_super_intelligence_tiers` | 1–6 | builds full `SuperIntelReport` |
| `super_intelligence` / `super_intelligence_with_ledger` | 1–6 | latter writes `super_intelligence_v1` ledger |
| `write_super_intelligence_ledger` | — | standalone ledger write |

### 8.4 PRD-22 formula primitives (`prd22.rs`)

Pure functions over scalars / `AssocGraph` (no vault). Return `calyx_core::Result`; errors
use `CalyxError::oracle_insufficient` (`CALYX_ORACLE_INSUFFICIENT`).

| Function | Behavior |
|---|---|
| `oracle_ceiling(tau_corr, flakiness, validity)` | `OracleCeiling`; `oracle_self_consistency = validity*(1-flakiness)`, `capped_tau = min(tau_corr, that)` |
| `oracle_predict(panel_bits, anchor_entropy_bits, requested_confidence)` | `OraclePrediction`; if `deficit_bits = max(0, entropy-panel) > EPSILON` → insufficient error |
| `butterfly_expand(graph, source, max_hops)` | `Vec<ConsequenceExpansion>` via `reach_scored`, sorted by score desc then cx id |
| `reverse_query(graph, answer, max_hops)` | reverses all edges, then `butterfly_expand` |
| `super_intelligence(SuperIntelligenceEvidence)` | `SuperIntelligenceVerdict { pass, failing_tiers }`; pushes tier names for each unmet condition |

Public formula structs: `OracleCeiling`, `OraclePrediction`, `ConsequenceExpansion`,
`SuperIntelligenceEvidence`, `SuperIntelligenceVerdict`. Re-exported as
`oracle_formula_predict`, `reverse_query_formula`, `super_intelligence_formula`.

---

## 9. Completion + energy descent

### 9.1 Energy (`energy.rs`)

Hopfield-style softmax energy over trusted-region attractors.

| Item | Signature / value |
|---|---|
| `DescentResult` | `{ steps_taken: usize, converged: bool, final_energy: f32 }` |
| `AnnealConfig` (trait) | `energy_beta(&self, &DomainId) -> Option<f32>` |
| `energy(x, region_members, beta)` | `-log_sum_exp(beta * cosine(x, member))`; `beta==0` → `-ln(n)` |
| `energy_softmax_weights(...)` | stable softmax of scaled cosines (`beta==0` → uniform) |
| `descent_step(free_slot, members, beta)` | weighted mean of members, then `normalize_f32` |
| `descend(free_slot, members, beta, max_steps, eps)` | iterate ≤ `max_steps`; converge when single member or `|ΔE| < eps` |
| `get_beta(domain, anneal)` | `anneal.energy_beta`, filtered finite/≥0, else `DEFAULT_BETA (1.0)` |

Validation errors use codes `CALYX_ORACLE_ENERGY_EMPTY_REGION` and
`CALYX_ORACLE_ENERGY_INVALID_INPUT` (empty region, empty/NaN slot, dim mismatch, bad
beta/eps). Constants: `MAX_STEPS=20`, `DEFAULT_EPS=1e-4`, `DEFAULT_BETA=1.0`.

### 9.2 Completion (`complete.rs`)

`complete(...)` and `complete_with_assay_and_region(...)` fill the **free** lenses of a
partial constellation given clamped measured lenses, gated by the honesty gate.

Traits: `CompletionRegion::members_for_lens` (attractors; `WardCompletionRegion` adapts
`TrustedRegion`s), `CompletionLedger::append_completion`
(`AsterCompletionLedger` writes `oracle_completion_v1`).

Steps: `validate_request` (panel-version match; clamp/free must be disjoint, exhaustive,
and present — else `SlotConflict`); `check_sufficiency_with_assay` (gate); extract dense
slot vectors (`SlotConflict`/`lens_dim_mismatch` on sparse/multi); clamp → `Measured`
slots; for each free lens, `descend` toward region members at `get_beta`; tag `Inferred`
iff `descent.converged && sufficiency.sufficient` else `Provisional`.
`CompletionDraft::from_descents` computes confidence `1 - mean_energy/mean_ln(members)`
(clamped, capped by `self_consistency.ceiling`), `converged` = all converged.
`CompletionResult::new` re-validates the slot partition/tags (`SlotConflict` on
overlap/missing/extra/tag-mismatch).

---

## 10. Time-of-next-occurrence prediction (`time_prediction.rs`)

Returns `calyx_core::Result<TimePrediction>` (errors are `CalyxError::oracle_insufficient`,
**not** `OracleError`).

`TimePrediction` fields: `cx_id`, `sufficient`, `support`, `active_support`,
`rolled_support`, `rollup_period_estimate_secs: Option<f64>`, `tz_offset_secs`,
`t_hat: EpochSecs`, `confidence`, `confidence_ceiling`, `cadence_secs`, `cadence_mad_secs`,
`interval: TimePredictionInterval { low, high }`, `periodic_confidence`.
`TimeBucket { hour, day_of_week, tz_offset_secs }` (Monday=0; epoch offset constant 3).

Entry points: `predict_next_occurrence[_with_tz_offset]` (reads series from vault) and
`predict_next_occurrence_from_series[_with_tz_offset]`.

Steps: validate `confidence_ceiling` finite in `[0,1]`; sort occurrence times; require
`>= MIN_TIME_PREDICTION_OCCURRENCES (3)` active times (else insufficient — distinguishes a
rolled-up series from a sparse one in the message); positive strictly-increasing gaps;
`cadence_secs = median(gaps)` (must be finite > 0); `cadence_mad_secs = MAD(gaps,
cadence)`; `t_hat = last_time + round(cadence)` (checked add). `periodic_confidence` = max
of hour×day, hour-of-day, day-of-week mode fractions.
`confidence = regularity * support_confidence * periodic_confidence`, capped by the
ceiling, where `regularity = clamp01(1 - mad/cadence)` and `support_confidence =
min(1, support/12)`. Interval half-width = `max(mad, cadence*(1-confidence))`.

---

## 11. Provenance / ledger tags

Every vault-backed operation writes an append-only ledger entry (`calyx-ledger`), actor
`calyx-oracle`:

| Tag | Writer | EntryKind |
|---|---|---|
| `oracle_predict_v1` | `predict.rs` | `Answer` |
| `oracle_expand_v1` | `butterfly.rs` | `Answer` |
| `reverse_query_v1` | `reverse_query.rs` | `Answer` |
| `oracle_self_consistency_v1` | `self_consistency.rs` | `Assay` |
| `oracle_completion_v1` | `complete.rs` | `Answer` |
| `super_intelligence_v1` | `super_intel_full.rs` | `Assay` |

Subjects are `content_address(...)` digests of domain/action/outcome/answer.

---

## 12. Divergences from the plan

- The honesty gate uses an **exact `panel_bits >= anchor_entropy_bits`** comparison; the
  plan writes `I(panel; oracle) ≥ τ_MI` with a τ_MI threshold. In code there is no separate
  τ_MI; the threshold *is* the outcome entropy `H(outcome)` (the assay's
  `anchor_entropy_bits`). `21 §2` actually states the gate as `I(panel;oracle) <
  H(outcome)` → insufficient, which matches the code.
- The plan describes `oracle_predict` returning `consequences` "each itself expandable" —
  in code first-order consequences come from `predict.rs`; deeper expansion is the separate
  `butterfly::build_tree`/`expand` path (depth ≤ 4).
- Plan's `super_intelligence` is a six-tier conjunction; code splits it into a 1–3 path
  (`super_intel.rs`) and the full six-tier path (`super_intel_full.rs`), plus a pure scalar
  formula in `prd22.rs`. All three coexist.

## Gaps / not covered

- `OracleError::FlakyAnchor` is in the public catalog but **never constructed** in this
  crate's `src/`; the consistency path returns `NoRecurrence` instead. Not a stub, but a
  dead constructor at the time of reading.
- No `oracle_predict` `confidence`-based refusal beyond the gate: a sufficient panel with
  low recurrence still returns a (possibly very low) confidence rather than an error.
- `27_INTELLIGENCE_OBJECTIVE.md` describes an `intelligence_report`/`next_best_action`/
  `growth_curve` objective `J` loop. **No code for `J` or those APIs exists in
  calyx-oracle**; this crate contributes the oracle-accuracy / mistake-closure tiers only.
  The objective loop is not implemented here.
- The PRD-22 `prd22.rs` butterfly/reverse operate on a generic `AssocGraph` and are
  separate from the vault-backed `butterfly.rs`/`reverse_query.rs`; they are formula
  primitives, not the production path.
- Time prediction and the PRD-22 formulas return `CalyxError`, not `OracleError`; callers
  must handle both error types.
