# 12. Ward Guard (calyx-ward)

The Ward guard ("TCT guard") enforces per-slot cosine-similarity policies over
Calyx panel slots. It is the fail-closed gate that decides whether a produced
embedding (a generated output, an incoming query, or an identity utterance) is
in-distribution against trusted, calibrated reference vectors. Every required
slot is scored independently — there is no aggregate/averaged/flattened vector
gate (`guard.rs` INVARIANT A3). Decisions are structured verdicts with the full
per-slot decomposition; non-pass outcomes are wrapped as out-of-distribution
(OOD) errors and may be routed into novelty handling and recorded in the
Ledger.

This document describes only what the source does. Where a behavior is not
present in source it is marked "Not determined from source".

Source files covered:

- `crates/calyx-ward/src/lib.rs` — module surface and public re-exports.
- `crates/calyx-ward/src/profile.rs` — `GuardProfile`, `GuardPolicy`, `GuardId`, `NoveltyAction`, `CalibrationMeta`, `SlotCalibrationMeta`.
- `crates/calyx-ward/src/verdict.rs` — `GuardVerdict`, `SlotVerdict`.
- `crates/calyx-ward/src/error.rs` — `WardError`, error code constants.
- `crates/calyx-ward/src/guard.rs` — core per-slot guard math, AllRequired/KofN evaluation, inert/high-stakes validation.
- `crates/calyx-ward/src/calibrate.rs` — conformal tau calibration.
- `crates/calyx-ward/src/required.rs` — Assay-bit derived required-slot selection.
- `crates/calyx-ward/src/query.rs` — `guard_query` / `guard_query_kernel_first` over trusted regions.
- `crates/calyx-ward/src/generate.rs` — `guard_generate` identity generation loop.
- `crates/calyx-ward/src/identity.rs` — `IdentityProfile`, `IdentitySlotConfig`.
- `crates/calyx-ward/src/speaker_lens.rs` — WavLM speaker embedding lens.
- `crates/calyx-ward/src/style_lens.rs` — RoBERTa/style embedding lens.
- `crates/calyx-ward/src/novelty.rs` — novelty routing, recurrence/surprise classification.
- `crates/calyx-ward/src/drift.rs` — rolling drift monitor, `GuardHealth`.
- `crates/calyx-ward/src/ledger.rs` — Ledger provenance writers.
- `crates/calyx-ward/src/polis.rs` — Polis civic-panel guard validation.
- `crates/calyx-ward/tests/ph38_injection_fsv.rs` + `ph38_injection_fsv/support.rs` — injection-corpus defense harness.

Cross-references: calibration corpus hashing and verdict provenance are appended
to the Ledger (see [11_ledger_provenance.md](11_ledger_provenance.md)). Required
slots are derived from Assay `bits_about` panels (see
[09_loom_assay_dda.md](09_loom_assay_dda.md)). Trusted regions for query gating
correspond to constellation regions returned by search (see
[08_sextant_search.md](08_sextant_search.md)). Cosine math (`dense_cosine`),
`Lens`, `SlotId`, `Clock`, and `Panel` come from core (see
[04_core_foundation.md](04_core_foundation.md)).

---

## 12.1 Guard profile and verdict model

### 12.1.1 GuardProfile

`GuardProfile` (`profile.rs`) is the configuration object every guard call
reads. It implements core's `GuardTauProfile` trait.

| Field | Type | Meaning |
|-------|------|---------|
| `guard_id` | `GuardId` (UUID newtype) | Stable identifier for the profile. |
| `panel_version` | `u64` | Panel version the profile was derived against. |
| `domain` | `String` | Domain label (used in novelty id hashing). |
| `tau` | `BTreeMap<SlotId, f32>` | Per-slot cosine threshold. `tau_for(slot)` returns `None` if a slot is not guarded. |
| `required_slots` | `Vec<SlotId>` | Slots that must be evaluated. |
| `policy` | `GuardPolicy` | `AllRequired` or `KofN { k }`. |
| `calibration` | `Option<CalibrationMeta>` | Calibration provenance; `is_calibrated()` is true iff present. |
| `novelty_action` | `NoveltyAction` | Action when a verdict fails. |

`tau_for(slot)` returns the stored tau or `None`. In `guard()`, a required slot
without a tau falls back to `DEFAULT_TAU = 0.7` (`guard.rs`).

### 12.1.2 SlotVerdict and GuardVerdict

