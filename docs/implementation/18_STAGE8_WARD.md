# Stage 8 — Ward Gτ Guard (PH37–PH39)

**Status:** active. Tracked by Stage 8 epic #257 and exit issue #280; PH37-PH39
atomic tasks are #258-#279. PH37 core T01-T06 (#258-#263) and PH38 T01 (#264)
are FSV-signed-off. PH37 blindspot hardening #275/#277/#278 remains open and
must be handled before the Ward exit can claim PH37 complete.

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
- **Post-sweep note.** PH37 blindspot tasks remain open after the core signoff:
  #275 (`guard_query` incoming-query OOD), #277 required-slot derivation from
  Assay load-bearing bits, and #278 Lodestar kernel-near guard priority.
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
- **Deliverables.** `calibrate.rs` (conformal: bound FAR at confidence 1−α; per-
  slot; provenance: corpus_hash, estimator, FAR/FRR, ts), `novelty.rs`
  (NewRegion|Quarantine|RejectClosed), drift monitor hook (Anneal).
- **Key tasks.** ROC/conformal per slot; identity slots strict, stylistic loose;
  uncalibrated τ → `provisional`, high-stakes refuses; `CALYX_GUARD_PROVISIONAL`.
- **FSV gate.** **injection corpus blocked ≥99% at the calibrated FAR** (real
  prompt-injection set on aiwonder); a valid-novelty input → new region (read
  the novel constellation + the calibration provenance).
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
