# 13 — calyx-ward (Fail-Closed Guard, Conformal Calibration)

**Source files covered:**
- `crates/calyx-ward/src/lib.rs`
- `crates/calyx-ward/src/profile.rs`
- `crates/calyx-ward/src/guard.rs`
- `crates/calyx-ward/src/verdict.rs`
- `crates/calyx-ward/src/calibrate.rs`
- `crates/calyx-ward/src/error.rs`
- `crates/calyx-ward/src/novelty.rs`
- `crates/calyx-ward/src/query.rs`
- `crates/calyx-ward/src/required.rs`
- `crates/calyx-ward/src/drift.rs`
- `crates/calyx-ward/src/generate.rs`
- `crates/calyx-ward/src/ledger.rs`
- `crates/calyx-ward/src/identity.rs`
- `crates/calyx-ward/src/polis.rs`
- `crates/calyx-ward/src/speaker_lens.rs`
- `crates/calyx-ward/src/style_lens.rs`
- `crates/calyx-core/src/cosine.rs` (`dense_cosine`, used by the guard)

calyx-ward is the fail-closed guard. It scores every required slot **independently** against a per-slot calibrated cosine threshold `tau` (no single averaged/flattened gate — INVARIANT A3, marked in `guard.rs`), and refuses out-of-distribution or ungrounded content. Per-slot thresholds are fixed by **conformal calibration** to a target false-accept rate (FAR).

See also: [05_core.md](05_core.md) (SlotId, AnchorKind, Lens, dense_cosine, Clock), [11_assay_signal_bits.md](11_assay_signal_bits.md) (load-bearing bits → required slots), [12_lodestar_kernel.md](12_lodestar_kernel.md) (kernel-near regions), [14_ledger_provenance.md](14_ledger_provenance.md) (calyx-ledger provenance writers), [15_anneal_optimization.md](15_anneal_optimization.md) (drift re-calibration hook).

---

## 1. The per-slot guard profile

### 1.1 `GuardProfile` (profile.rs)

The configuration object every guard call reads.

| Field | Type | Meaning |
|---|---|---|
| `guard_id` | `GuardId` | Stable profile id (newtype over `Uuid`, `#[serde(transparent)]`). |
| `panel_version` | `u64` | Version of the panel the profile was derived from. |
| `domain` | `String` | Domain label. |
| `tau` | `BTreeMap<SlotId, f32>` | Per-slot calibrated cosine threshold. |
| `required_slots` | `Vec<SlotId>` | Slots that must be scored for the constellation to pass. |
| `policy` | `GuardPolicy` | How per-slot passes combine (§2.3). |
| `calibration` | `Option<CalibrationMeta>` | Calibration provenance; `None` = provisional. |
| `novelty_action` | `NoveltyAction` | What a FAIL does (§4). |

Methods: `is_calibrated()` (`calibration.is_some()`), `tau_for(&SlotId) -> Option<f32>` (`None` means the slot is not guarded). `GuardProfile` implements `calyx_core::GuardTauProfile`. `GuardId` provides `new`, `as_uuid`, `From<Uuid>`, `Display`, and `FromStr` (UUID parse).

### 1.2 Cosine policy

Scoring uses `calyx_core::dense_cosine(left, right) -> Option<f32>` (`cosine.rs`). It returns `None` (treated as fail) when vectors differ in length, are empty, contain non-finite values, or have a zero/non-finite denominator (norm). It does **not** assume inputs are pre-normalized — it divides by `‖left‖·‖right‖`. A slot passes iff `cos >= tau` (`guard.rs::guard`). Boundary `cos == tau` PASSES (test `boundary_cos_equal_tau_passes`).

### 1.3 Per-slot thresholds

`tau` is per-slot. Constants:

| Constant | Value | File |
|---|---|---|
| `DEFAULT_TAU` | `0.7` | guard.rs |
| `TAU_COLD_START` | `= DEFAULT_TAU` (0.7) | calibrate.rs |
| `CIVIC_TAU` | `0.7` | polis.rs |
| `LOAD_BEARING_MIN_BITS` | `0.05` | required.rs |

When a required slot has no `tau` entry, `guard()` uses `DEFAULT_TAU` (0.7) — the cold-start prior (test `absent_tau_uses_default_threshold`). Required slots are derived from Assay bits (§7).

---

## 2. Verdict logic