`SlotVerdict` (`verdict.rs`): `{ slot: SlotId, cos: f32, tau: f32, pass: bool }`.

`GuardVerdict`:

| Field | Type | Meaning |
|-------|------|---------|
| `guard_id` | `GuardId` | Copied from the profile. |
| `overall_pass` | `bool` | Aggregate decision under the policy. |
| `provisional` | `bool` (`serde default`) | `true` when the profile is not calibrated (`!profile.is_calibrated()`). |
| `per_slot` | `Vec<SlotVerdict>` | Full per-slot decomposition, preserved on pass and fail. |
| `action` | `Option<NoveltyAction>` | `Some(novelty_action)` only when `!overall_pass`, else `None`. |

`failing_slots()` returns the `SlotVerdict`s with `pass == false`;
`all_slot_details()` returns the full slice.

### 12.1.3 Error model (WardError)

`WardError` (`error.rs`) is the fail-closed error enum. `code()` maps each
variant to a stable `CALYX_*` string. The same code can back multiple variants
(e.g. all calibration-shortfall variants map to `CALYX_GUARD_PROVISIONAL`).

| Variant | `code()` | Trigger |
|---------|----------|---------|
| `Ood { guard_id, failing }` | `CALYX_GUARD_OOD` | Verdict did not pass (wrapped by `guard_result*`). |
| `Provisional { guard_id }` | `CALYX_GUARD_PROVISIONAL` | High-stakes call on an uncalibrated profile. |
| `MissingSlotCalibration { guard_id, slot }` | `CALYX_GUARD_PROVISIONAL` | High-stakes required slot lacks tau or per-slot calibration. |
| `InsufficientCalibrationData { n, min }` | `CALYX_GUARD_PROVISIONAL` | `bad_scores.len() < MIN_BAD_SCORES` (50). |
| `InvalidCalibrationInput { reason }` | `CALYX_GUARD_PROVISIONAL` | Bad alpha/target_far/scores or empty inputs. |
| `InvalidRequiredSlotDerivation { reason }` | `CALYX_GUARD_PROVISIONAL` | Bad config or no load-bearing slots; identity-slot misconfig. |
| `InertProfile { guard_id, reason }` | `CALYX_GUARD_INERT_PROFILE` | Empty required slots, or `KofN { k: 0 }`. |
| `MissingSlot { slot }` | `CALYX_GUARD_MISSING_SLOT` | Required slot absent from produced or matched vectors. |
| `PolicyViolation { k, n_required }` | `CALYX_GUARD_POLICY_VIOLATION` | `KofN` with `k > n_required`. |
| `NotAFailure { guard_id }` | `CALYX_GUARD_NOT_A_FAILURE` | Novelty handling called on a passing verdict. |
| `GuardIdMismatch { profile_guard_id, verdict_guard_id }` | `CALYX_GUARD_ID_MISMATCH` | Verdict guard id ≠ profile guard id in novelty handler. |
| `IdentitySlotNotRequired { slot }` | `CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED` | Identity slot not in `required_slots`. |
| `NoveltySink { reason }` | `CALYX_GUARD_NOVELTY_SINK` | Novelty vault write failure. |
| `ModelNotFound { path }` | `CALYX_WARD_MODEL_NOT_FOUND` | Lens model/tokenizer path missing. |
| `InvalidInput { reason }` | `CALYX_WARD_INVALID_INPUT` | Bad lens input (empty audio/text, NaN, wrong modality). |
| `ModelDimMismatch { expected, actual }` | `CALYX_WARD_MODEL_DIM_MISMATCH` | Lens output dim ≠ expected. |
| `Runtime { reason }` | `CALYX_WARD_RUNTIME_ERROR` | ORT/session/IO runtime errors. |
| `MissingFrequency { cx_id, detail }` | `CALYX_WARD_MISSING_FREQUENCY` | Recurrence base row/scalar missing (fails closed). |
| `InvalidFrequency { cx_id, value }` | `CALYX_WARD_INVALID_FREQUENCY` | `frequency` scalar not a non-negative integer. |
| `InvalidDomain { reason }` | `CALYX_WARD_INVALID_DOMAIN` | Surprise score / domain overflow. |

`Display` for each variant prefixes the code; `Ood` appends `slot/cos/tau` for
every failing slot.

---

## 12.2 Policies: AllRequired and KofN

`GuardPolicy` (`profile.rs`):

