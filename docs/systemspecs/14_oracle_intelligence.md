# 14. Oracle Intelligence (calyx-oracle)

Stage 11 of the Calyx pipeline. The `calyx-oracle` crate implements consequence
prediction, vault-backed completion of partial constellations, reverse (cause)
query traversal, time-of-next-occurrence prediction, an Assay-backed honesty
gate, an energy-descent substrate, oracle self-consistency measurement, and a
six-tier "super-intelligence" predicate. All vault-facing primitives append a
provenance entry to the append-only ledger (cross-ref doc 11, Ledger /
Provenance).

This document describes only what the source contains. Items not determinable
from source are marked "Not determined from source".

## Source files covered

- `src/lib.rs` — crate root, module list, public re-exports.
- `src/types.rs` (+ `src/types_tests.rs`) — public contract types.
- `src/error.rs` — `OracleError` catalog.
- `src/predict.rs` (+ `src/predict/context.rs`, `src/predict_tests.rs`) — `oracle_predict`.
- `src/butterfly.rs` (+ `src/butterfly/context.rs`, `src/butterfly_tests.rs`) — consequence tree expansion / selection.
- `src/reverse_query.rs` (+ `src/reverse_query_context.rs`, `src/reverse_query_tests.rs`) — vault reverse traversal.
- `src/complete.rs` (+ `src/complete_tests.rs`, `src/complete_test_support.rs`) — completion primitive.
- `src/energy.rs` (+ `src/energy_tests.rs`) — energy / descent substrate.
- `src/honesty_gate.rs` (+ `src/honesty_gate_tests.rs`) — sufficiency check.
- `src/self_consistency.rs` (+ `src/self_consistency_tests.rs`) — flakiness/validity measurement.
- `src/super_intel.rs` (+ `src/super_intel_tests.rs`) — tiers 1–3.
- `src/super_intel_full.rs` (+ `src/super_intel_full_tests.rs`) — full six-tier predicate.
- `src/super_intel_types.rs` — `Tier`, `TierResult`, `SuperIntelReport`, `Cause`.
- `src/time_prediction.rs` (+ `src/time_prediction_tests.rs`) — next-occurrence prediction.
- `src/prd22.rs` — graph-based formula primitives (PRD-22).
- `tests/` — integration / FSV harnesses (`predict_fsv.rs`, `super_intel_fsv.rs`, `complete_tests.rs`, `rolled_recurrence_fsv.rs`, `ph50_exit_fsv.rs`, support modules).

Crate dependencies (`Cargo.toml`): `calyx-assay`, `calyx-anneal`, `calyx-aster`,
`calyx-core`, `calyx-forge`, `calyx-ledger`, `calyx-lodestar`, `calyx-paths`,
`calyx-ward`, `serde`, `serde_json`.

---

## 14.1 Module / feature map

| Module | Public surface | Vault-backed | Writes ledger | Purpose |
|--------|----------------|--------------|---------------|---------|
| `types` | contract structs/enums | no | no | Shared Oracle data contracts |
| `error` | `OracleError`, 6 code constants | no | no | Structured error catalog |
| `predict` | `oracle_predict`, `Action` | yes | yes (`oracle_predict_v1`) | Forward consequence prediction |
| `butterfly` | `expand`, `build_tree`, `select` | yes | yes (`oracle_expand_v1`) | Hop-attenuated consequence tree |
| `reverse_query` | `reverse_query` | yes | yes (`reverse_query_v1`) | Cause traversal (epistemic symmetry) |
| `complete` | `complete`, `complete_with_assay_and_region`, traits | yes | yes (`oracle_completion_v1`) | Fill free slots via energy descent |
| `energy` | `energy`, `descend`, `descent_step`, `energy_softmax_weights`, `get_beta` | no | no | Energy / softmax descent math |
| `honesty_gate` | `check_sufficiency`, `check_sufficiency_with_assay`, `SufficiencyAssay`, `VaultSufficiencyAssay` | yes | no | Refuse-on-insufficiency gate |
| `self_consistency` | `oracle_self_consistency` | yes | yes (`oracle_self_consistency_v1`) | Flakiness/validity → ceiling |
| `super_intel` | tiers 1–3 measurers, `KernelRecallGate`, `HeldOutSplit` | mixed | no | First three super-intel tiers |
| `super_intel_full` | tiers 4–6 + `super_intelligence` | yes | yes (`super_intelligence_v1`) | Full six-tier predicate |
| `super_intel_types` | `Tier`, `TierResult`, `SuperIntelReport`, `Cause` | no | no | Super-intel result contracts |
| `time_prediction` | `predict_next_occurrence*`, `time_bucket` | yes | no | Next-occurrence timestamp |
| `prd22` | `oracle_ceiling`, `oracle_predict`, `butterfly_expand`, `reverse_query`, `super_intelligence` | no (graph) | no | PRD-22 formula primitives |

Common metadata keys read from constellations (`src/predict.rs`, `src/reverse_query.rs`, `src/self_consistency.rs`, `src/butterfly.rs`):