### 2.1 Verdict types (verdict.rs)

`SlotVerdict`: `{ slot: SlotId, cos: f32, tau: f32, pass: bool }`.

`GuardVerdict`: `{ guard_id: GuardId, overall_pass: bool, provisional: bool (#[serde(default)]), per_slot: Vec<SlotVerdict>, action: Option<NoveltyAction> }`. Helpers: `failing_slots() -> Vec<&SlotVerdict>`, `all_slot_details() -> &[SlotVerdict]`. The full per-slot decomposition is always preserved (for callers and FSV readback), even on PASS.

### 2.2 `guard()` steps (guard.rs)

`guard(profile, produced: &ProducedSlots, matched: &MatchedSlots, high_stakes: bool) -> Result<GuardVerdict, WardError>` where `ProducedSlots = MatchedSlots = BTreeMap<SlotId, Vec<f32>>`.

1. Build `required` = `profile.required_slots`, sorted + deduped.
2. `validate_non_inert_required`: error if `required` empty (`InertProfile{reason:"empty_required_slots"}`); for `KofN{k}`, `k==0` → `InertProfile{reason:"kofn_zero"}`, `k > required.len()` → `PolicyViolation`.
3. If `high_stakes`: `validate_high_stakes_profile` requires `calibration.is_some()` (else `Provisional`) and every required slot to have both a `tau` entry **and** a `calibration.per_slot` entry (else `MissingSlotCalibration`).
4. For each required slot: fetch produced & matched vectors (missing either → fail-closed `MissingSlot`), `tau = tau_for(slot).unwrap_or(DEFAULT_TAU)`, compute `dense_cosine`. On `Some(cos)`: `pass = cos >= tau`. On `None` (invalid/mismatched/zero vector): record `cos = 0.0, pass = false` — no panic. Push `SlotVerdict`.
5. `pass_count` = number of passing slots.
6. Combine per §2.3 → `overall_pass`.
7. `action = (!overall_pass).then(|| profile.novelty_action.clone())` (PASS → `None`).
8. `provisional = !profile.is_calibrated()`.

### 2.3 AND-combination rule (exact)

```
pass_count = |{ s in per_slot : s.cos >= s.tau }|
AllRequired  : overall_pass = (pass_count == per_slot.len())   // logical AND of all slots
KofN { k }   : overall_pass = (pass_count >= k)
```

`AllRequired` is the strict no-flatten AND: every load-bearing axis must individually clear its own `tau`. There is no averaged/flattened vector gate (INVARIANT A3).

### 2.4 Wrapper / states

- `guard_non_high_stakes(profile, produced, matched)` → `guard(.., false)`.
- `guard_result` / `guard_result_with_stakes`: run `guard`, return verdict on PASS, else `Err(WardError::Ood{guard_id, failing})` where `failing` = the non-passing slot verdicts. This is the "refuse" surface.
- `validate_non_inert_profile(profile)`: pre-flight inert/policy check.

The verdict surface has these outcomes: **PASS** (`overall_pass=true`); **FAIL** with `action` = `NewRegion` (new-region/learn), `Quarantine`, or `RejectClosed` (refuse). At the generation layer (§5) these map to `Accepted` / `Novel` / `Rejected`.

---

## 3. Conformal calibration (calibrate.rs)

Maps a target FAR to a per-slot `tau`. Constants: `MIN_BAD_SCORES = 50`, `ESTIMATOR = "conformal_quantile_v1"`.

### 3.1 `SlotKind` and default target FAR (the FAR defaults)

`SlotKind::default_target_far()`:

| SlotKind | default_target_far |
|---|---|
| `Identity` | **0.01** (strictest) |
| `Stylistic` | **0.05** (loosest) |
| `Content` | **0.03** |

There is no single global default FAR constant; the default FAR is per slot-kind via `default_target_far()`, and a `CalibrationInput.target_far` exceeding the slot-kind ceiling is rejected (`InvalidCalibrationInput`).

### 3.2 `CalibrationInput`

`{ slot: SlotId, good_scores: Vec<f32>, bad_scores: Vec<f32>, slot_kind: SlotKind, target_far: f32 }`. `good_scores` = cos for known-good outputs, `bad_scores` = cos for known-bad (injection/OOD/wrong).

### 3.3 `calibrate_slot(input, alpha, clock)` algorithm