| Policy | Pass condition |
|--------|----------------|
| `AllRequired` | `pass_count == per_slot.len()` (every required slot meets its tau). |
| `KofN { k }` | `pass_count >= k` (at least k required slots pass). |

### 12.2.1 `guard()` evaluation steps (`guard.rs::guard`)

1. Build `required` = sorted, deduped copy of `profile.required_slots`.
2. `validate_non_inert_required`: reject `InertProfile` if `required` is empty;
   reject `InertProfile{reason:"kofn_zero"}` if `KofN { k: 0 }`; reject
   `PolicyViolation` if `KofN` `k > required.len()`.
3. If `high_stakes`, run `validate_high_stakes_profile` (see 12.2.2).
4. For each required slot:
   - Fetch produced and matched vectors; missing → `MissingSlot` (fail closed).
   - `tau = profile.tau_for(slot).unwrap_or(DEFAULT_TAU)`.
   - Compute `dense_cosine(produced, matched)`. `Some(cos)` → `pass = cos >= tau`.
     `None` (zero-norm or shape mismatch) → `cos = 0.0, pass = false` (no panic).
   - Push `SlotVerdict`.
5. `pass_count` = number of passing slots. Apply policy to set `overall_pass`.
6. `action = (!overall_pass).then(|| profile.novelty_action.clone())`.
7. `provisional = !profile.is_calibrated()`.

Boundary rule: a slot passes when `cos >= tau` (equality passes).

### 12.2.2 High-stakes validation (`validate_high_stakes_profile`)

For high-stakes calls the profile must carry calibration provenance, and every
required slot must have both a tau entry and a per-slot calibration entry:

- No `profile.calibration` → `Provisional`.
- For any required slot with `tau_for(slot).is_none()` OR
  `!calibration.per_slot.contains_key(slot)` → `MissingSlotCalibration`.

### 12.2.3 Guard wrappers

| Function | Behavior |
|----------|----------|
| `guard(profile, produced, matched, high_stakes)` | Core evaluator; returns `GuardVerdict` or `WardError`. |
| `guard_non_high_stakes` | `guard(..., false)`. |
| `guard_result` | `guard_result_with_stakes(..., false)`. |
| `guard_result_with_stakes` | Runs `guard`; if `!overall_pass` returns `WardError::Ood { failing }`, else the verdict. |
| `validate_non_inert_profile` | Standalone inert check without scoring. |

`MatchedSlots` and `ProducedSlots` are both `BTreeMap<SlotId, Vec<f32>>`.

---

## 12.3 Conformal tau calibration (`calibrate.rs`)

Calibration computes a per-slot tau from a corpus of known-good and known-bad
cosine scores so that the achieved false-accept rate (FAR) stays at or below a
target with a confidence guarantee.

Constants:

| Constant | Value | Role |
|----------|-------|------|
| `TAU_COLD_START` | `DEFAULT_TAU` (0.7) | Cold-start tau before calibration. |
| `MIN_BAD_SCORES` | `50` | Minimum known-bad scores required. |
| `ESTIMATOR` | `"conformal_quantile_v1"` | Estimator label stored in provenance. |

`SlotKind` and default FAR ceilings (`default_target_far`):

| `SlotKind` | Default target FAR |
|------------|--------------------|
| `Identity` | 0.01 |
| `Stylistic` | 0.05 |
| `Content` | 0.03 |

`CalibrationInput`: `{ slot, good_scores, bad_scores, slot_kind, target_far }`.

### 12.3.1 `calibrate_slot(input, alpha, clock)` steps

1. `validate_input`: `alpha` finite in `[0,1]`; `target_far` finite in `[0,1]`;
   `target_far <= slot_kind.default_target_far()` else `InvalidCalibrationInput`.
2. If `bad_scores.len() < MIN_BAD_SCORES` → `InsufficientCalibrationData`.
3. Sort good and bad scores (`sorted_scores` rejects non-finite or out-of-`[-1,1]`).
4. `tau = conformal_tau(sorted_bad, target_far, alpha)` (see 12.3.2).
5. Achieved `far` = fraction of bad scores `>= tau`.
6. Achieved `frr` = fraction of good scores `< tau` (0.0 if no good scores).
7. `corpus_hash` = SHA-256 over `(slot, slot_kind, target_far, alpha, good_scores, bad_scores)`.
8. Return `(tau, CalibrationMeta::new(corpus_hash, ESTIMATOR, far, frr, 1.0 - alpha, clock))`.
   `confidence = 1.0 - alpha`; `ts` from injected `Clock`.