| Constant | Value | Defined in |
|----------|-------|------------|
| `ORACLE_ACTION_METADATA_KEY` | `"oracle.action"` | `predict.rs` |
| (fallback action) | `"action"` | `predict.rs`, `reverse_query.rs`, `butterfly.rs` |
| `ORACLE_DOMAIN_METADATA_KEY` | `"oracle.domain"` | `self_consistency.rs` |
| `ORACLE_FALLBACK_DOMAIN_METADATA_KEY` | `"domain"` | `self_consistency.rs` |
| `ORACLE_EFFECT_METADATA_KEY` | `"oracle.effect"` | `reverse_query.rs` |
| `ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY` | `"oracle.structural_confidence"` | `reverse_query.rs` |

All ledger entries are written via `vault.append_ledger_entry(...)` with
`ActorId::Service("calyx-oracle")`. Predict/expand/reverse/complete use
`EntryKind::Answer`; self-consistency and super-intelligence use
`EntryKind::Assay`.

---

## 14.2 Contract types (`src/types.rs`, `src/super_intel_types.rs`)

### 14.2.1 Core types (`types.rs`)

| Type | Kind | Key fields |
|------|------|-----------|
| `DomainId` | newtype `String` (transparent serde) | `new`, `as_str`, `Display`, `From<&str>`, `From<String>` |
| `Prediction` | struct | `outcome: AnchorValue`, `confidence: f32`, `consequences: Vec<Consequence>`, `bound: SufficiencyBound`, `provenance: LedgerRef`, `guard: GuardVerdict` |
| `SufficiencyBound` | struct | `i_panel_oracle: f32` (serde `I_panel_oracle`), `dpi_ceiling: f32`, `sufficient: bool`, `per_sensor_deficit: Vec<(LensId, f32)>` |
| `OracleSelfConsistency` | struct | `flakiness: f32`, `validity: f32`, `ceiling: f32`, `provisional: bool`, `provenance: Option<LedgerRef>` |
| `Consequence` | struct | `action_or_event: String`, `domain: DomainId`, `outcome: AnchorValue`, `confidence: f32`, `hop: u8`, `provenance: LedgerRef` |
| `ConsequenceTree` | struct | `root: Consequence`, `children: Vec<ConsequenceTree>`, `max_depth: u8` |
| `CompletionResult` | struct | `filled_cx: Vec<TaggedSlot>`, `confidence: f32`, `converged: bool`, `energy: f32`, `provenance: LedgerRef` |
| `TaggedSlot` | struct | `lens_id: LensId`, `vector: Vec<f32>`, `tag: SlotTag` |
| `SlotTag` | enum | `Measured`, `Inferred`, `Provisional` (snake_case serde) |
| `SlotSet` | type alias | `HashSet<LensId>` |
| `CompletionSlotPartition<'a>` | struct | `all_slots`, `clamp`, `free` (borrows of `SlotSet`) |

Constant: `DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH: u8 = 4`.

`OracleSelfConsistency::with_provenance` computes `ceiling = validity * (1.0 - flakiness)` (line 91); `measured(...)` and `provisional(...)` are convenience constructors.

`ConsequenceTree::leaf(root)` builds a childless tree with `max_depth = 4`.

`CompletionResult::new` validates the slot partition via `validate_completion_slots` before constructing. It exposes `inferred_slots()`, `provisional_slots()`, `measured_slots()` filtering by tag.

**Slot partition validation (`validate_completion_slots`, lines 207–239).** Given
`all_slots`, `clamp`, `free` and the filled slots, it computes:
- `overlap` = `clamp ∩ free` (must be empty — clamp/free disjoint).
- `missing` = `all_slots \ (clamp ∪ free)` plus `all_slots \ filled` (every slot must be covered and filled).
- `extra` = `(clamp ∪ free) \ all_slots` plus `filled \ all_slots`.
- `tag_mismatch` = a clamped slot tagged non-`Measured`, or a free slot tagged `Measured`.
If any list is non-empty it returns `OracleError::SlotConflict { overlap, missing, extra, tag_mismatch }`.

### 14.2.2 Super-intel contract types (`super_intel_types.rs`)

| Type | Kind | Notes |
|------|------|-------|
| `Tier` | enum (snake_case) | `OracleClean`, `PanelSufficient`, `KernelExists`, `Calibrated`, `GoodhartDefended`, `MistakeClosed`; `Tier::ORDER` is the 6-element predicate order; `as_str()` / `Display` |
| `TierResult` | struct | `tier: Tier`, `passed: bool`, `measured_value: f32`, `threshold: f32`, `cheapest_fix: Option<String>` |
| `SuperIntelReport` | struct | `domain: DomainId`, `tiers: Vec<TierResult>`, `failing_tier: Option<Tier>`, `cheapest_fix: Option<String>`, `overall: bool` |
| `Cause` | struct | `action_or_event: String`, `domain: DomainId`, `confidence: f32`, `provisional: bool`, `provenance: LedgerRef` |

`SuperIntelReport::new` (lines 90–107) sets `overall = tiers.iter().all(passed)`;
`failing_tier` = the first tier in `Tier::ORDER` that has a failing entry
(`first_failing_tier`, lines 150–156 — predicate order, not insertion order);
`cheapest_fix` is taken from that failing tier's entry. An empty tier list is
vacuously `overall = true`. Helpers: `failing_tier_report()`, `passed_count()`,
`failed_count()`.

---

## 14.3 Error catalog (`src/error.rs`)

`OracleError` variants, codes, and remediation:

| Variant | `code()` | `remediation()` |
|---------|----------|-----------------|
| `Insufficient { bound }` | `CALYX_ORACLE_INSUFFICIENT` | "add outcome/execution lenses before prediction" |
| `FlakyAnchor { self_consistency }` | `CALYX_ORACLE_FLAKY_ANCHOR` | "re-measure the grounded oracle anchor and quarantine flaky outcomes" |
| `NoRecurrence { domain }` | `CALYX_ORACLE_NO_RECURRENCE` | "collect grounded recurrence pairs for the domain" |
| `DomainNotFound` | `CALYX_ORACLE_DOMAIN_NOT_FOUND` | "register the oracle domain before prediction" |
| `LedgerWriteFailure` | `CALYX_ORACLE_LEDGER_WRITE_FAILURE` | "retry after repairing the ledger write path" |
| `SlotConflict { overlap, missing, extra, tag_mismatch }` | `CALYX_ORACLE_SLOT_CONFLICT` | "make clamp/free disjoint and exhaustive; tag clamped slots measured and free slots inferred or provisional" |
| `AssayFailure { source: CalyxError }` | `source.code` | `source.remediation` |

`Display` renders `"{code}: {message}; remediation: {remediation}"`. `From<CalyxError>`
wraps into `AssayFailure`; `From<OracleError> for CalyxError` unwraps `AssayFailure`
or rebuilds a `CalyxError` from the code/message/remediation.

(`FlakyAnchor` is part of the catalog but is not constructed by any module read
in this crate — verify against callers.)

---

## 14.4 Forward prediction — `oracle_predict` (`src/predict.rs`)

`oracle_predict<C>(vault, action: &Action, domain, clock) -> Result<Prediction, OracleError>`.

`Action` (public): `action_id: String`, `panel: Panel`, `guard: Option<GuardVerdict>`.

Constants: `ORACLE_ACTION_METADATA_KEY = "oracle.action"`, fallback `"action"`,
`HOP_ATTENUATION = 0.7`, `PROVISIONAL_GUARD_ID = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c104"`,
`LEDGER_TAG = "oracle_predict_v1"`.

### Algorithm (steps)

1. **Gate.** `check_sufficiency(vault, panel, domain, clock)` → `SufficiencyBound`; returns `OracleError::Insufficient` if not sufficient (section 14.8).
2. **Gather evidence** (`prediction_evidence`): scan `ColumnFamily::Base`, decode each constellation, keep rows whose `oracle.domain`/`domain` metadata equals `domain`. For each, read its recurrence series (`read_series`) and collect `OutcomeObservation`s from non-empty occurrence contexts whose action matches (`collect_series`). Empty observation set → `NoRecurrence`.
3. **Posterior** (`posterior`): bucket observations by JSON `outcome_label`, count each, rank by count desc then label asc. Top bucket is the predicted outcome.
4. **Raw confidence** (`raw_confidence`, lines 227–236):

   ```
   support     = top_count / total
   separation  = (top_count - second_count) / total      // saturating sub
   sample_supp = total / (total + 2.0)
   raw         = (support * separation * sample_supp).clamp(0,1)
   ```

5. **Self-consistency** (`oracle_self_consistency`, section 14.9) yields `ceiling`.
6. **Apply ceiling** (`apply_confidence_ceiling`, line 238):
   `confidence = unit(raw).min(unit(self_consistency)).min(unit(dpi_ceiling))`.
7. **Guard.** Use the caller-supplied `GuardVerdict`, else `provisional_guard(panel)` (a permissive `provisional = true` verdict with `cos = 1.0`, `tau = DEFAULT_TAU`, all slots pass).
8. **Ledger write** (`oracle_predict_v1`): records domain digest, action id/digest, outcome digest, source cx ids, recurrence stats (`top_count`, `second_count`, `distinct_outcomes`, `total`), `raw_confidence`, `self_consistency_ceiling`, `dpi_ceiling`, final `confidence`, `ts`.
9. **First-order consequences** (`first_order_consequences`, lines 250–294): over observations whose outcome equals the predicted label, bucket consequences by `(action_or_event, domain, outcome_label)`, count, and emit one `Consequence` per bucket with `hop = 1` and

   ```
   conf = (confidence * HOP_ATTENUATION * bucket.count / predicted_count).clamp(0, confidence)
   ```

`PredictionContext` (`predict/context.rs`) parses occurrence JSON: action via `action_id` else `action`; outcome via `outcome_anchor` else `oracle_verdict`; consequences from the singular `consequence` field chained with the plural `consequences` array (blank `action_or_event` filtered out; default domain `"oracle"`).

---

## 14.5 Butterfly consequence tree — `build_tree`, `expand`, `select` (`src/butterfly.rs`)

Constants: `MAX_DEPTH = DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH = 4`,
`HOP_ATTENUATION = 0.7`, `MIN_CONFIDENCE_THRESHOLD = 0.05`,
`PROVISIONAL_SEQ = u64::MAX`, `LEDGER_TAG = "oracle_expand_v1"`.

Public API:
- `build_tree(vault, root, clock) -> ConsequenceTree`.
- `expand(vault, consequence, clock) -> Vec<Consequence>` — builds the tree and `flatten_descendants` (all non-root nodes, pre-order).
- `select(tree, desired_outcome) -> Option<&ConsequenceTree>` — best-scoring terminal (leaf) node.
- `provisional_ledger_ref()` / `is_provisional_ledger_ref(&LedgerRef)` — sentinel `LedgerRef { seq: u64::MAX, hash: [0;32] }`.