`alpha` = the confidence-bound miss probability (`confidence = 1 - alpha`).

1. Validate: `alpha` and `target_far` finite in `[0,1]`; `target_far <= slot_kind.default_target_far()`.
2. Require `bad_scores.len() >= MIN_BAD_SCORES` (50), else `InsufficientCalibrationData{n,min}`.
3. Validate every score finite and in `[-1,1]` (cosine range), then sort ascending (`total_cmp`).
4. `tau = conformal_tau(sorted_bad, target_far, alpha)` (§3.4).
5. Achieved `far` = fraction of `bad_scores >= tau`. Achieved `frr` = fraction of `good_scores < tau` (0.0 if no good scores).
6. `corpus_hash` = SHA-256 over `slot ‖ slot_kind ‖ target_far(le) ‖ alpha(le) ‖ good_scores(le) ‖ 0xff ‖ bad_scores(le)`.
7. Return `(tau, CalibrationMeta::new(corpus_hash, ESTIMATOR, far, frr, 1.0 - alpha, clock))`.

### 3.4 `conformal_tau` — the exact threshold formula

`tau` is chosen as the **smallest candidate cosine** whose empirical FAR on the bad set is at most `target_far` **and** whose one-sided binomial confidence bound holds:

1. If `bad_scores` empty → `InsufficientCalibrationData`.
2. If `target_far == 0.0` → return `next_above(max(bad_scores))` (just above the largest bad score; FAR=0).
3. Build candidate set: for each distinct sorted bad score `s`, add `s` and `next_above(s)` (next representable f32). Sort + dedup.
4. For each candidate `c` ascending:
   - `bad_accepts` = count of `bad_scores >= c`; `candidate_far = bad_accepts / n_bad`.
   - Accept `c` iff `candidate_far <= target_far + EPSILON` **and** `confidence_bound_satisfied(bad_accepts, n_bad, target_far, alpha)`.
   - Return the first such `c`.
5. If none qualifies → return `next_above(max(bad_scores))`.

`confidence_bound_satisfied`: `binomial_cdf_at_most(bad_accepts, n_bad, target_far) <= alpha + EPSILON`. The binomial CDF `P[X <= bad_accepts]` with `X ~ Binomial(n_bad, target_far)` is computed by iterative term accumulation from `(1-p)^n`. This is a one-sided conformal guarantee: with confidence `1 - alpha`, the true accept rate of bad inputs is bounded by `target_far`.

`next_above(v)`: next representable f32 (`from_bits(bits+1)` for v>0, `from_bits(bits-1)` for v<0, `from_bits(1)` for v==0).

### 3.5 `calibrate(profile_template, inputs, alpha, clock)`

Calibrates a whole profile: for each input runs `calibrate_slot`, inserts `tau`, adds the slot to `required_slots` if absent. Sorts+dedups `required_slots`. Sets `profile.calibration = Some(merge_meta(...))`. `merge_meta`: profile-level `corpus_hash` = SHA-256 over each `(slot, slot.corpus_hash)`; profile `far`/`frr` = **max** over slots (worst case); `per_slot` = `BTreeMap<SlotId, SlotCalibrationMeta>`. Empty inputs → `InvalidCalibrationInput`.

### 3.6 Provenance structs

`CalibrationMeta { corpus_hash: [u8;32], estimator: String, far, frr, confidence: f32, ts: i64, per_slot: BTreeMap<SlotId, SlotCalibrationMeta> }`. `ts` from injected `Clock`. `SlotCalibrationMeta` = same fields minus `per_slot`. `is_calibrated()` is true once `calibration` is set.

---

## 4. OOD / trusted-region representation and refuse/quarantine/learn paths

### 4.1 `NoveltyAction` (profile.rs)

| Variant | Meaning | Generation/query effect |
|---|---|---|
| `NewRegion` | FAIL opens a new safe region (learn). | record status `AwaitingGrounding`; `GenerateOutput::Novel`. |
| `Quarantine` | hold for review. | record status `Quarantined`; `Novel`. |
| `RejectClosed` | refuse, fail closed (high-stakes). | `NoveltyHandler` returns `WardError::Ood`; generation → `Rejected`. |

### 4.2 Query-time trusted regions (query.rs)

`TrustedRegion { cx_id: CxId, slots: MatchedSlots }` represents a grounded constellation region.