### 12.3.2 `conformal_tau` (threshold formula)

```rust
// target_far == 0.0  -> tau just above the largest bad score (admits nothing)
if target_far == 0.0 { return next_above(max(bad_scores)); }

// Otherwise scan candidate thresholds (each bad score and the float just above it),
// sorted ascending, and pick the FIRST candidate that meets BOTH:
let candidate_far = (#bad_scores >= candidate) / bad_count;
candidate_far <= target_far + EPSILON
    && binomial_cdf_at_most(bad_accepts, bad_count, target_far) <= alpha + EPSILON
// If none qualify, fall back to next_above(max(bad_scores)).
```

The first clause is the empirical FAR bound; the second
(`confidence_bound_satisfied`) is a one-sided binomial confidence test:
`binomial_cdf_at_most(successes, trials, p)` is the CDF
`P(X <= successes)` for `Binomial(trials, target_far)`, and must be `<= alpha`.
`next_above` returns the next representable f32 above a value (handling 0 and
sign), so the chosen tau strictly excludes the bad score it sits above.

### 12.3.3 `calibrate(profile_template, inputs, alpha, clock)` (whole profile)

1. Empty inputs → `InvalidCalibrationInput`.
2. For each input, run `calibrate_slot`; write the tau into `profile.tau`; add the
   slot to `required_slots` if absent.
3. Sort/dedup `required_slots`.
4. `merge_meta`: profile-level `CalibrationMeta` whose `corpus_hash` is SHA-256
   over each `(slot, slot.corpus_hash)`, `far`/`frr` are the **max** across
   slots, and `per_slot` holds each slot's `SlotCalibrationMeta`.

`CalibrationMeta` fields: `corpus_hash [u8;32]`, `estimator`, `far`, `frr`,
`confidence`, `ts`, `per_slot: BTreeMap<SlotId, SlotCalibrationMeta>`.

### 12.3.4 Required-slot derivation (`required.rs`)

`LOAD_BEARING_MIN_BITS = 0.05`. `derive_required_slots(panel, config)` reads
Assay `Slot.bits_about[anchor]` for `SlotState::Active` slots and selects slots
whose `bits >= min_bits` (non-finite bits → `InvalidRequiredSlotDerivation`).
`RequiredSlotDerivation::assay_bits(anchor)` uses the 0.05-bit threshold and
cold-start tau `DEFAULT_TAU`; `::manual(anchor, slots)` supplies an explicit set.
`derive_required_profile` applies the derived/explicit slots, inserts
`cold_start_tau` for any slot missing a tau (preserving existing calibrated tau),
sets `required_slots`, and records `panel_version`. Empty manual sets or no
load-bearing slots → `InvalidRequiredSlotDerivation`.

---

## 12.4 OOD wrapper and novelty routing

### 12.4.1 OOD

There is no separate "OOD score". A verdict is OOD iff `overall_pass == false`.
`guard_result_with_stakes` converts a non-passing verdict into
`WardError::Ood { guard_id, failing }` where `failing` is the failing
`SlotVerdict`s. In query gating (`query.rs`), the OOD `gap` is
`(-best_margin).max(0.0)`, where margin is the minimum `cos - tau` across slots.

### 12.4.2 Novelty actions

`NoveltyAction` (`profile.rs`) and the durable `NoveltyStatus` it maps to in
`NoveltyHandler::handle`:

| `NoveltyAction` | `NoveltyStatus` | Post-write behavior |
|-----------------|-----------------|---------------------|
| `NewRegion` | `AwaitingGrounding` | Returns `Ok(record)`. |
| `Quarantine` | `Quarantined` | Returns `Ok(record)`. |
| `RejectClosed` | `Rejected` | Writes record, then returns `Err(WardError::Ood)`. |

### 12.4.3 `NoveltyHandler::handle` steps (`novelty.rs`)

1. `verdict.guard_id != profile.guard_id` → `GuardIdMismatch`.
2. `verdict.overall_pass` → `NotAFailure` (novelty requires a failed verdict).
3. Map `novelty_action` to `NoveltyStatus`.
4. Collect failing `SlotVerdict`s; `ts` from injected clock.
5. `novel_id` = UUID derived by SHA-256 over guard id, panel version, domain,
   ts, produced slot vectors (bit patterns), and per-slot verdict fields, with
   UUIDv4 variant/version bits set.