### Expansion algorithm (`expand_node`, lines 96–151)

1. If `node.root.hop >= MAX_DEPTH` → prune (`depth_prunes`).
2. `child_confidence = attenuate(node.confidence) = (unit(conf) * 0.7).clamp(0,1)`. If `< MIN_CONFIDENCE_THRESHOLD (0.05)` → prune (`threshold_prunes`).
3. `outgoing_candidates`: scan Base CF for domain-matching constellations; for each, read its series and `collect_candidates`. Candidates are deduped into a `BTreeMap<ChildKey, ChildCandidate>` keyed by `(domain, action_or_event, outcome_label)`.
4. For each candidate not already in the `visited` set (cycle guard keyed by `(domain, action_or_event)`):
   - Build a child `Consequence` with `confidence = child_confidence`, `hop = parent.hop + 1`.
   - Provenance = `pending_ledger_ref()` (`seq=0`) if grounded, else `provisional_ledger_ref()` (`seq=u64::MAX`).
   - If grounded: insert key into `visited`, recurse, then remove (DFS backtracking). If not grounded: count `provisional_edges`, do not recurse.
5. After building, write one `oracle_expand_v1` ledger entry (root digests + full `ExpansionStats`), then `apply_grounded_provenance` rewrites every non-provisional child provenance to that ledger ref (provisional refs left as the sentinel).

`ExpansionContext` (`butterfly/context.rs`): a child is `grounded = edge.grounded && !edge.provisional` (default `grounded = true`, `provisional = false`).

### Terminal selection scoring (`anchor_score`, lines 312–326)

`select` walks to leaves and keeps the highest score per `AnchorValue` type pair:

| Actual / desired pair | Score |
|------------------------|-------|
| `Bool == Bool` / `Enum == Enum` / `Text == Text` | `1.0` if equal, else `None` |
| `Number` vs `Number` (both finite) | `1.0 / (1.0 + |left - right|)` |
| `OneHot` vs `OneHot` | Jaccard: `|∩| / |∪|`, `None` if `∪` empty or score 0 |
| `Vector` vs `Vector` | cosine `dot / (‖l‖·‖r‖)`; `None` on length mismatch, empty, non-finite, or zero norm |
| any other pairing | `None` |

---

## 14.6 Reverse query — vault traversal (`src/reverse_query.rs`)

`reverse_query<C>(vault, answer: &AnchorValue, domain, clock) -> Result<Vec<Cause>, OracleError>`.
Implements "epistemic symmetry": given an answer/outcome, find the actions/events that cause it.

Constants: `MAX_REVERSE_DEPTH = 3`, `ORACLE_EFFECT_METADATA_KEY = "oracle.effect"`,
`ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY = "oracle.structural_confidence"`,
`STRUCTURAL_CONFIDENCE = 0.35` (default), `LEDGER_TAG = "reverse_query_v1"`.

### Algorithm (steps)

