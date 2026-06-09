# Stage 8 — Ward Gτ Guard (PH37–PH39)

**Status:** active. Tracked by Stage 8 epic #257 and exit issue #280; PH37-PH39
atomic tasks are #258-#279. PH37 T01-T09 (#258-#263, #275, #277, #278) and
PH38 T01-T06 (#264-#268, #276) are FSV-signed-off. PH37 is complete; PH38
post-T06 hardening #355/#356, Ledger guard provenance #279, and PH39 remain
before the Ward exit can claim Stage 8 complete. #357 timestamp unit hardening,
#351 drift metric semantics hardening, #352 held-out injection split hardening,
#354 per-slot calibration health hardening, and #358 GuardHealth serde
compatibility hardening are signed off. #355 drift notification retry hardening
is signed off.

Teleological Constellation Training at query/write time: the panel is a frozen
alignment target and every model-produced vector is gated by a per-output cosine
guard `Gτ`. Stops drift + prompt injection; turns novelty into a new safe region.
Lands in `calyx-ward`. **Living-system role:** immune system / self-vs-non-self.

---

## PH37 — Gτ guard math + GuardProfile
- **Objective.** Per-slot cosine gate with all-required (or KofN) pass logic;
  no-flatten enforced.
- **Deps.** PH22 (slots/lenses), PH13 (cosine).
- **Post-sweep note.** PH37 T01 (#258) adds the canonical profile/config types:
  `GuardId`, `GuardPolicy`, `NoveltyAction`, `CalibrationMeta`, and
  `GuardProfile`, with deterministic serde round-trip tests and aiwonder JSON
  readback evidence under
  `/home/croyse/calyx/data/fsv-issue258-ph37-t01-20260609-tsus`.
- **Post-sweep note.** PH37 T02 (#259) adds `SlotVerdict`, `GuardVerdict`, and
  `WardError` with durable aiwonder JSON/log readback evidence under
  `/home/croyse/calyx/data/fsv-issue259-ph37-t02-20260609`.
- **Post-sweep note.** PH37 T03 (#260) adds the `AllRequired` guard in
  `calyx-ward::guard`, with durable aiwonder readback evidence under
  `/home/croyse/calyx/data/fsv-issue260-ph37-t03-20260609-20a2a34`.
- **Post-sweep note.** PH37 T04 (#261) adds `KofN` policy and
  `guard_result()` OOD wrapping, with durable aiwonder readback evidence under
  `/home/croyse/calyx/data/fsv-issue261-ph37-t04-20260609-bd35e1e`.
- **Post-sweep note.** PH37 T05 (#262) adds no-average/no-flatten source
  enforcement and the average-pass/slot-fail rejection proof, with durable
  aiwonder readback evidence under
  `/home/croyse/calyx/data/fsv-issue262-ph37-t05-20260609-3dbe1a6`.
- **Post-sweep note.** PH37 T06 (#263) adds the PH37 readback harness for
  per-slot verdict JSON, average-pass rejection, OOD emission, source-marker
  smoke, profile roundtrip, and invalid-vector fail-closed evidence under
  `/home/croyse/calyx/data/fsv-issue263-ph37-t06-20260609-4cde3b7`.
- **Post-sweep note.** PH37 T07 (#275) adds the storage-agnostic
  `guard_query(profile, query_slots, trusted_regions)` incoming-query OOD gate,
  with durable aiwonder evidence under
  `/home/croyse/calyx/data/fsv-issue275-ph37-t07-20260609-8b71024`.
- **Post-sweep note.** PH37 T08 (#277) adds storage-agnostic required-slot
  derivation from `Panel.slots[*].bits_about[anchor]` using the inclusive
  0.05-bit load-bearing threshold, with explicit manual override and durable
  aiwonder evidence under
  `/home/croyse/calyx/data/fsv-issue277-ph37-t08-20260609-e75ade1`.
- **Post-sweep note.** PH37 T09 (#278) adds kernel-first query guarding:
  Lodestar `kernel_search` can feed kernel-near regions, Ward evaluates those
  before peripheral regions, and the source-marked verdict records
  `kernel_near` or `peripheral`. Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue278-ph37-t09-20260609-c2d3e30`.
- **Deliverables.** `guard.rs` (`cos(produced_k, matched_k) ≥ τ_k`),
  `GuardProfile { tau: Map<SlotId,f32>, required_slots, policy, calibration,
  novelty_action }`, per-slot verdict breakdown.
- **Key tasks.** require **every** required slot to pass (no flattened vector,
  A3); `CALYX_GUARD_OOD` on fail; verdict carries per-slot `(cos,tau,pass)`.
- **FSV gate.** an output passing the average but failing one required slot is
  **rejected**; read durable per-slot verdict JSON and source-readback artifacts
  from aiwonder. No concatenated-slot path is allowed.
- **Axioms/PRD.** A12, A3, `09 §1/§2/§4`.

## PH38 — τ calibration (conformal) + novelty→new-region
- **Objective.** Calibrate `τ` per slot against grounded outcomes with a bounded
  false-accept rate; a FAIL opens a new region, not a silent accept.
- **Deps.** PH37, PH28 (grounded outcomes).
- **Post-sweep note.** PH38 T01 (#264) adds `calyx-ward::calibrate` and
  `calibrate_slot`, slot-kind FAR caps, quantile-tie handling that matches
  Ward's `cos >= tau` predicate, and aiwonder readback evidence under
  `/home/croyse/calyx/data/fsv-issue264-ph38-t01-20260609-f95c817`.
- **Post-sweep note.** PH38 T02 (#265) adds the `high_stakes` guard parameter,
  `GuardVerdict.provisional`, `guard_non_high_stakes`, and fail-closed
  `CALYX_GUARD_PROVISIONAL` refusal for uncalibrated high-stakes calls.
  Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue265-ph38-t02-20260609-5c23db5`.
- **Post-sweep note.** PH38 T03 (#266) adds `NoveltyHandler`, `NovelId`,
  `NoveltyRecord`, `NoveltyStatus`, the object-safe `VaultSink`, and
  `novel_regions()` routing for `NewRegion`, `Quarantine`, and `RejectClosed`.
  Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue266-ph38-t03-20260609-fa0c263`.
- **Post-sweep hardening.** #350 makes `NoveltyHandler` fail closed with
  `CALYX_GUARD_ID_MISMATCH` when `GuardProfile.guard_id` and
  `GuardVerdict.guard_id` differ, before any sink write. #353 re-exports the
  novelty error constants from the `calyx-ward` crate root. Durable aiwonder
  evidence:
  `/home/croyse/calyx/data/fsv-issue350-ph38-guard-id-mismatch-20260609-a1fca2f`.
- **Post-sweep note.** PH38 T04 (#267) adds `DriftMonitor`, `AnnealHook`,
  bounded non-blocking drift events, `guard_health()`, and recovery/unknown-guard
  health snapshots. Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue267-ph38-t04-20260609-912b707`.
- **Post-sweep note.** PH38 T05 (#268) adds the real injection-corpus FSV gate:
  `/home/croyse/calyx/data/injection_corpus` is pinned from
  `deepset/prompt-injections`, embedded through resident TEI, calibrated with
  `calyx-ward::calibrate`, and then run through `guard()`. Durable aiwonder
  evidence: `/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`
  proves `block_rate=0.99239546`, `estimator=conformal_quantile_v1`, and
  valid novelty -> `AwaitingGrounding`.
- **Post-sweep note.** PH38 T06 (#276) adds Sextant
  `QueryGuard::InRegionOnly(GuardProfile)`: candidate hits are filtered through
  Ward, surviving hits carry `GuardVerdict`, and dropped OOD/missing-doc hits
  are recorded in the guarded-search report/explain payload. Durable aiwonder
  evidence:
  `/home/croyse/calyx/data/fsv-issue276-ph38-t06-20260609-c0b5d7f`.
- **Post-sweep hardening.** #357 normalizes Ward calibration, novelty, and
  `guard_health.last_calibrated` timestamps to Unix milliseconds before Ledger
  guard provenance lands. Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue357-ph38-timestamp-units-20260609-6e3ff73`.
- **Post-sweep hardening.** #351 renames runtime drift health/event surfaces to
  rejection/OOD rate while preserving the calibrated FAR bound as the comparison
  threshold. Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue351-ph38-rejection-rate-20260609-c6a2ccc`.
- **Post-sweep hardening.** #352 makes the PH38 T05 injection FSV calibrate on
  the corpus `train` split and report held-out `test` injection block rate
  separately from calibration FAR and whole-corpus block rate. Durable aiwonder
  evidence:
  `/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995`.
- **Post-sweep hardening.** #354 preserves distinct per-slot calibration FAR/FRR
  metadata in `CalibrationMeta.per_slot`, exposes per-slot calibrated FAR bounds
  through `GuardHealth.per_slot_calibrated_far_bound`, and makes drift monitoring
  compare each slot against its own calibrated bound. Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue354-ph38-per-slot-calibration-20260609-f672547`.
- **Post-sweep hardening.** #358 adds serde-default compatibility for
  `GuardHealth.per_slot_calibrated_far_bound`, so pre-#354 health JSON without
  that field still deserializes and reserializes with an empty bound map.
  Durable aiwonder evidence:
  `/home/croyse/calyx/data/fsv-issue358-guard-health-serde-20260609-b298497`.
- **Post-sweep hardening.** #355 separates active drift state from successful
  Anneal notification state, so a full hook channel increments `dropped_events`
  but keeps retrying until the slot is actually notified. Durable aiwonder
  evidence:
  `/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`.
  Close #356 to tighten Sextant multi-slot guard behavior before PH38 exit.
- **Deliverables.** `calibrate.rs` (conformal: bound FAR at confidence 1−α; per-
  slot; provenance: corpus_hash, estimator, FAR/FRR, ts, plus
  `CalibrationMeta.per_slot`), `novelty.rs`
  (NewRegion|Quarantine|RejectClosed), drift monitor hook (Anneal).
- **Key tasks.** ROC/conformal per slot; identity slots strict, stylistic loose;
  uncalibrated τ → `provisional`, high-stakes refuses; `CALYX_GUARD_PROVISIONAL`.
- **FSV gate.** **injection corpus blocked >=99% at the calibrated FAR** is
  signed off in #268 on the real prompt-injection set on aiwonder; valid novelty
  writes a durable file-backed novelty row and reads back as `AwaitingGrounding`.
  **Sextant InRegionOnly** is signed off in #276 with a before/after hit-set
  readback proving OOD exclusion and surviving-hit guard verdicts.
  **Per-slot calibration health** is signed off in #354 with profile, health,
  and hook-event JSON proving slot 1 FAR `0.01`, slot 2 FAR `0.05`, slot 1 FRR
  `1.0`, slot 2 FRR `0.0`, and hook comparison against slot 1's own bound.
  **GuardHealth serde compatibility** is signed off in #358 with legacy JSON
  readback proving the new per-slot bound map defaults to empty when absent.
  **Drift hook retry** is signed off in #355 with before/after event readback
  proving slot 3 is absent before channel recovery and present after retry while
  drift remains true.
- **Axioms/PRD.** A12, A2, `09 §3`, `19 §4`.

## PH39 — Identity-locked generation (speaker/style)
- **Objective.** Pin a generator (voice/style/persona) to a grounded
  constellation; every output must stay inside the `Gτ` ball on identity slots.
- **Deps.** PH38, PH19 (speaker/style lenses).
- **Deliverables.** `SpeakerMatch`/`StyleHold` anchor handling; identity-slot
  required-set; integration with `guard_generate`.
- **Key tasks.** commission a WavLM speaker lens + a style lens (HF); require
  cos ≥ calibrated τ on identity slots; injection that breaks character →
  quarantine.
- **FSV gate.** a target-speaker constellation guards TTS output (in-region
  similarity measured, e.g. against VoxCeleb); an injection that would break
  persona lands outside τ on the style slots → quarantined (read verdicts).
- **Axioms/PRD.** `09 §5b`, A12, `05 §7`.

---

## Stage 8 exit
Ward is the boundary — every AI output must sit inside a grounded region on every
load-bearing axis, making injection defense, drift detection, and continual
learning one calibrated cosine gate, plus injection-proof identity-locked
generation — PRD `GUARD`. Also powers TCT dedup (Stage 9) and Anneal's mistake-
closure.