6. `vault.write_novel(record)` (vault errors surface as `WardError`).
7. If `RejectClosed`, return `Err(Ood)`; else return `Ok(record)`.

`VaultSink` is the storage seam: `write_novel(&NoveltyRecord)` and
`novel_records()`. `novel_regions(vault, since_ts)` lists `AwaitingGrounding`
records with `ts >= since_ts`.

### 12.4.4 Recurrence/surprise novelty classification

`classify_novelty(cx_id, vault, clock)` reads the recurrence series from the
Aster vault (`FREQUENCY_SCALAR`; missing/invalid frequency fails closed with
`MissingFrequency` / `InvalidFrequency`):

- `frequency <= 1` → `NonRecurring`.
- `frequency >= 3` and finite positive cadence and the latest occurrence is
  older than `last_t + 2*cadence_secs` → `OverdueRecurrence { expected_t, overdue_by_secs }`.
- Otherwise → `Recurring { frequency, cadence_secs }`.

`surprise_score_from_counts(frequency, total)` is the per-event surprise in bits:

```rust
let p = (frequency.max(1) / total).clamp(MIN_POSITIVE, 1.0);
SurpriseScore = -p.ln() / 2.0_f32.ln();   // -log2(p)
```

The source comments this is a retrieval-anomaly score only and MUST NOT modify
stored bits. `surprise_bits(cx_id, domain, vault)` divides the cx frequency by
the summed domain frequency (overflow → `InvalidDomain`).

`novelty_action_for_signal`: `Recurring` → `None`;
`NonRecurring`/`OverdueRecurrence` → `NewRegion`; `Anomaly` → `Quarantine`.
`overdue_recurrence_scan` returns the overdue cx ids in a domain.

---

## 12.5 Drift monitoring (`drift.rs`)

`DriftMonitor` tracks rolling per-slot rejection rates for one calibrated guard
and notifies an `AnnealHook` when a slot drifts.

Constants:

| Constant | Value |
|----------|-------|
| `DEFAULT_DRIFT_WINDOW` | 500 |
| `DEFAULT_DRIFT_CHANNEL_CAPACITY` | 32 |
| `REJECTION_RATE_DRIFT_MULTIPLIER` | 1.5 |

### 12.5.1 Construction

`DriftMonitor::new(profile, window_size, anneal_hook)` (or
`with_channel_capacity`) extracts per-slot calibrated FAR bounds and FRR from
the profile (`calibration_maps`): per-slot values come from
`calibration.per_slot[slot]`, falling back to the profile-level `far`/`frr` for
slots without per-slot meta; `last_calibrated` = `calibration.ts` (0 if
uncalibrated). A bounded `sync_channel` and a worker thread forward drift events
to the hook off the hot path; `Drop` closes the channel and joins the worker.

### 12.5.2 `record_verdict` / `check_slot` steps

1. Ignore verdicts whose `guard_id` differs.
2. For each `SlotVerdict`, push `pass` into the slot's `VecDeque`, trimming to
   `window_size` (min 1).
3. `check_slot`: skip slots without a calibrated FAR bound. Compute
   `current_rejection_rate` = fraction of `false` (rejected) in the window.
   `drift = current_rejection_rate > calibrated_far_bound * 1.5`.
4. On drift: add slot to `drift_slots`; if not already notified, `try_send` a
   `DriftEvent`. Successful send marks the slot notified; a full or disconnected
   channel increments `dropped_events` (non-blocking). On non-drift: clear the
   slot from `drift_slots` and `notified_drift_slots`.

### 12.5.3 GuardHealth

`guard_health(monitor, guard_id)` returns the live snapshot for the matching
guard, else a zeroed `GuardHealth`.

| `GuardHealth` field | Meaning |
|---------------------|---------|
| `guard_id` | Guard queried. |
| `per_slot_rejection_rate` | Rolling rejection fraction per slot. |
| `per_slot_calibrated_far_bound` | Calibrated FAR bound per slot. |
| `per_slot_frr` | Calibrated FRR per slot. |
| `drift` | True if any slot is currently drifting. |
| `last_calibrated` | Calibration timestamp. |
| `dropped_events` | Drift events dropped due to a full/closed channel. |

`AnnealHook` is the object-safe seam (documented as a stand-in until Anneal's
PH48 queue is live).

---

## 12.6 Injection-corpus defenses