`QueryVerdict` (`#[serde(tag="status")]`): `Pass { nearest_cx, gap: f32, per_slot }` | `Ood { nearest_cx: Option<CxId>, gap: Option<f32>, per_slot, action: NoveltyAction }`. `is_pass()` helper.

`RegionSource`: `KernelNear` | `Peripheral`.

`KernelFirstQueryVerdict`: same shape as `QueryVerdict` plus `match_source` (kernel vs peripheral).

`guard_query_kernel_first(profile, query_slots, kernel_regions, peripheral_regions)` steps:
1. `validate_non_inert_profile`.
2. Evaluate kernel regions: run `guard_non_high_stakes` against each region's slots; track best PASS and best OOD by margin (`nearest_margin` = `min over slots of cos - tau`). Return first kernel PASS (Lodestar kernel-near regions are checked first).
3. If no kernel PASS, evaluate peripheral regions the same way; return a peripheral PASS if any.
4. Otherwise build an `Ood` verdict: nearest = candidate with greatest margin (kernel vs peripheral), `gap = max(0, -margin)`, `action = profile.novelty_action`; if no candidates, all `None` with empty `per_slot`.

`guard_query` wraps `guard_query_kernel_first` with an empty kernel set, collapsing to `QueryVerdict`.

### 4.3 Novelty routing for produced/generated FAILs (novelty.rs)

`NoveltyHandler::new(vault: Arc<dyn VaultSink>, clock: Arc<dyn Clock>)`. `handle(profile, verdict, produced)`:
- `verdict.guard_id != profile.guard_id` → `GuardIdMismatch`.
- `verdict.overall_pass` → `NotAFailure` (novelty handling requires a FAIL).
- Map `novelty_action` → `NoveltyStatus`: `NewRegion→AwaitingGrounding`, `Quarantine→Quarantined`, `RejectClosed→Rejected`.
- Write a `NoveltyRecord` via the sink; if `RejectClosed`, then **also** return `WardError::Ood` (fail closed) after persisting; otherwise return the record.

`NoveltyRecord { novel_id: NovelId, guard_id, produced_slots: ProducedSlots, failing_verdicts: Vec<SlotVerdict>, action_taken: NoveltyAction, ts: i64, status: NoveltyStatus }`. `novel_id` is a deterministic UUIDv4-shaped SHA-256 digest over guard_id, panel_version, domain, ts, produced vectors, and per-slot verdicts. `VaultSink` trait: `write_novel(&NoveltyRecord)`, `novel_records() -> Vec<NoveltyRecord>`. `novel_regions(vault, since_ts)` lists `AwaitingGrounding` records at/after `since_ts`.

### 4.4 Recurrence / surprise novelty signals (novelty.rs, Aster-backed)

`NoveltySignal` (`#[serde(tag="signal")]`): `Recurring{frequency:u64, cadence_secs:f64}` | `NonRecurring` | `OverdueRecurrence{expected_t:EpochSecs, overdue_by_secs:u64}` | `Anomaly{surprise_bits:SurpriseScore}`.

- `classify_novelty(cx_id, vault, clock)`: reads base `frequency` scalar from the `AsterVault`; `<=1`→`NonRecurring`; for `freq>=3` with a finite positive cadence, if `now > last_t + 2·cadence` → `OverdueRecurrence`; else `Recurring`.
- `surprise_bits` / `surprise_score_from_counts(frequency, total)`: `bits = -log2(p)`, `p = clamp(max(freq,1)/total, MIN_POSITIVE, 1.0)`. Annotated: surprise is a retrieval-anomaly signal only and MUST NOT modify stored bits.
- `novelty_action_for_signal`: `Recurring→None`, `NonRecurring|OverdueRecurrence→NewRegion`, `Anomaly→Quarantine`.
- `overdue_recurrence_scan(domain, vault, clock)` returns overdue cx_ids. `Domain { id: String, cx_ids: Vec<CxId> }`. `SurpriseScore` (transparent f32 newtype, `new` requires finite ≥ 0).

`FREQUENCY_SCALAR`/`read_series` come from calyx-aster; frequency must be a non-negative integer or `InvalidFrequency`/`MissingFrequency` fail closed. See [06_aster_storage_engine.md](06_aster_storage_engine.md).

---

## 5. Generation-time integration (generate.rs, identity.rs)