1. **Init** `WalkState`: visited answer labels seeded with the JSON of the target answer; visited actions seeded with the answer's text/enum value; empty cause map (`BTreeMap<CauseKey, CauseAccumulator>`); `found = false`.
2. **`walk_answer`** (recursive, depth-bounded by `MAX_REVERSE_DEPTH = 3`): scan Base CF, decode, keep domain-matching constellations. For each:
   - **Structural match** (`collect_structural_cause`): if any anchor value equals the answer, or the `oracle.effect` metadata decodes/matches the answer, and the constellation has an action (`oracle.action` else `action`), record a **provisional** cause. Its confidence is read from `oracle.structural_confidence` metadata (clamped to [0,1]) or `0.35` default.
   - **Recurrence match** (`collect_recurrence_causes`): read the series; for each non-empty occurrence context (`ReverseContext`), for each edge whose `outcome.value == answer && domain == domain`, take the action (`action_id` else `action`, else the constellation's base action). If the action is in `visited_actions` → cycle skip. Otherwise mark `found`, record a cause: `provisional = !edge.is_grounded()` (`is_grounded = grounded && !provisional`), grounded confidence `count/(count+1)` for count 1 = `0.5`.
   - **Antecedent recursion** (`maybe_walk_antecedent`): only when the edge is grounded and `depth < MAX_REVERSE_DEPTH`. Pushes the action as a new answer (`AnchorValue::Text`), guards against revisits via `visited_answers`/`visited_actions`, recurses at `depth + 1`, then backtracks (removes the inserted labels).
3. **No match** anywhere → `OracleError::DomainNotFound`.
4. **Accumulate** (`upsert_cause` / `CauseAccumulator`): per `(domain, action_or_event)`, grounded observations increment a count; provisional observations keep the max provisional confidence. `into_cause`: if any grounded, `provisional = false`, `confidence = grounded_count/(grounded_count+1)`; else `provisional = true`, `confidence = max provisional confidence`.
5. **Sort** (`sort_causes`): grounded before provisional, then confidence desc, then action asc, then domain asc.
6. **Ledger** (`reverse_query_v1`): answer digest, cause/grounded/provisional counts, per-cause digests, `MAX_REVERSE_DEPTH`, full `ReverseStats`, `ts`. The returned ledger ref is stamped into every cause's `provenance`.

`grounded_confidence(count) = count / (count + 1)` (line 335) is the shared
grounded confidence used by both reverse and the accumulator.

---

## 14.7 Completion — `complete` (`src/complete.rs`)

Fills the *free* slots of a partial constellation by energy descent toward
Ward trusted-region attractors, holding *clamped* (measured) slots fixed.

Constant: `COMPLETION_LEDGER_TAG = "oracle_completion_v1"`.

Traits:
- `CompletionRegion::members_for_lens(domain, cx, lens_id) -> Result<Vec<Vec<f32>>>` — supplies attractor vectors per free lens.
- `CompletionLedger::append_completion(payload) -> Result<LedgerRef>`.
- `WardCompletionRegion<'a>` (struct holding `&Panel`, `&[TrustedRegion]`) implements `CompletionRegion` by collecting each region's slot vector for the lens's `slot_id`.

`CompletionLedgerPayload` (public) fields: `tag`, `domain_id`, `cx_id`, `clamp: Vec<String>`, `free: Vec<String>`, `confidence`, `energy`, `converged`, `ceiling`, `ts`.

Entry points:
- `complete<C, R>(vault, cx, panel, domain, clamp, free, region, self_consistency, anneal, clock)` — wires a `VaultSufficiencyAssay` and an `AsterCompletionLedger`, then delegates.
- `complete_with_assay_and_region<A, L, R>(...)` — generic over assay / ledger / region (the testable core).

### Algorithm (steps, lines 142–218)

1. **Validate request** (`validate_request`): `cx.panel_version` must equal `panel.version` (else `CalyxError::stale_derived`). Compute `all_slots` from panel lenses. Require `clamp`/`free` disjoint (`overlap`), `clamp ∪ free` exhaustive over `all_slots` and present in `cx`, and no `extra` slots; else `SlotConflict`.
2. **Sufficiency** (`check_sufficiency_with_assay`) — produces a `SufficiencyBound`; its `sufficient` flag gates whether inferred slots may be tagged `Inferred`.
3. **Dense vectors** (`dense_vectors_by_lens`): require `SlotVector::Dense` for present slots (`Sparse`/`Multi` → `lens_dim_mismatch`).
4. **Measured slots** (`measured_slots`): each clamped lens becomes a `TaggedSlot { tag: Measured }` from its dense vector; a missing clamp vector → `SlotConflict`.
5. **Per free lens** (sorted): fetch region members; initial vector = the cx slot vector if present, else the `mean_vector` of members; `beta = get_beta(domain, anneal)`; run `descend(&mut vector, members, beta, MAX_STEPS=20, DEFAULT_EPS=1e-4)`. Tag = `Inferred` iff `descent.converged && sufficiency.sufficient`, else `Provisional`. Record a `SlotDescent { final_energy, converged, member_count }`.
6. **Draft confidence** (`CompletionDraft::from_descents`, lines 262–289):
   - No descents (no free slots): `confidence = min(1.0, ceiling)`, `converged = true`, `energy = 0`.
   - Else `mean_energy = mean(final_energy)`, `mean_log_members = mean(ln(member_count))`, and

     ```
     raw_confidence = 1.0                    if mean_log_members <= EPSILON
                    = 1.0 - mean_energy / mean_log_members   otherwise
     confidence = raw_confidence.clamp(0,1).min(valid_ceiling(ceiling))
     converged  = all descents converged
     energy     = mean_energy
     ```

7. **Ledger** (`oracle_completion_v1`) then construct `CompletionResult::new(...)` with the slot partition validated again (section 14.2.1).

---

## 14.8 Honesty gate / sufficiency (`src/honesty_gate.rs`)

`check_sufficiency<C>(vault, panel, domain, clock) -> Result<SufficiencyBound>`
and the assay-generic `check_sufficiency_with_assay<A>(assay, ...)`.

Constant: `SOLE_CARRIER_BITS = 0.10`.

Trait `SufficiencyAssay::panel_sufficiency(panel, domain, clock) -> Result<PanelSufficiency>`.
`VaultSufficiencyAssay<'a, C>` implements it by loading an `AssayStore` from the
vault (cross-ref doc 09, Loom / Assay / DDA) under an `AssayCacheKey::scoped(panel.version, domain, vault_id, AnchorKind::Reward)`,
reading the required `Panel`, `OutcomeEntropy`, and per-`Lens` rows, computing
`per_sensor_attribution(slot_bits, SOLE_CARRIER_BITS)`, then calling
`panel_sufficiency_with_context(...)` with a `DeficitRoutingContext`
(`panel_id = "oracle:{domain}:panel:{version}"`, anchor `Reward`, `computed_at_seq = clock.now()`).

### Decision (steps)

1. `validate_report`: `panel_bits` and `anchor_entropy_bits` must be finite ≥ 0.
2. `sufficient = report.panel_bits >= report.anchor_entropy_bits`.
3. If sufficient → `Ok(SufficiencyBound { i_panel_oracle = panel_bits, dpi_ceiling = panel_bits, sufficient = true, per_sensor_deficit = [] })`.
4. If not sufficient → compute `lens_deficits` (per-lens positive deficit bits summed from `report.deficits` slot/per-slot gaps; empty attribution → `assay_insufficient_samples` error) and return `Err(OracleError::Insufficient { bound })`.

`bits()` rejects non-finite/negative bits (`aster_corrupt_shard`); `trust()`
reads the panel row's `TrustTag` (default `Provisional`).

---

## 14.9 Oracle self-consistency (`src/self_consistency.rs`)

`oracle_self_consistency<C>(vault, domain, clock) -> Result<OracleSelfConsistency>`.

Constants: `MIN_FLAKINESS_PAIRS = 10`, `MIN_VALIDITY_SAMPLES = MIN_ASSAY_SAMPLES`
(from `calyx-assay`), `KSG_K = 3`, `LEDGER_TAG = "oracle_self_consistency_v1"`.

### Algorithm (steps)

1. **Domain series** (`domain_series`): scan Base CF, keep domain-matching constellations, read each one's recurrence occurrences. Empty → `DomainNotFound`.
2. **Per series** parse occurrence contexts (`RecurrenceEvidence`): verdict label from `oracle_verdict` else `outcome_anchor`; ground-truth label from `ground_truth_anchor` (optional).
3. **Flakiness.** For each series count verdict-label agreement pairs. `pair_count(n) = n*(n-1)/2`. Accumulate `total_pairs` and `agreement_pairs = Σ pair_count(count_per_label)`. Require `total_pairs >= 10`, else `NoRecurrence`.

   ```
   flakiness = (1.0 - agreement_pairs/total_pairs).clamp(0,1)
   ```

4. **Validity** (`validity`, lines 161–198):
   - No ground-truth samples → `(0.0, provisional = true)`.
   - Fewer than `MIN_VALIDITY_SAMPLES` → `NoRecurrence`.
   - All verdict == ground_truth → `(1.0, false)`.
   - Ground-truth entropy ≈ 0 → fraction of exact matches.
   - Otherwise `validity = (KSG_MI(one-hot verdict ; ground-truth codes, k=3).bits / entropy_bits(ground_truth)).clamp(0,1)` via `ksg_mi_continuous_discrete` (cross-ref doc 09 Assay MI estimators).
5. **Ceiling** = `validity * (1 - flakiness)` (computed inside `OracleSelfConsistency::with_provenance`).
6. **Ledger** (`oracle_self_consistency_v1`, `EntryKind::Assay`): domain digest, `pair_count`, `agreement_pairs`, `validity_samples`, `flakiness`, `validity`, `ceiling`, `provisional`, `ts`. The ledger ref is stored back into the result's `provenance`.

---

## 14.10 Super-intelligence tiers 1–3 (`src/super_intel.rs`)

Constants: `ORACLE_CLEAN_THRESHOLD = 0.7`, `KERNEL_RECALL_RATIO = 0.95`.

Traits / types:
- `OracleConsistencySource` (impl for `AsterVault<C>` → calls `oracle_self_consistency`).
- `KernelRecallSource::kernel_recall_report(held_out, clock) -> Result<RecallReport, LodestarError>`.
- `KernelRecallGate<'a>` wraps a `KernelIndex`, a full `AnnIndex`, a `CorpusReader`, and `RecallTestParams` (forces `min_recall_ratio = 0.95`); calls `kernel_recall_test_with_clock` (cross-ref doc 10, Lodestar / Graph Kernel).
- `HeldOutSplit { split_id, training_ids: Vec<CxId>, held_out_ids: Vec<CxId> }` with `held_out_count()` and `has_training_leakage()` (any held-out id also in training).
- `ShortCircuit { Enabled, MeasureAll (default) }`.
- `TierMeasurementRequest<'a, O, A, K>` aggregates oracle / assay / kernel sources, panel, domain, held-out, clock, short-circuit.

| Tier | Measure fn | Measured value | Threshold | Pass condition |
|------|-----------|----------------|-----------|----------------|
| `OracleClean` | `measure_tier_oracle_clean[_with_source]` | self-consistency `ceiling` | `0.7` | `ceiling >= 0.7` |
| `PanelSufficient` | `measure_tier_panel_sufficient[_with_assay]` | `panel_bits` | `anchor_entropy_bits` | `panel_bits >= anchor_entropy_bits` |
| `KernelExists` | `measure_tier_kernel_exists` | recall `ratio` | `0.95` | `ratio >= 0.95` and `n_queries_tested > 0` |

`measure_tiers_1_to_3` runs the three in order, honoring `ShortCircuit::Enabled`
(stop after a failing tier). `measure_super_intelligence_tiers_1_to_3` wraps the
results in a `SuperIntelReport`.

Shared scoring helpers:
- `measured_tier(tier, measured_value, threshold, fix)` → passes iff both finite, both ≥ 0, and `measured_value >= threshold`; sanitizes non-finite to `0.0`; attaches `fix` only when failing.
- `failed_tier(tier, threshold, fix)` → `passed = false`, `measured_value = 0`.
- `valid_measurement(v, t)` → both finite and ≥ 0.

Cheapest-fix strings are derived per tier (e.g. `oracle_clean_fix` returns "add
validity-tracking anchor" when provisional or validity < 0.7, else "label more
oracle instances to reduce flakiness"; `panel_sufficiency_fix` names the
max-deficit lens with its bit gap).

---

## 14.11 Super-intelligence full predicate, tiers 4–6 (`src/super_intel_full.rs`)

Constants: `GOODHART_THRESHOLD = 0.9`, `CALIBRATION_CEILING_DELTA = 0.0`,
`LEDGER_TAG = "super_intelligence_v1"`.

Measurement structs (public): `CalibrationMeasurement { calibration_error, held_out_count, calibrated_slots }`,
`GoodhartDefenseMeasurement { pass_rate, held_out_count, report_passed, violation_count }`,
`MistakeClosureMeasurement { recurring_mistakes, replayed_mistakes }`.

Source traits and their concrete impls:
- `CalibrationSource` — impl for `calyx_ward::GuardProfile`. Validates the guard domain matches; requires a calibration profile (else `Provisional` ward error); takes the max per-slot FAR (or overall FAR) as `calibration_error` (cross-ref doc Ward).
- `GoodhartDefenseSource` — impl for `calyx_anneal::GoodhartReport`. `pass_rate = in_region_frac` (or 1/0 from `passed`); records `violation_count`.
- `MistakeClosureSource` — impl for `calyx_anneal::RegressionReport`. Validates via `regression_rate`; `recurring_mistakes = regression_count`, `replayed_mistakes = results.len()` (cross-ref doc Anneal).

| Tier | Measure fn | Measured value | Threshold | Pass condition |
|------|-----------|----------------|-----------|----------------|
| `Calibrated` | `measure_tier_calibrated` | `calibration_error` | `oracle_ceiling + 0.0` | `calibration_error <= threshold` (held-out non-empty) |
| `GoodhartDefended` | `measure_tier_goodhart_defended` | `pass_rate` | `0.9` | `report_passed && pass_rate >= 0.9` |
| `MistakeClosed` | `measure_tier_mistake_closed` | `recurring_mistakes` (as f32) | `0.0` | `recurring_mistakes == 0` |

`measure_super_intelligence_tiers(request)` runs all six in `Tier::ORDER`,
threading the oracle self-consistency `ceiling` into the calibration threshold,
and honoring `ShortCircuit` (`should_stop` halts when the last pushed tier
failed). `super_intelligence(vault, request)` returns the report;
`super_intelligence_with_ledger` also returns the ledger ref;
`write_super_intelligence_ledger` writes the `super_intelligence_v1` payload
(`overall`, `failing_tier`, `cheapest_fix`, all tiers, `ts`) under
`EntryKind::Assay`.

`SuperIntelligenceRequest<'a, O, A, K, C, G, M>` carries all six tier sources
plus panel/domain/held-out/clock/short-circuit.

---

## 14.12 Time prediction (`src/time_prediction.rs`)

`predict_next_occurrence[_with_tz_offset]` (vault + cx_id) and
`predict_next_occurrence_from_series[_with_tz_offset]` (direct `RecurrenceSeries`).
Returns `TimePrediction`.

Constants: `MIN_TIME_PREDICTION_OCCURRENCES = 3`, `FULL_CONFIDENCE_SUPPORT = 12.0`,
`SECS_PER_HOUR = 3600`, `SECS_PER_DAY = 86400`, `UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO = 3`.

`TimePrediction` fields: `cx_id`, `sufficient`, `support`, `active_support`,
`rolled_support`, `rollup_period_estimate_secs: Option<f64>`, `tz_offset_secs`,
`t_hat: EpochSecs`, `confidence`, `confidence_ceiling`, `cadence_secs`,
`cadence_mad_secs`, `interval: TimePredictionInterval { low, high }`,
`periodic_confidence`. `TimeBucket { hour, day_of_week, tz_offset_secs }`.

### Algorithm (steps)

1. Validate `confidence_ceiling` finite in [0,1].
2. Sort occurrence times. If fewer than 3 active occurrences: if rolled support > 0 → "rolled recurrence … cannot define cadence" error; else "sparse recurrence series" error (both `CALYX_ORACLE_INSUFFICIENT`).
3. `gaps` = positive consecutive deltas (timestamps must be strictly increasing).
4. `cadence_secs = median(gaps)`; must be finite and > 0.
5. `cadence_mad_secs = median(|gap - cadence|)` (median absolute deviation).
6. `t_hat = last_time + round(cadence_secs)` (checked add).
7. **Periodic confidence** (`periodic_confidence_with_tz_offset`): max of three mode fractions — joint (hour×day, 24·7 buckets), hour-of-day (24), day-of-week (7) — each = `max_bucket_count / n`.
8. **Confidence** (`confidence`, lines 193–205):

   ```
   regularity         = (1.0 - cadence_mad/cadence).clamp(0,1)
   support_confidence = min(support / 12.0, 1.0)
   confidence = (regularity * support_confidence * periodic_confidence)
                  .min(confidence_ceiling).clamp(0,1)
   ```

9. **Interval**: `half_width = round(max(cadence_mad, cadence * (1 - confidence)))`; `interval = [t_hat - half_width, t_hat + half_width]` (checked, half-width ≥ 0).

`total_support = max(series.frequency, occurrences.len())`;
`rolled_support = frequency - occurrences.len()` (saturating);
`time_bucket` maps an epoch second + tz offset to local hour/day-of-week using
`rem_euclid`/`div_euclid` (Monday = 0 via the epoch offset constant).

---

## 14.13 PRD-22 formula primitives (`src/prd22.rs`)

Pure formula functions over `calyx_paths::AssocGraph` (cross-ref doc 10) — no
vault, no ledger. Re-exported from the crate root under aliased names
(`oracle_formula_predict`, `reverse_query_formula`, `super_intelligence_formula`).

Types: `OracleCeiling`, `OraclePrediction`, `ConsequenceExpansion { cx_id, score }`,
`SuperIntelligenceEvidence`, `SuperIntelligenceVerdict { pass, failing_tiers }`.

| Function | Behavior |
|----------|----------|
| `oracle_ceiling(tau_corr, flakiness, validity)` | validates each ∈ [0,1]; `oracle_self_consistency = (validity*(1-flakiness)).clamp(0,1)`; `capped_tau = tau_corr.min(oracle_self_consistency)` |
| `oracle_predict(panel_bits, anchor_entropy_bits, requested_confidence)` | `deficit_bits = max(anchor_entropy_bits - panel_bits, 0)`; if `deficit > EPSILON` → `CALYX_ORACLE_INSUFFICIENT`; else returns `requested_confidence` |
| `butterfly_expand(graph, source, max_hops)` | `reach_scored` over the graph; sorts by score desc then cx_id asc |
| `reverse_query(graph, answer, max_hops)` | reverses every edge (`reverse_graph`) then `butterfly_expand` from `answer` |
| `super_intelligence(evidence)` | collects failing-tier labels (`clean`, `sufficient`, `kernel` if `kernel_recall_ratio < min`, `calibrated`, `goodhart`, `mistake_closed`); `pass = failing_tiers.is_empty()` |

The in-file unit test pins known values, e.g. `oracle_ceiling(0.9,0.2,0.75)` →
`oracle_self_consistency = 0.6`, `capped_tau = 0.6`.

---

## 14.14 Energy descent substrate (`src/energy.rs`)

PH51 energy substrate used by `complete()` (section 14.7). No vault/ledger.

Constants: `MAX_STEPS = 20`, `DEFAULT_EPS = 1e-4`, `DEFAULT_BETA = 1.0`,
error codes `CALYX_ORACLE_ENERGY_EMPTY_REGION`, `CALYX_ORACLE_ENERGY_INVALID_INPUT`.

`DescentResult { steps_taken, converged, final_energy }`. Trait
`AnnealConfig::energy_beta(domain) -> Option<f32>`; `get_beta(domain, anneal)`
returns the configured beta if finite ≥ 0, else `DEFAULT_BETA`.

Formulas (free-energy over cosine similarities to region attractors; uses
`calyx_forge::cpu::cosine_batch` / `normalize_f32`, cross-ref doc 06):

- **`energy(x, region_members, beta)`** — validate beta ≥ 0 finite, validate region shape (non-empty, matching dims, finite). If `beta == 0` → `-ln(|members|)`. Else `scaled_i = beta * cos(x, member_i)` and

  ```
  energy = -log_sum_exp(scaled)
  ```

- **`energy_softmax_weights(x, region_members, beta)`** — uniform `1/n` when `beta == 0`; else `stable_softmax(scaled)` = `exp(score_i - log_sum_exp)`.
- **`descent_step(free_slot, region_members, beta)`** — weighted mean of members by softmax weights, then `normalize_f32` in place.
- **`descend(free_slot, region_members, beta, max_steps, eps)`** — validate `eps ≥ 0`; iterate `descent_step`; converge when a single member, or `|next_energy - prev_energy| < eps`; cap at `max_steps`.

Forge errors are wrapped into `OracleError::AssayFailure` with remediation
"repair Forge cosine/normalize inputs before energy descent".

---

## 14.15 Provenance summary (cross-ref doc 11)

| Ledger tag | Writer | EntryKind |
|------------|--------|-----------|
| `oracle_predict_v1` | `predict::write_prediction_ledger` | `Answer` |
| `oracle_expand_v1` | `butterfly::write_expansion_ledger` | `Answer` |
| `reverse_query_v1` | `reverse_query::write_reverse_ledger` | `Answer` |
| `oracle_completion_v1` | `complete::AsterCompletionLedger` | `Answer` |
| `oracle_self_consistency_v1` | `self_consistency::write_ledger` | `Assay` |
| `super_intelligence_v1` | `super_intel_full::write_super_intelligence_ledger` | `Assay` |

Subject ids are content-addressed (`calyx_core::content_address`) over the
domain / action / outcome digests relevant to each primitive. The `energy` and
`prd22` modules and the honesty gate do not write ledger entries.

---

## 14.16 Cross-references

- Doc 04 (Core Foundation) — `AnchorValue`, `LensId`, `Panel`, `Constellation`, `LedgerRef`, `Clock`, `content_address`.
- Doc 05 (Aster Storage) — `AsterVault`, `ColumnFamily::Base`, `read_series`, `RecurrenceSeries`, `EpochSecs`.
- Doc 06 (Forge Math Runtime) — `cosine_batch`, `normalize_f32` used by energy descent.
- Doc 08 (Sextant Search) — sibling Stage; oracle traversal is recurrence-driven rather than ANN-driven.
- Doc 09 (Loom / Assay / DDA) — `AssayStore`, `PanelSufficiency`, `entropy_bits`, `ksg_mi_continuous_discrete`, `MIN_ASSAY_SAMPLES`.
- Doc 10 (Graph Kernel / Lodestar / Paths) — `KernelIndex`, `AnnIndex`, `kernel_recall_test_with_clock`, `RecallReport`; `AssocGraph` / `reach_scored` for PRD-22 primitives.
- Doc 11 (Ledger / Provenance) — `append_ledger_entry`, `EntryKind`, `SubjectId`, `ActorId`.
- Ward / Anneal crates — `GuardVerdict`, `GuardProfile`, `TrustedRegion`, `CalibrationMeta`, `GoodhartReport`, `RegressionReport`.