The injection-corpus defense is built from the standard calibration + guard
pipeline applied to a labelled injection corpus; there is no dedicated
production module. The FSV harness lives in
`tests/ph38_injection_fsv.rs` and `ph38_injection_fsv/support.rs`.

Mechanism: a benign centroid is computed from `label == 0` (benign) calibration
rows. Cosine scores of benign rows become `good_scores`; injection rows
(`label == 1`) become `bad_scores`. `calibrate` produces a content-slot tau at
`TARGET_FAR = 0.01`, `ALPHA = 0.05`. Each held-out injection vector is then
guarded against the centroid; a non-passing verdict counts as "blocked". The
harness asserts the held-out block rate `>= REQUIRED_BLOCK_RATE = 0.99`.

Harness constants (`support.rs`):

| Constant | Value |
|----------|-------|
| `TARGET_FAR` | 0.01 |
| `REQUIRED_BLOCK_RATE` | 0.99 |
| `ALPHA` | 0.05 |
| `NOVELTY_COS` | 0.30 |
| `CONTENT_SLOT` | `SlotId(1)` |

Corpus error codes (harness-local): `CALYX_WARD_MISSING_INJECTION_CORPUS`,
`CALYX_WARD_MISSING_INJECTION_VECTORS`, `CALYX_WARD_INVALID_INJECTION_CORPUS`,
`CALYX_WARD_INJECTION_CORPUS_IO`, `CALYX_WARD_INJECTION_CORPUS_JSON`. Blocked
injections route through novelty (`AwaitingGrounding`) via the `FileVault` sink.

---

## 12.7 Identity profiles and lenses

### 12.7.1 IdentityProfile (`identity.rs`)

`IdentitySlotConfig`: `{ slot_id, anchor_kind, tau_override: Option<f32> }`.
`is_identity_anchor()` is true for `AnchorKind::SpeakerMatch` or
`AnchorKind::StyleHold`.

`IdentityProfile` wraps a `GuardProfile`, the identity slot configs, and a
`matched_slot_cache` of unit-normalized reference vectors. `IdentityProfile::new`
validates (also enforced on `Deserialize`):

1. Each identity slot must be in `guard_profile.required_slots` else
   `IdentitySlotNotRequired`.
2. No duplicate identity slots (`InvalidRequiredSlotDerivation`).
3. Each anchor must be `SpeakerMatch` or `StyleHold`.
4. `tau_override` (if present) is validated and written into `guard_profile.tau`;
   otherwise the slot must already have a valid tau in `[0,1]`.
5. The matched vector must exist (`MissingSlot`) and is unit-normalized
   (`normalize_matched`; zero-norm/non-finite → `InvalidCalibrationInput`).
6. Every required slot must be covered by an identity slot.

### 12.7.2 Speaker lens (`speaker_lens.rs`)

Frozen WavLM speaker adapter implementing core's `Lens`.

| Constant | Value |
|----------|-------|
| `DEFAULT_WAVLM_MODEL_PATH` | `/home/croyse/calyx/models/wavlm/wavlm-base-plus-sv.onnx` |
| `WAVLM_SAMPLE_RATE` | 16000 |
| `WAVLM_DIM` | 512 |
| source repo / revision | `Xenova/wavlm-base-plus-sv` @ `e610296…` |

`embed_speaker`: validate audio (non-empty, non-zero sample rate, no NaN/Inf),
trim edge silence (`abs > 1e-6`), linear-resample to 16 kHz if needed, run the
ONNX backend, then unit-normalize (dim must equal 512). `SpeakerProviderPolicy`:
`CudaFailLoud` (CUDA device 0, `error_on_failure`, no CPU fallback) or
`CpuExplicit`. `lens_id` = `LensId::from_parts(name, model_sha256, corpus_hash,
"dense:f32:audio:speaker:512")`. `SpeakerEmbeddingBackend` is the test seam.

### 12.7.3 Style lens (`style_lens.rs`)

Frozen RoBERTa-style adapter implementing `Lens`.

| Constant | Value |
|----------|-------|
| `DEFAULT_STYLE_MODEL_PATH` | `/home/croyse/calyx/models/style/style-embed-v1.onnx` |
| `DEFAULT_STYLE_TOKENIZER_PATH` | `/home/croyse/calyx/models/style/tokenizer.json` |
| `STYLE_DIM` | 768 |
| `STYLE_MAX_TOKENS` | 512 |
| source repo / revision | `AnnaWegmann/Style-Embedding` @ `d7d0f5c…` |