`GenerateInput { candidate_audio: Option<Vec<f32>>, candidate_text: Option<String>, sample_rate: u32, matched_cx_id: CxId }`.

`GenerateOutput`: `Accepted { verdict, provenance_tag, ledger_ref: Option<LedgerRef> }` | `Novel { record } ` | `Rejected { verdict, provenance_tag, ledger_ref }`.

Provenance tags: `GUARDED_PASS_TAG="guarded:pass"`, `GUARDED_REJECT_TAG="guarded:reject"`, `GUARDED_REJECT_UNPROVENANCED_TAG="guarded:reject:unprovenanced"`.

`guard_generate(identity_profile, input, speaker_lens: &dyn Lens, style_lens: &dyn Lens, novelty_handler, high_stakes)`:
1. Reject inert identity profile (≥1 identity slot + non-inert guard profile).
2. If `high_stakes && !is_calibrated` → `Provisional`.
3. Build produced slots: `SpeakerMatch` slots embed `candidate_audio` via the speaker lens; `StyleHold` slots embed `candidate_text` via the style lens; any other anchor kind → error.
4. `guard(...)` against the profile's cached `matched_slot_cache`.
5. Route: PASS → `Accepted` (`guarded:pass`); FAIL → `NoveltyHandler::handle` → `Novel`, or if `RejectClosed` → `Rejected` (`guarded:reject:unprovenanced`).

`guard_generate_with_ledger` additionally appends the verdict to the ledger (Accepted/Rejected get a `ledger_ref`; Rejected tag becomes `guarded:reject`).

`IdentityProfile { guard_profile: GuardProfile, identity_slots: Vec<IdentitySlotConfig>, matched_slot_cache: MatchedSlots }`. `IdentitySlotConfig { slot_id, anchor_kind: AnchorKind, tau_override: Option<f32> }`; `is_identity_anchor()` = `SpeakerMatch | StyleHold`. `IdentityProfile::new` validates: each identity slot must be in `required_slots` (else `IdentitySlotNotRequired`), no duplicates, anchor kind must be identity, tau present/in `[0,1]`, matched vector present and unit-normalized (zero norm rejected). Custom `Deserialize` re-runs `new()` to enforce invariants on load.

### 5.1 Identity lenses (speaker_lens.rs, style_lens.rs)

Both are frozen ONNX lenses (crate `ort` 2.0.0-rc.12, CUDA feature). Provider policy enum (`CudaFailLoud` / `CpuExplicit`) — CUDA fails loud, no CPU fallback. Each computes a `LensId` from model SHA-256 + source repo/revision + output shape, and unit-normalizes output.

| Lens | Const path | Dim | Notes |
|---|---|---|---|
| `SpeakerLens` (WavLM) | `DEFAULT_WAVLM_MODEL_PATH` | `WAVLM_DIM=512` | `WAVLM_SAMPLE_RATE=16000`; trims edge silence, linear-resamples to 16 kHz. |
| `StyleLens` (RoBERTa style-embed) | `DEFAULT_STYLE_MODEL_PATH`, `DEFAULT_STYLE_TOKENIZER_PATH` | `STYLE_DIM=768` | `STYLE_MAX_TOKENS=512`; tokenizes via `tokenizers`. |

Backends are trait seams (`SpeakerEmbeddingBackend`, `StyleEmbeddingBackend`) so tests inject fakes; production uses the pinned ONNX session.

---

## 6. Drift monitoring (drift.rs)

Constants: `DEFAULT_DRIFT_WINDOW=500`, `DEFAULT_DRIFT_CHANNEL_CAPACITY=32`, `REJECTION_RATE_DRIFT_MULTIPLIER=1.5`.

`DriftMonitor::new(profile, window_size, anneal_hook: Arc<dyn AnnealHook>)` spawns a worker thread reading a bounded `sync_channel`; `record_verdict` pushes each slot's pass/fail into a rolling window (capped at `window_size`). For each slot with a calibrated FAR bound, `drift = rolling_rejection_rate > calibrated_far_bound * 1.5`. On a new drift it sends one `DriftEvent { guard_id, slot, current_rejection_rate, calibrated_far_bound }` to the Anneal hook (channel full/disconnected → `dropped_events += 1`; drift slot de-notified when it recovers). The hot path never blocks.