`embed_style`: reject empty/whitespace text, tokenize (truncate to 512 tokens),
run ONNX, then attention-mask **mean-pool** token embeddings (`mean_pool`) and
unit-normalize (dim must equal 768). Same provider policies and lens-id scheme;
output shape tag `dense:f32:text:style:768`. Both lenses load the model with
`GraphOptimizationLevel::Level3` and hash model (and tokenizer) files into the
lens id for provenance.

---

## 12.8 guard_query and guard_generate flows

### 12.8.1 `guard_query` / `guard_query_kernel_first` (`query.rs`)

A `TrustedRegion` is `{ cx_id, slots: MatchedSlots }` — a trusted constellation
region the query is tested against. `guard_query` delegates to
`guard_query_kernel_first(profile, query_slots, &[], trusted_regions)`.

`guard_query_kernel_first(profile, query_slots, kernel_regions, peripheral_regions)`:

1. `validate_non_inert_profile`.
2. `evaluate_regions` over kernel regions: for each region run
   `guard_non_high_stakes(profile, query_slots, region.slots)`; the candidate's
   `margin` = minimum `cos - tau` across slots; keep best passing and best OOD by
   highest margin. If any kernel region passes, return `Pass` with
   `match_source = KernelNear`, `gap = 0.0`.
3. Otherwise evaluate peripheral regions; a pass returns
   `match_source = Peripheral`, `gap = 0.0`.
4. If neither passes, return `Ood` with the best OOD candidate across both tiers
   (`nearest_cx`, `match_source`, `gap = (-margin).max(0.0)`, `per_slot`) and
   `action = profile.novelty_action`. With no regions, `Ood` with `None` fields.

`QueryVerdict` is the kernel-agnostic projection (`Pass` / `Ood`);
`KernelFirstQueryVerdict` additionally records `match_source: RegionSource`
(`KernelNear` / `Peripheral`). Query gating uses non-high-stakes guarding.

### 12.8.2 `guard_generate` (`generate.rs`)

Identity-locked generation loop (PH39). `GenerateInput`:
`{ candidate_audio: Option<Vec<f32>>, candidate_text: Option<String>,
sample_rate, matched_cx_id }`.

`guard_generate(identity_profile, input, speaker_lens, style_lens,
novelty_handler, high_stakes)` steps:

1. `reject_inert_identity_profile`: must have ≥1 identity slot and a non-inert
   guard profile.
2. If `high_stakes && !identity_profile.is_calibrated()` → `Provisional`.
3. `produced_slots`: for each identity slot, measure the candidate through the
   matching lens — `SpeakerMatch` ⇒ speaker lens over audio (bytes prepared at
   16 kHz f32 LE; resampled if needed); `StyleHold` ⇒ style lens over text. Other
   anchors → `InvalidRequiredSlotDerivation`. Lenses must return dense vectors.
4. `guard(guard_profile, produced, matched_slot_cache, high_stakes)`.
5. `route_verdict`:
   - Pass → `Accepted { verdict, provenance_tag: "guarded:pass", ledger_ref:None }`.
   - Fail → `novelty_handler.handle(...)`:
     - `Ok(record)` → `Novel { record }`.
     - `Err(Ood)` when `novelty_action == RejectClosed` →
       `Rejected { provenance_tag: "guarded:reject:unprovenanced" }`.
     - other `Err` propagates.

`GenerateOutput` variants: `Accepted`, `Novel`, `Rejected`. Provenance tags:
`GUARDED_PASS_TAG="guarded:pass"`, `GUARDED_REJECT_TAG="guarded:reject"`,
`GUARDED_REJECT_UNPROVENANCED_TAG="guarded:reject:unprovenanced"`.

`guard_generate_with_ledger` wraps the above and appends the verdict to the
Ledger for `Accepted`/`Rejected` outcomes (re-tagging rejects as
`guarded:reject`), attaching the resulting `LedgerRef`.

---

## 12.9 Ledger provenance integration (`ledger.rs`)

Ward writes two record kinds to the Ledger via a `LedgerAppender<S: LedgerCfStore, C: Clock>`.
Actor is `"calyx-ward"` (`ActorId::Service`). See
[11_ledger_provenance.md](11_ledger_provenance.md).

| Writer | EntryKind | SubjectId | Provenance tag |
|--------|-----------|-----------|----------------|
| `append_calibration_provenance` | `Guard` | `Guard(guard_id bytes)` | `ward_calibration_v1` |
| `append_guard_verdict` | `Guard` | `Cx(cx_id)` | `ward_guard_verdict_v1` |

- `calibrate_with_ledger` runs `calibrate` then appends calibration provenance,
  returning `(GuardProfile, LedgerRef)`. Appending an uncalibrated profile
  errors with `ledger_corrupt`.
- `guard_with_ledger` runs `guard` then appends the verdict, returning
  `(GuardVerdict, LedgerRef)`.
- The calibration payload encodes guard id, panel version, policy
  (`all_required` / `k_of_n`), required slots, tau map, and full
  `CalibrationMeta` (corpus hashes hex-encoded, per-slot meta). The verdict
  payload encodes cx id, guard id, `overall_pass`, `provisional`, action
  (`new_region`/`quarantine`/`reject_closed`), and per-slot `{slot,cos,tau,pass}`.

`WardLedgerError` wraps `WardError` or `CalyxError`; `WardLedgerResult<T>` is the
combined result type.

---

## 12.10 Polis civic-panel guard (`polis.rs`)

A deterministic validation surface that guards synthetic civic personas
slot-by-slot (no averaging across axes).

| Constant | Value |
|----------|-------|
| `CIVIC_SLOT_COUNT` | 21 |
| `CIVIC_TAU` | 0.7 |
| temporal slots excluded | 22, 23, 24 |

`evaluate_polis_civic_pairs(pairs)`: validates each persona has exactly 21
finite, non-zero axes (else `SlotCountMismatch` / `InvalidAxis`), builds a
calibrated `AllRequired` profile (per-slot tau 0.7, estimator
`polis-synthetic-sign`), and guards each pair's right persona against its left
as the matched reference at `high_stakes = true`. A pair "ties" when the verdict
passes; mismatch with the planted tie → `TieMismatch`. Single-axis cosine over
sign-valued axes makes each slot pass iff the two personas agree on that axis's
sign. Error codes: `CALYX_POLIS_EMPTY_PERSONA_SET`,
`CALYX_POLIS_SLOT_COUNT_MISMATCH`, `CALYX_POLIS_INVALID_AXIS`,
`CALYX_POLIS_TIE_MISMATCH` (plus wrapped `WardError`).

---

## 12.11 Constants and thresholds (summary)

| Constant | Value | File |
|----------|-------|------|
| `DEFAULT_TAU` | 0.7 | `guard.rs` |
| `TAU_COLD_START` | 0.7 | `calibrate.rs` |
| `MIN_BAD_SCORES` | 50 | `calibrate.rs` |
| `ESTIMATOR` | `conformal_quantile_v1` | `calibrate.rs` |
| `SlotKind::Identity/Stylistic/Content` target FAR | 0.01 / 0.05 / 0.03 | `calibrate.rs` |
| `LOAD_BEARING_MIN_BITS` | 0.05 | `required.rs` |
| `DEFAULT_DRIFT_WINDOW` | 500 | `drift.rs` |
| `DEFAULT_DRIFT_CHANNEL_CAPACITY` | 32 | `drift.rs` |
| `REJECTION_RATE_DRIFT_MULTIPLIER` | 1.5 | `drift.rs` |
| `WAVLM_SAMPLE_RATE` / `WAVLM_DIM` | 16000 / 512 | `speaker_lens.rs` |
| `STYLE_DIM` / `STYLE_MAX_TOKENS` | 768 / 512 | `style_lens.rs` |
| `CIVIC_SLOT_COUNT` / `CIVIC_TAU` | 21 / 0.7 | `polis.rs` |
| `TARGET_FAR` / `REQUIRED_BLOCK_RATE` / `ALPHA` | 0.01 / 0.99 / 0.05 | injection FSV harness |

### Notes / undeterminable

- "No-average / no-flatten" is an enforced design invariant (`guard.rs` INVARIANT
  A3: per-slot independent scoring; `polis.rs` guards per-axis). There is no code
  path that averages or flattens slot vectors before thresholding.
- Provisional high-stakes refusal is enforced by `validate_high_stakes_profile`
  and the `guard_generate` calibration check (`CALYX_GUARD_PROVISIONAL`).
- The `Anomaly` novelty signal variant exists in `NoveltySignal` but is not
  produced by `classify_novelty` in source; its construction site is Not
  determined from source.