`AnnealHook` trait: `on_rejection_rate_drift(guard_id, slot, current_rejection_rate, calibrated_far_bound)`. `GuardHealth { guard_id, per_slot_rejection_rate, per_slot_calibrated_far_bound, per_slot_frr, drift: bool, last_calibrated: i64, dropped_events: usize }` via `guard_health(monitor, guard_id)` (unknown guard → zeros). FAR/FRR bounds are taken per-slot from `calibration.per_slot`, falling back to the profile-level max. Anneal recalibration: see [15_anneal_optimization.md](15_anneal_optimization.md).

---

## 7. Required-slot derivation (required.rs)

`RequiredSlotDerivation { anchor: AnchorKind, min_bits: f32, cold_start_tau: f32, manual_required_slots: Option<Vec<SlotId>> }`. Constructors: `assay_bits(anchor)` (`min_bits=0.05`, `cold_start_tau=DEFAULT_TAU`) and `manual(anchor, slots)`.

`derive_required_slots(panel, config)`: for each `Active` slot, reads `slot.bits_about[anchor]`; non-finite bits → error; keeps slots with `bits >= min_bits` (default 0.05), returning `RequiredSlotEvidence { slot, bits }`. `derive_required_profile(profile, panel, config)`: applies manual or Assay-derived slots, inserts `cold_start_tau` for slots lacking a tau (so every required slot has an explicit threshold), and sets `panel_version`. Empty derived set / empty manual set → `InvalidRequiredSlotDerivation`. Bits come from calyx-assay; see [11_assay_signal_bits.md](11_assay_signal_bits.md).

---

## 8. Polis civic guard (polis.rs)

A deterministic synthetic-persona FSV surface (PH70). Constants: `CIVIC_SLOT_COUNT=21`, `CIVIC_TAU=0.7`. `evaluate_polis_civic_pairs(pairs)` builds a fully-calibrated `AllRequired` profile (21 single-element slots, FAR/FRR=0), runs `guard(..., high_stakes=true)` per pair (a pair "ties" iff `overall_pass`), and errors `TieMismatch` if the actual tie ≠ planted tie. Types: `CivicPersona`, `CivicPersonaPair`, `PolisCivicProof`, `PairProof`, `PolisCivicError` (codes `CALYX_POLIS_EMPTY_PERSONA_SET`, `_SLOT_COUNT_MISMATCH`, `_INVALID_AXIS`, `_TIE_MISMATCH`, plus wrapped `Ward`). `synthetic_polis_persona_pairs()` returns 4 fixed pairs.

---

## 9. Errors (error.rs)

`WardError` (enum, `Clone+Debug+PartialEq`, implements `std::error::Error`). Each variant maps to a stable string code via `code()`:

| Variant | Code | Trigger |
|---|---|---|
| `Ood{guard_id, failing}` | `CALYX_GUARD_OOD` | verdict did not pass / fail-closed refuse. |
| `Provisional{guard_id}` | `CALYX_GUARD_PROVISIONAL` | high-stakes use of uncalibrated guard. |
| `MissingSlotCalibration{guard_id,slot}` | `CALYX_GUARD_PROVISIONAL` | required slot lacks high-stakes calibration. |
| `InsufficientCalibrationData{n,min}` | `CALYX_GUARD_PROVISIONAL` | `bad_scores < 50`. |
| `InvalidCalibrationInput{reason}` | `CALYX_GUARD_PROVISIONAL` | bad alpha/target_far/scores. |
| `InvalidRequiredSlotDerivation{reason}` | `CALYX_GUARD_PROVISIONAL` | bad slot derivation / identity config. |
| `InertProfile{guard_id,reason}` | `CALYX_GUARD_INERT_PROFILE` | empty required slots or `kofn_zero`. |
| `MissingSlot{slot}` | `CALYX_GUARD_MISSING_SLOT` | produced/matched vector absent. |
| `PolicyViolation{k,n_required}` | `CALYX_GUARD_POLICY_VIOLATION` | `KofN k > n_required`. |
| `NotAFailure{guard_id}` | `CALYX_GUARD_NOT_A_FAILURE` | novelty handling on a PASS. |
| `GuardIdMismatch{...}` | `CALYX_GUARD_ID_MISMATCH` | verdict/profile guard_id differ. |
| `IdentitySlotNotRequired{slot}` | `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED` | identity slot not in required set. |
| `NoveltySink{reason}` | `CALYX_GUARD_NOVELTY_SINK` | vault write failure. |
| `ModelNotFound{path}` | `CALYX_WARD_MODEL_NOT_FOUND` | lens model file missing. |
| `InvalidInput{reason}` | `CALYX_WARD_INVALID_INPUT` | bad lens input. |
| `ModelDimMismatch{expected,actual}` | `CALYX_WARD_MODEL_DIM_MISMATCH` | lens output dim wrong. |
| `Runtime{reason}` | `CALYX_WARD_RUNTIME_ERROR` | ONNX/runtime failure. |
| `MissingFrequency{cx_id,detail}` | `CALYX_WARD_MISSING_FREQUENCY` | recurrence base row/scalar missing. |
| `InvalidFrequency{cx_id,value}` | `CALYX_WARD_INVALID_FREQUENCY` | non-integer/negative frequency. |
| `InvalidDomain{reason}` | `CALYX_WARD_INVALID_DOMAIN` | bad surprise/domain math. |

---

## 10. Persistence / ledger (ledger.rs) — cross-ref Aster & Ledger

calyx-ward holds no on-disk format of its own; it persists through **calyx-ledger** (provenance) and **calyx-aster** (novelty/recurrence). See [14_ledger_provenance.md](14_ledger_provenance.md) and [06_aster_storage_engine.md](06_aster_storage_engine.md).

`WardLedgerError` (enum `Ward(WardError) | Ledger(CalyxError)`), `WardLedgerResult<T>`. Writers append `EntryKind::Guard` entries via `LedgerAppender` (actor `"calyx-ward"`):
- `append_calibration_provenance(appender, profile)`: requires a calibrated profile; payload tag `ward_calibration_v1` with guard_id, panel_version, policy, required_slots, tau, and full calibration meta (hex `corpus_hash`). Subject = `SubjectId::Guard(guard_id bytes)`.
- `append_guard_verdict(appender, cx_id, verdict)`: payload tag `ward_guard_verdict_v1` with overall_pass, provisional, action, per-slot breakdown. Subject = `SubjectId::Cx(cx_id)`.
- `calibrate_with_ledger`, `guard_with_ledger`: convenience wrappers that calibrate/guard then append.

`GuardProfile`, all verdict and calibration types, `NoveltyRecord`, and query verdicts derive `Serialize`/`Deserialize` (JSON) and round-trip (proptests in each module). Novelty records persist to an `AsterVault` via the `VaultSink` seam. Aster reads `FREQUENCY_SCALAR` and `read_series` for recurrence.

---

## 11. Divergences from the plan (`docs/dbprdplans/09_WARD_TCT_GUARD.md`)

- Plan and code agree on the core: per-slot cosine `Gτ`, `AllRequired`/`KofN`, `GuardProfile` shape, `NoveltyAction` triad, default `τ≈0.7` as a cold-start prior, conformal calibration with stored FAR/FRR/ts provenance.
- The plan lists a single calibration step; the code makes the conformal estimator concrete: estimator id `conformal_quantile_v1`, `MIN_BAD_SCORES=50`, per-`SlotKind` FAR ceilings (Identity 0.01 / Content 0.03 / Stylistic 0.05), and a binomial-confidence-bounded quantile (§3.4) rather than a plain quantile.
- The plan's `guard_health` summary shows scalar `{far,frr,drift,last_calibrated}`; the code returns per-slot maps plus `dropped_events` (drift telemetry detail).
- Drift→Anneal is wired through an in-process `AnnealHook` trait + bounded channel, explicitly "until Anneal's PH48 queue is live."

---

## Gaps / not covered

- `AnnealHook` is an interim object-safe seam (comment: "until Anneal's PH48 queue is live"); the live Anneal queue is not part of this crate.
- ONNX lens execution requires the real model files at the default paths and a CUDA-capable runtime (`CudaFailLoud` has no CPU fallback); only the trait-backend seam is exercised without models.
- Surprise/novelty-signal classification depends on Aster recurrence scalars; absent data fails closed rather than estimating.
- FSV fixture tests (`#[ignore]`, env-var gated, e.g. `CALYX_WARD_*_FSV_DIR`) write JSON artifacts and are not run by default.
- Polis civic guard is a synthetic-persona proof surface (sign-agreement over 21 axes), not a general civic-data pipeline.
